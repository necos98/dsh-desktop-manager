// Servizio applicativo: orchestrazione ambienti (OCP/DIP).
// Dipende solo dalle porte EnvGateway + SettingsStoragePort + NpmRegistryClient:
// nuovi ambienti o nuovi backend si aggiungono senza modificare questo servizio.
// Non tocca il DOM: restituisce outcome che la UI applica alle righe.
import {
  WINDOWS_DEFAULT_PORT,
  desiredVersionOf,
  pruneEnvs,
  targetOf,
  upsertEnv,
  wslDefaultPort,
  type EnvRow,
} from '../domain/environments';
import type { EnvGateway } from '../infra/envGateway';
import { NpmRegistryClient } from '../infra/registryClient';
import {
  BrowserSettingsStorage,
  saveAllSettings,
  settingsFor,
  type SettingsStoragePort,
} from './settingsStore';
import type { EnvProbe, EnvSettings, RegistryData } from '../types';

export interface ScanResult {
  envs: EnvRow[];
  /** Messaggio di errore non fatale (es. elenco WSL fallito) oppure "". */
  status: string;
}

export interface StartOutcome {
  ok: boolean;
  message: string;
  port: number;
  reached: boolean;
  authUrl: string | null;
}

export interface StopOutcome {
  message: string;
}

export type UpdateOutcome =
  | { ok: true; message: string; note: string; probe: EnvProbe | null }
  | { ok: false; error: string };

export interface RegistryOutcome {
  registry: RegistryData | null;
  error: string | null;
}

export class EnvironmentService {
  constructor(
    private readonly gateway: EnvGateway,
    private readonly storage: SettingsStoragePort = new BrowserSettingsStorage(),
    private readonly registryClient: NpmRegistryClient = new NpmRegistryClient(),
  ) {}

  settingsFor(id: string, fallback: Partial<EnvSettings>): EnvSettings {
    return settingsFor(this.storage, id, fallback);
  }

  persist(envs: EnvRow[]): void {
    saveAllSettings(this.storage, envs);
  }

  /** Rileva Windows + distro WSL, aggiorna/prosciuga le righe (pura orchestrazione I/O). */
  async scanEnvironments(current: EnvRow[]): Promise<ScanResult> {
    let envs = [...current];
    let status = "";

    let windowsProbe: EnvProbe | null = null;
    try {
      windowsProbe = await this.gateway.detectWindows();
    } catch {
      windowsProbe = null;
    }
    envs = upsertEnv(envs, {
      id: "windows",
      kind: "windows",
      name: "Windows",
      probe: windowsProbe,
      settings: this.settingsFor("windows", { port: WINDOWS_DEFAULT_PORT }),
    });

    let distros: { name: string; state: string }[] = [];
    try {
      distros = await this.gateway.listWslDistros();
    } catch (e) {
      status = "Errore elenco WSL: " + String(e);
    }
    const probes = await Promise.all(
      distros.map(async (d) => {
        try {
          return { distro: d.name, probe: await this.gateway.probeWsl(d.name) };
        } catch (e) {
          return {
            distro: d.name,
            probe: {
              kind: "wsl",
              name: d.name,
              distro: d.name,
              installed: false,
              error: String(e),
            } as EnvProbe,
          };
        }
      }),
    );
    const seen = new Set(["windows", ...probes.map((p) => "wsl:" + p.distro)]);
    envs = pruneEnvs(envs, seen);
    for (const p of probes) {
      const index = distros.findIndex((d) => d.name === p.distro);
      envs = upsertEnv(envs, {
        id: "wsl:" + p.distro,
        kind: "wsl",
        name: p.distro,
        distro: p.distro,
        wslState: distros[index]?.state,
        probe: p.probe,
        settings: this.settingsFor("wsl:" + p.distro, { port: wslDefaultPort(index) }),
      });
    }
    if (envs.length === 0) {
      envs = upsertEnv(envs, {
        id: "windows",
        kind: "windows",
        name: "Windows",
        probe: windowsProbe,
        settings: this.settingsFor("windows", { port: WINDOWS_DEFAULT_PORT }),
      });
    }
    return { envs, status };
  }

  /**
   * Aggiorna i flag running via is_port_open (salta le righe busy).
   * Ritorna true se almeno un flag e cambiato (la UI decide badge/layout).
   */
  async refreshRunningStates(envs: EnvRow[]): Promise<boolean> {
    let changed = false;
    await Promise.all(
      envs.map(async (e) => {
        if (e.busy) return;
        let running = false;
        try {
          running = await this.gateway.isPortOpen(e.settings.port);
        } catch {
          running = false;
        }
        if (e.running !== running) {
          e.running = running;
          changed = true;
        }
      }),
    );
    return changed;
  }

  /** Avvia dsh sull'ambiente (trova prima una porta libera a partire da quella configurata). */
  async startEnvironment(e: EnvRow): Promise<StartOutcome> {
    const free = await this.gateway.findFreePort(e.settings.port);
    const target = targetOf(e);
    target.port = free;
    const res = await this.gateway.startEnv(target);
    if (res.ok && res.port) {
      let message = res.message;
      if (!res.reached) {
        message = res.message + " — la GUI non risponde ancora: riprova tra poco.";
      } else if (!res.authUrl) {
        message = res.message + " — URL di autenticazione non trovato nel log.";
      }
      return {
        ok: true,
        message,
        port: res.port,
        reached: res.reached,
        authUrl: res.authUrl ?? null,
      };
    }
    return { ok: false, message: res.message, port: e.settings.port, reached: false, authUrl: null };
  }

  /** Ferma l'istanza avviata dal manager (il backend non tocca mai processi esterni). */
  async stopEnvironment(e: EnvRow): Promise<StopOutcome> {
    const res = await this.gateway.stopEnv(targetOf(e));
    return { message: res.message };
  }

  /** Installa/aggiorna dsh alla versione desiderata e rilegge la probe. */
  async updateEnvironment(e: EnvRow, registry: RegistryData | null): Promise<UpdateOutcome> {
    const target = desiredVersionOf(e, registry);
    if (!target) {
      return { ok: false, error: "Nessuna versione selezionabile (registry non raggiungibile?)." };
    }
    const verb = e.probe?.installed ? "Aggiornamento" : "Installazione";
    const res = await this.gateway.runUpdate(targetOf(e), target);
    const note = res.output.trim().slice(-3000);
    let probe: EnvProbe | null = null;
    try {
      probe = e.kind === "windows" ? await this.gateway.detectWindows() : await this.gateway.probeWsl(e.distro ?? "");
    } catch {
      probe = null;
    }
    if (res.ok) return { ok: true, message: verb + " completato (v" + target + ").", note, probe };
    return { ok: false, error: verb + " fallito (exit " + res.exitCode + ")." };
  }

  async loadRegistry(): Promise<RegistryOutcome> {
    try {
      return { registry: await this.registryClient.fetchRegistry(), error: null };
    } catch (e) {
      return { registry: null, error: String(e) };
    }
  }
}