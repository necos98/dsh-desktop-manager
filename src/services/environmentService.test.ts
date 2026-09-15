import { describe, expect, it } from 'vitest';
import type { EnvRow } from '../domain/environments';
import type { EnvGateway } from '../infra/envGateway';
import { NpmRegistryClient } from '../infra/registryClient';
import { MemorySettingsStorage } from './settingsStore';
import { EnvironmentService } from './environmentService';
import { staleProbeFor } from './environmentService';
import type { EnvProbe, EnvTarget, RegistryData, StartResult, StopResult, UpdateResult, WslDistro } from '../types';

const probeWin = (version: string | null = '1.0.0'): EnvProbe => ({ kind: 'windows', name: 'Windows', installed: version !== null, version });
const probeWsl = (distro: string): EnvProbe => ({ kind: 'wsl', name: distro, distro, installed: true, version: '1.0.0' });

function row(over: Partial<EnvRow> = {}): EnvRow {
  return {
    id: 'windows',
    kind: 'windows',
    name: 'Windows',
    settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: null },
    probe: probeWin(),
    running: false,
    busy: false,
    authUrl: null,
    authSentAt: null,
    ...over,
  };
}

interface FakeOpts {
  distros?: WslDistro[];
  failList?: boolean;
  start?: Partial<StartResult>;
  portsOpen?: number[];
  runningProbeVersion?: string;
}

function fakeGateway(service: { lastStart?: EnvTarget }, opts: FakeOpts = {}): EnvGateway {
  return {
    detectWindows: async () => probeWin(opts.runningProbeVersion ?? '1.0.0'),
    listWslDistros: async () => {
      if (opts.failList) throw new Error('wsl assente');
      return opts.distros ?? [];
    },
    scanBoot: async () => {
      if (opts.failList) throw new Error('wsl assente');
      const distros = opts.distros ?? [];
      return {
        distros,
        cached: distros.map((d) => ({
          name: d.name,
          state: d.state,
          home: '/home/u',
          hasNpm: true,
          dshNativePath: '/home/u/.local/bin/dsh',
          dshVersion: '1.0.0',
        })),
      };
    },
    probeWslFast: async (d) => probeWsl(d),
    probeWsl: async (d) => probeWsl(d),
    isPortOpen: async (p) => (opts.portsOpen ?? []).includes(p),
    findFreePort: async (from) => from,
    startEnv: async (target) => {
      service.lastStart = target;
      return { ok: true, message: 'ok', port: target.port, reached: true, authUrl: 'http://127.0.0.1:' + target.port + '/?token=t', ...opts.start };
    },
    stopEnv: async () => ({ ok: true, message: 'fermato' }) as StopResult,
    layoutTabs: async () => undefined,
    openInBrowser: async () => undefined,
    runUpdate: async () => ({ ok: true, exitCode: 0, output: 'ok', logPath: null }) as UpdateResult,
    readEnvLog: async (target) => [`/tmp/dsh-desktop-manager-${target.port}.log`, 'riga1\nriga2'],
    diagnoseWsl: async (distro) => ({ distro, dshInstalled: true, dshVersion: '1.0.0', hasNpm: true, portOpenFromWindows: false, portOpenInDistro: true, logTail: 'tail', state: 'Running', error: null }),
    listNodeRuntimes: async () => [{ id: '/home/u/.nvm/versions/node/v24.20.0/bin', label: 'nvm v24.20.0 (default)', nodeVersion: 'v24.20.0', isDefault: true, source: 'nvm' }],
  };
}

const svc = (opts: FakeOpts = {}) => {
  const seen: { lastStart?: EnvTarget } = {};
  const service = new EnvironmentService(fakeGateway(seen, opts), new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error("no-net"); }));
  return { service, seen };
};

