import { describe, expect, it } from 'vitest';
import {
  AUTH_GRACE_MS,
  TABBAR_H,
  badgeStatus,
  buildLayoutInput,
  desiredVersionOf,
  envById,
  guiUrl,
  isUpdateAvailable,
  markRowProbed,
  pruneEnvs,
  runningEnvIds,
  showTabEnv,
  tabUrlFor,
  targetOf,
  toolchainStatus,
  toolchainWarning,
  upsertEnv,
  versionChangeOf,
  versionVerb,
  wslDefaultPort,
  type EnvRow,
} from './environments';
import type { RegistryData } from '../types';

function row(over: Partial<EnvRow> = {}): EnvRow {
  return {
    id: 'windows',
    kind: 'windows',
    name: 'Windows',
    settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: null },
    probe: null,
    running: false,
    busy: false,
    authUrl: null,
    authSentAt: null,
    ...over,
  };
}

const registry = (latest: string | null): RegistryData | null =>
  latest === null ? null : { latest, distTags: { latest }, versions: [latest] };

describe('targetOf', () => {
  it('proietta la riga sul target backend (null quando assenti)', () => {
    const t = targetOf(row({ name: 'U', distro: 'U', settings: { port: 3100, extraArgs: ['a'], workspace: 'w', desiredVersion: null } }));
    expect(t).toMatchObject({ kind: 'windows', name: 'U', distro: 'U', port: 3100, extraArgs: ['a'], workspace: 'w' });
    expect(targetOf(row()).distro).toBeNull();
    expect(targetOf(row()).workspace).toBeNull();
  });
});

describe('desiredVersionOf', () => {
  it('la versione pinnata vince sul registry', () => {
    const e = row({ settings: { port: 1, extraArgs: [], workspace: null, desiredVersion: '1.2.3' } });
    expect(desiredVersionOf(e, registry('9.9.9'))).toBe('1.2.3');
  });
  it('latest delega al registry', () => {
    expect(desiredVersionOf(row(), registry('2.0.0'))).toBe('2.0.0');
    expect(desiredVersionOf(row(), null)).toBeNull();
  });
});

describe('versionChangeOf / versionVerb', () => {
  const reg2 = registry('2.0.0');
  it('install quando dsh assente o senza versione', () => {
    expect(versionChangeOf(row({ probe: null }), reg2)).toBe('install');
    expect(versionChangeOf(row({ probe: { kind: 'w', name: 'w', installed: false } }), reg2)).toBe('install');
    expect(versionChangeOf(row({ probe: { kind: 'w', name: 'w', installed: true, version: null } }), reg2)).toBe('install');
  });
  it('upgrade / downgrade / reinstall a confronto', () => {
    const mk = (v: string) => row({ probe: { kind: 'w', name: 'w', installed: true, version: v } });
    expect(versionChangeOf(mk('1.0.0'), reg2)).toBe('upgrade');
    expect(versionChangeOf(mk('3.0.0'), reg2)).toBe('downgrade');
    expect(versionChangeOf(mk('2.0.0'), reg2)).toBe('reinstall');
  });
  it('null senza target (registry assente)', () => {
    expect(versionChangeOf(row({ probe: { kind: 'w', name: 'w', installed: true, version: '1.0.0' } }), null)).toBeNull();
    expect(versionVerb(null)).toBe('Installazione');
  });
  it('verbi italiani per pulsanti e messaggi', () => {
    expect(versionVerb('install')).toBe('Installazione');
    expect(versionVerb('upgrade')).toBe('Aggiornamento');
    expect(versionVerb('downgrade')).toBe('Downgrade');
    expect(versionVerb('reinstall')).toBe('Reinstallazione');
  });
  it('badge downgrade visibile in lista', () => {
    const e = row({ probe: { kind: 'w', name: 'w', installed: true, version: '3.0.0' } });
    const b = badgeStatus(e, reg2);
    expect(b.text).toContain('Downgrade');
    expect(b.cls).toBe('warn');
  });
});