describe('EnvironmentService.scanEnvironments', () => {
  it('rileva windows + distro e assegna porte 3080 / 3100+n', async () => {
    const { service } = svc({ distros: [{ name: 'Ubuntu', state: 'Running' }, { name: 'Debian', state: 'Stopped' }] });
    const { envs, status } = await service.scanEnvironments([]);
    expect(status).toBe('');
    expect(envs.map((e) => e.id)).toEqual(['windows', 'wsl:Ubuntu', 'wsl:Debian']);
    expect(envs.find((e) => e.id === 'windows')?.settings.port).toBe(3080);
    expect(envs.find((e) => e.id === 'wsl:Debian')?.settings.port).toBe(3101);
    expect(envs.find((e) => e.id === 'wsl:Ubuntu')?.wslState).toBe('Running');
  });

  it('seconda scansione conserva settings/running e prosciuga distro rimosse', async () => {
    const { service } = svc({ distros: [{ name: 'Ubuntu', state: 'Running' }] });
    const first = await service.scanEnvironments([]);
    const win = first.envs.find((e) => e.id === 'windows')!;
    win.settings.port = 4000;
    win.running = true;
    const second = await service.scanEnvironments([...first.envs, row({ id: 'wsl:Vecchia', kind: 'wsl' })]);
    expect(second.envs.some((e) => e.id === 'wsl:Vecchia')).toBe(false);
    expect(second.envs.find((e) => e.id === 'windows')?.settings.port).toBe(4000);
    expect(second.envs.find((e) => e.id === 'windows')?.running).toBe(true);
  });

  it('errore WSL -> status non fatale e solo windows', async () => {
    const { service } = svc({ failList: true });
    const { envs, status } = await service.scanEnvironments([]);
    expect(envs).toHaveLength(1);
    expect(status).toContain('Errore elenco WSL');
  });
});

describe('EnvironmentService.scanBootFast (prima pittura)', () => {
  it('dipinge subito windows + distro senza sonde per-distro', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    let probes = 0;
    const gw = fakeGateway(seen, { distros: [{ name: 'Ubuntu', state: 'Running' }] });
    gw.probeWsl = async (d) => { probes++; return probeWsl(d); };
    gw.probeWslFast = async (d) => { probes++; return probeWsl(d); };
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error("no-net"); }));
    const { envs, status } = await service.scanBootFast([]);
    expect(status).toBe('');
    expect(envs.map((e) => e.id)).toEqual(['windows', 'wsl:Ubuntu']);
    expect(probes).toBe(0);
    // La riga usa la cache (versione nota, toolchain dalla cache).
    const wsl = envs.find((e) => e.id === 'wsl:Ubuntu')!;
    expect(wsl.probe?.version).toBe('1.0.0');
    expect(wsl.probe?.hasNpm).toBe(true);
  });

  it('distro senza cache -> probe null (in attesa, mai "Non installato" falso)', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen, { distros: [{ name: 'Nuova', state: 'Stopped' }] });
    gw.scanBoot = async () => ({ distros: [{ name: 'Nuova', state: 'Stopped' }], cached: [] });
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error("no-net"); }));
    const { envs } = await service.scanBootFast([]);
    expect(envs.find((e) => e.id === 'wsl:Nuova')?.probe).toBeNull();
  });

  it('errore boot -> status non fatale e solo windows', async () => {
    const { service } = svc({ failList: true });
    const { envs, status } = await service.scanBootFast([]);
    expect(envs).toHaveLength(1);
    expect(status).toContain('Errore elenco WSL');
  });
});

describe('EnvironmentService.enrichRow (arricchimento background)', () => {
  it('wsl senza runtime scelto -> sonda veloce con stato', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen);
    let fastArgs: [string, string | null | undefined][] = [];
    gw.probeWslFast = async (d, s) => { fastArgs.push([d, s]); return probeWsl(d); };
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error("no-net"); }));
    const e = row({ kind: 'wsl', distro: 'Ubuntu', id: 'wsl:Ubuntu', wslState: 'Running', probe: null });
    const probe = await service.enrichRow(e);
    expect(probe.installed).toBe(true);
    expect(fastArgs).toEqual([['Ubuntu', 'Running']]);
  });

  it('wsl con runtime scelto -> sonda completa (PATH esatto)', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen);
    let full = 0;
    let fast = 0;
    gw.probeWsl = async (d) => { full++; return probeWsl(d); };
    gw.probeWslFast = async (d) => { fast++; return probeWsl(d); };
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error("no-net"); }));
    const e = row({
      kind: 'wsl', distro: 'Ubuntu', id: 'wsl:Ubuntu', probe: null,
      settings: { port: 3100, extraArgs: [], workspace: null, desiredVersion: null, nodeRuntime: '/home/u/.nvm/x/bin' },
    });
    await service.enrichRow(e);
    expect(full).toBe(1);
    expect(fast).toBe(0);
  });

  it('errore sonda -> probe scollegata senza throw', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen);
    gw.probeWslFast = async () => { throw new Error('distro spenta'); };
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error("no-net"); }));
    const probe = await service.enrichRow(row({ kind: 'wsl', distro: 'U', id: 'wsl:U', probe: null }));
    expect(probe.installed).toBe(false);
    expect(probe.error).toContain('distro spenta');
  });
});

describe('staleProbeFor', () => {
  const cached = { name: 'U', state: 'Running', home: '/home/u', hasNpm: true, dshNativePath: '/home/u/.local/bin/dsh', dshVersion: '1.2.3' };
  it('senza cache -> null (riga in attesa)', () => {
    expect(staleProbeFor('U', undefined, [])).toBeNull();
  });
  it('con cache e senza runtime scelto -> probe provvisoria', () => {
    const p = staleProbeFor('U', cached, [])!;
    expect(p.installed).toBe(true);
    expect(p.version).toBe('1.2.3');
    expect(p.hasNpm).toBe(true);
  });
  it('con runtime scelto -> null (va riverificata)', () => {
    const current = [row({ id: 'wsl:U', kind: 'wsl', settings: { port: 3100, extraArgs: [], workspace: null, desiredVersion: null, nodeRuntime: '/x/bin' } })];
    expect(staleProbeFor('U', cached, current)).toBeNull();
  });
  it('cache senza versione -> non installata ma toolchain nota', () => {
    const p = staleProbeFor('U', { ...cached, dshVersion: null, dshNativePath: null }, [])!;
    expect(p.installed).toBe(false);
    expect(p.hasNpm).toBe(true);
  });
});
describe('EnvironmentService.refreshRunningStates', () => {
  it('aggiorna i flag e salta le righe busy', async () => {
    const { service } = svc({ portsOpen: [3080] });
    const a = row({ id: 'a', settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: null } });
    const b = row({ id: 'b', busy: true, settings: { port: 9999, extraArgs: [], workspace: null, desiredVersion: null } });
    expect(await service.refreshRunningStates([a, b])).toBe(true);
    expect(a.running).toBe(true);
    expect(b.running).toBe(false);
    expect(await service.refreshRunningStates([a, b])).toBe(false);
  });
});

describe('EnvironmentService.startEnvironment', () => {
  it('usa la porta libera e arricchisce i messaggi degradati', async () => {
    const { service, seen } = svc();
    const ok = await service.startEnvironment(row());
    expect(ok.ok).toBe(true);
    expect(seen.lastStart?.port).toBe(3080);
    expect(ok.authUrl).toContain('token=');
  });
  it('non-reached -> messaggio GUI non risponde', async () => {
    const { service } = svc({ start: { reached: false, authUrl: null, message: 'lanciato' } });
    const out = await service.startEnvironment(row());
    expect(out.message).toContain('non risponde');
  });
  it('reached senza authUrl -> avviso 401/log', async () => {
    const { service } = svc({ start: { reached: true, authUrl: null, message: 'ok' } });
    expect((await service.startEnvironment(row())).message).toContain('autenticazione');
  });
  it('start fallito -> ok false e porta originale', async () => {
    const { service } = svc({ start: { ok: false, message: 'boom', port: 0, reached: false, authUrl: null } });
    const out = await service.startEnvironment(row());
    expect(out.ok).toBe(false);
    expect(out.message).toBe('boom');
  });
});