describe('isUpdateAvailable', () => {
  it('true solo se installata < desiderata', () => {
    const reg = registry('2.0.0');
    expect(isUpdateAvailable(row({ probe: { kind: 'windows', name: 'W', installed: true, version: '1.0.0' } }), reg)).toBe(true);
    expect(isUpdateAvailable(row({ probe: { kind: 'windows', name: 'W', installed: true, version: '2.0.0' } }), reg)).toBe(false);
    expect(isUpdateAvailable(row({ probe: { kind: 'windows', name: 'W', installed: true, version: '3.0.0' } }), reg)).toBe(false);
  });
  it('false senza probe, senza versione o senza registry', () => {
    expect(isUpdateAvailable(row({ probe: null }), registry('2.0.0'))).toBe(false);
    expect(isUpdateAvailable(row({ probe: { kind: 'w', name: 'w', installed: false } }), registry('2.0.0'))).toBe(false);
    expect(isUpdateAvailable(row({ probe: { kind: 'w', name: 'w', installed: true, version: '1.0.0' } }), null)).toBe(false);
  });
});

describe('toolchainStatus / toolchainWarning', () => {
  it('missing quando npm manca (wsl: nativi + interop ignorata)', () => {
    const e = row({ kind: 'wsl', distro: 'U', id: 'wsl:U', probe: { kind: 'wsl', name: 'U', installed: false, hasNpm: false } });
    expect(toolchainStatus(e.probe)).toBe('missing');
    expect(toolchainWarning(e)).toContain('npm');
    expect(toolchainWarning(e)).toContain('nativi');
    expect(toolchainWarning(e)).toContain('interop');
    expect(toolchainWarning(e)).toContain('Il manager non installa toolchain');
  });
  it('missing su Windows quando npm manca', () => {
    const e = row({ probe: { kind: 'windows', name: 'W', installed: false, hasNpm: false } });
    expect(toolchainStatus(e.probe)).toBe('missing');
    expect(toolchainWarning(e)).toContain('npm');
    expect(toolchainWarning(e)).not.toContain('nativi');
  });
  it('unknown quando non verificata', () => {
    expect(toolchainStatus(null)).toBe('unknown');
    expect(toolchainWarning(row({ probe: null }))).toBeNull();
  });
  it('ok con npm presente', () => {
    const e = row({ probe: { kind: 'windows', name: 'W', installed: true, hasNpm: true } });
    expect(toolchainStatus(e.probe)).toBe('ok');
    expect(toolchainWarning(e)).toBeNull();
  });
});

describe('badgeStatus con installazione rotta', () => {
  it('mai vnull: installato senza versione -> Installazione da verificare', () => {
    const e = row({ probe: { kind: 'wsl', name: 'U', installed: true, version: null, error: 'exec: node: not found' } });
    const b = badgeStatus(e, null);
    expect(b.text).toBe('Installazione da verificare');
    expect(b.text).not.toContain('null');
    expect(b.cls).toBe('warn');
  });
});

describe('guiUrl', () => {
  it('costruisce il loopback', () => {
    expect(guiUrl(3080)).toBe('http://127.0.0.1:3080');
  });
});

describe('envById / runningEnvIds', () => {
  it('trova per id e elenca i running', () => {
    const envs = [row({ id: 'a' }), row({ id: 'b', running: true })];
    expect(envById(envs, 'b')?.id).toBe('b');
    expect(envById(envs, 'x')).toBeUndefined();
    expect(runningEnvIds(envs)).toEqual(['b']);
  });
});

describe('showTabEnv', () => {
  it('windows visibile se installato', () => {
    expect(showTabEnv(row({ probe: { kind: 'windows', name: 'W', installed: true } }))).toBe(true);
    expect(showTabEnv(row({ probe: null }))).toBe(false);
  });
  it('wsl solo se Running o gia in esecuzione', () => {
    const base = row({ kind: 'wsl', probe: { kind: 'wsl', name: 'U', installed: true } });
    expect(showTabEnv({ ...base, wslState: 'Running' })).toBe(true);
    expect(showTabEnv({ ...base, wslState: 'Stopped' })).toBe(false);
    expect(showTabEnv({ ...base, wslState: 'Stopped', running: true })).toBe(true);
  });
});

describe('badgeStatus', () => {
  it('priorita: running > busy > non installato > update > versione', () => {
    const reg = registry('2.0.0');
    expect(badgeStatus(row({ running: true, settings: { port: 1, extraArgs: [], workspace: null, desiredVersion: null } }), reg).cls).toBe('ok');
    expect(badgeStatus(row({ busy: true }), reg).cls).toBe('busy');
    expect(badgeStatus(row({ probe: null }), reg)).toEqual({ text: 'Rilevamento…', cls: 'busy' });
    expect(badgeStatus(row({ probe: { kind: 'w', name: 'w', installed: false } }), reg)).toEqual({ text: 'Non installato', cls: 'warn' });
    expect(badgeStatus(row({ probe: { kind: 'w', name: 'w', installed: true, version: '1.0.0' } }), reg)).toMatchObject({ cls: 'warn' });
    expect(badgeStatus(row({ probe: { kind: 'w', name: 'w', installed: true, version: '2.0.0' } }), reg)).toMatchObject({ text: 'v2.0.0 (reinstallabile)', cls: 'idle' });
  });
});