describe('EnvironmentService.stopEnvironment / loadRegistry', () => {
  it('stop delega al gateway', async () => {
    const { service } = svc();
    expect((await service.stopEnvironment(row())).message).toBe('fermato');
  });
  it('loadRegistry cattura l errore senza lanciare', async () => {
    const { service } = svc();
    const out = await service.loadRegistry();
    expect(out.registry).toBeNull();
    expect(out.error).toContain('no-net');
  });
  it('loadRegistry salva in cache e la riusa quando la rete manca', async () => {
    const storage = new MemorySettingsStorage();
    const fetchOk = async () => ({
      ok: true,
      json: async () => ({ 'dist-tags': { latest: '9.9.9' }, versions: { '9.9.9': {} } }),
    });
    const online = new EnvironmentService(
      fakeGateway({}, {}),
      storage,
      new NpmRegistryClient(fetchOk as unknown as typeof fetch),
    );
    const first = await online.loadRegistry();
    expect(first.registry?.latest).toBe('9.9.9');
    expect(first.stale).toBeUndefined();
    // Rete caduta: stesso storage -> registry dalla cache, nessuno errore.
    const offline = new EnvironmentService(
      fakeGateway({}, {}),
      storage,
      new NpmRegistryClient(async () => { throw new Error('no-net'); }),
    );
    const second = await offline.loadRegistry();
    expect(second.registry?.latest).toBe('9.9.9');
    expect(second.error).toBeNull();
    expect(second.stale).toBe(true);
  });
  it('loadRegistry con rete lenta non aspetta oltre il timeout', async () => {
    const hanging = new Promise<never>(() => undefined);
    const service = new EnvironmentService(
      fakeGateway({}, {}),
      new MemorySettingsStorage(),
      new NpmRegistryClient((() => hanging) as unknown as typeof fetch),
    );
    const t0 = Date.now();
    const out = await service.loadRegistry();
    const elapsed = Date.now() - t0;
    expect(out.registry).toBeNull();
    expect(elapsed).toBeLessThan(15000);
  });
});

describe('EnvironmentService.listRuntimes', () => {
  it('elenca i runtime della distro', async () => {
    const { service } = svc();
    const out = await service.listRuntimes(row({ kind: 'wsl', distro: 'Ubuntu', id: 'wsl:Ubuntu' }));
    expect(out.ok).toBe(true);
    if (out.ok) expect(out.runtimes[0].id).toContain('v24.20.0');
  });
  it('su windows -> errore dedicato', async () => {
    const { service } = svc();
    expect((await service.listRuntimes(row())).ok).toBe(false);
  });
});

describe('EnvironmentService.readEnvironmentLog / diagnoseEnvironment', () => {
  it('log ok -> percorso e coda', async () => {
    const { service } = svc();
    const out = await service.readEnvironmentLog(row());
    expect(out.ok).toBe(true);
    if (out.ok) expect(out.log.tail).toContain('riga1');
  });
  it('log fallito -> errore senza throw', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen);
    gw.readEnvLog = async () => { throw new Error('nessun log'); };
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error('no-net'); }));
    const out = await service.readEnvironmentLog(row());
    expect(out.ok).toBe(false);
  });
  it('diagnostica wsl -> diag passo-passo', async () => {
    const { service } = svc();
    const out = await service.diagnoseEnvironment(row({ kind: 'wsl', distro: 'Ubuntu', id: 'wsl:Ubuntu' }));
    expect(out.ok).toBe(true);
    if (out.ok) expect(out.diag.portOpenInDistro).toBe(true);
  });
  it('diagnostica su windows -> errore dedicato', async () => {
    const { service } = svc();
    expect((await service.diagnoseEnvironment(row())).ok).toBe(false);
  });
});