describe('upsertEnv / pruneEnvs', () => {
  it('aggiorna senza perdere settings/running', () => {
    const s = { port: 9999, extraArgs: [] as string[], workspace: null, desiredVersion: null };
    const envs = [row({ id: 'wsl:U', kind: 'wsl', running: true, settings: s })];
    const next = upsertEnv(envs, { id: 'wsl:U', kind: 'wsl', name: 'U-new', probe: null, settings: s });
    expect(next[0].name).toBe('U-new');
    expect(next[0].settings.port).toBe(9999);
    expect(next[0].running).toBe(true);
    expect(next).not.toBe(envs);
  });
  it('inserisce nuove righe spente', () => {
    const next = upsertEnv([], { id: 'windows', kind: 'windows', name: 'Windows', probe: null, settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: null } });
    expect(next).toHaveLength(1);
    expect(next[0].running).toBe(false);
  });
  it('prune rimuove gli id scomparsi', () => {
    const envs = [row({ id: 'windows' }), row({ id: 'wsl:Old', kind: 'wsl' })];
    expect(pruneEnvs(envs, new Set(['windows']))).toHaveLength(1);
  });
  it('markRowProbed aggiorna probe/wslState senza toccare settings/running', () => {
    const s = { port: 9999, extraArgs: [] as string[], workspace: null, desiredVersion: null };
    const envs = [row({ id: 'wsl:U', kind: 'wsl', running: true, probe: null, settings: s })];
    const probe = { kind: 'wsl', name: 'U', installed: true, version: '1.0.0' };
    const next = markRowProbed(envs, 'wsl:U', probe, 'Running');
    expect(next[0].probe?.version).toBe('1.0.0');
    expect(next[0].wslState).toBe('Running');
    expect(next[0].settings.port).toBe(9999);
    expect(next[0].running).toBe(true);
    expect(next).not.toBe(envs);
  });
  it('markRowProbed su id assente non tocca nulla', () => {
    const envs = [row({ id: 'windows' })];
    expect(markRowProbed(envs, 'wsl:Nope', null)).toHaveLength(1);
  });
});

describe('wslDefaultPort / TABBAR_H', () => {
  it('base 3100 + indice', () => {
    expect(wslDefaultPort(0)).toBe(3100);
    expect(wslDefaultPort(2)).toBe(3102);
  });
  it('tabbar 44px', () => {
    expect(TABBAR_H).toBe(44);
  });
});

describe('tabUrlFor / buildLayoutInput', () => {
  it('usa l authUrl dentro la finestra di grazia', () => {
    const e = row({ settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: null }, authUrl: 'http://x/?token=1', authSentAt: null });
    expect(tabUrlFor(e, 1000)).toBe('http://x/?token=1');
    expect(tabUrlFor(e, AUTH_GRACE_MS + 1)).toBe('http://x/?token=1');
  });
  it('dopo la grazia torna all url nudo', () => {
    const e = row({ settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: null }, authUrl: 'http://x/?token=1', authSentAt: 0 });
    expect(tabUrlFor(e, AUTH_GRACE_MS + 1)).toBe('http://127.0.0.1:3080');
  });
  it('buildLayoutInput include solo i running ed estende la grazia', () => {
    const running = row({ id: 'a', running: true, settings: { port: 3080, extraArgs: [], workspace: null, desiredVersion: null }, authUrl: 'http://x/?token=1', authSentAt: null });
    const stopped = row({ id: 'b', settings: { port: 3100, extraArgs: [], workspace: null, desiredVersion: null } });
    const input = buildLayoutInput([running, stopped], 'a', { tabbar: 44, width: 1200, height: 800 }, 5000);
    expect(input.activeEnv).toBe('a');
    expect(input.tabs).toEqual([{ envId: 'a', url: 'http://x/?token=1' }]);
    expect(running.authSentAt).toBe(5000);
  });
});