describe('EnvironmentService.updateEnvironment', () => {
  const reg: RegistryData = { latest: '2.0.0', distTags: { latest: '2.0.0' }, versions: ['2.0.0', '1.0.0'] };
  it('senza target -> errore registry', async () => {
    const { service } = svc();
    const out = await service.updateEnvironment(row(), null);
    expect(out.ok).toBe(false);
  });
  it('successo -> messaggio, nota troncata e re-probe', async () => {
    const { service } = svc({ runningProbeVersion: '2.0.0' });
    const out = await service.updateEnvironment(row(), reg);
    expect(out.ok).toBe(true);
    if (out.ok) {
      expect(out.message).toContain('2.0.0');
      expect(out.probe?.version).toBe('2.0.0');
    }
  });
  it('downgrade -> verbo Downgrade nel messaggio', async () => {
    const { service } = svc({ runningProbeVersion: '1.0.0' });
    const e = row({ probe: probeWin('2.0.0'), settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: '1.0.0' } });
    const out = await service.updateEnvironment(e, reg);
    expect(out.ok).toBe(true);
    if (out.ok) expect(out.message).toContain('Downgrade');
  });
  it('reinstall stessa versione -> verbo Reinstallazione', async () => {
    const { service } = svc({ runningProbeVersion: '2.0.0' });
    const e = row({ probe: probeWin('2.0.0'), settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: '2.0.0' } });
    const out = await service.updateEnvironment(e, reg);
    expect(out.ok).toBe(true);
    if (out.ok) expect(out.message).toContain('Reinstallazione');
  });
  it('ambiente in esecuzione -> stop + riavvio dopo il cambio', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen);
    let stopped = false;
    gw.stopEnv = async () => { stopped = true; return { ok: true, message: 'fermato' }; };
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error('no-net'); }));
    const e = row({ running: true, probe: probeWin('1.0.0'), settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: '2.0.0' } });
    const out = await service.updateEnvironment(e, reg);
    expect(out.ok).toBe(true);
    expect(stopped).toBe(true);
    expect(e.running).toBe(true);
    if (out.ok) expect(out.message).toContain('riavviata');
  });
  it('update fallito -> output completo in note e percorso del file di log', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen);
    const logPath = 'C:\\Users\\u\\AppData\\Roaming\\dsh-desktop-manager\\logs\\update-windows-windows-20260102-030405.log';
    gw.runUpdate = async () => ({
      ok: false,
      exitCode: -1,
      output: 'errore esecuzione npm: Impossibile trovare il file specificato. (os error 2)',
      logPath,
    });
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error('no-net'); }));
    const out = await service.updateEnvironment(row(), reg);
    expect(out.ok).toBe(false);
    if (out.ok) return;
    // L'output del gateway arriva alla UI integro (non solo "exit -1").
    expect(out.note).toBe('errore esecuzione npm: Impossibile trovare il file specificato. (os error 2)');
    expect(out.logPath).toBe(logPath);
    expect(out.error).toContain('fallito (exit -1)');
    expect(out.error).toContain(logPath);
  });
  it('update fallito senza file di log -> note presente, logPath null', async () => {
    const seen: { lastStart?: EnvTarget } = {};
    const gw = fakeGateway(seen);
    gw.runUpdate = async () => ({ ok: false, exitCode: 3, output: 'npm nativo mancante', logPath: null });
    const service = new EnvironmentService(gw, new MemorySettingsStorage(), new NpmRegistryClient(async () => { throw new Error('no-net'); }));
    const out = await service.updateEnvironment(row(), reg);
    if (out.ok) throw new Error('atteso fallimento');
    expect(out.note).toBe('npm nativo mancante');
    expect(out.logPath).toBeNull();
    expect(out.error).toContain('Output completo qui sotto');
  });
});