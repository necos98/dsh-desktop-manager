// Infrastruttura: porta verso il backend Rust (DIP).
// Il dominio/servizi dipendono dall'interfaccia EnvGateway; l'unica
// implementazione di produzione invoca i comandi Tauri. Nei test si inietta un fake.
import { invoke } from '@tauri-apps/api/core';
import type {
  EnvProbe,
  EnvTarget,
  LayoutInput,
  NodeRuntime,
  StartResult,
  StopResult,
  UpdateResult,
  WslDiag,
  WslDistro,
} from '../types';

/** Porta backend (ISP: un metodo per comando, nessuna dipendenza UI). */
export interface EnvGateway {
  /** Verifica dsh sull'ambiente Windows (PATH, bun/npm global). */
  detectWindows(): Promise<EnvProbe>;
  /** Elenca le distro WSL disponibili. */
  listWslDistros(): Promise<WslDistro[]>;
  /** Verifica dsh dentro una specifica distro WSL (runtime scelto o null). */
  probeWsl(distro: string, nodeRuntime?: string | null): Promise<EnvProbe>;
  /** Runtime Node disponibili nella distro (nvm decrescenti + sistema). */
  listNodeRuntimes(distro: string): Promise<NodeRuntime[]>;
  /** True se la porta risulta aperta su 127.0.0.1. */
  isPortOpen(port: number): Promise<boolean>;
  /** Trova la prima porta libera partendo da from. */
  findFreePort(from: number): Promise<number>;
  /** Avvia dsh web sull'ambiente indicato (porta dedicata). */
  startEnv(target: EnvTarget): Promise<StartResult>;
  /** Ferma l'istanza di dsh avviata dal manager sull'ambiente indicato. */
  stopEnv(target: EnvTarget): Promise<StopResult>;
  /** Crea/posiziona le webview figlie con le GUI di DSH sotto la barra tab. */
  layoutTabs(input: LayoutInput): Promise<void>;
  /** Apre l'URL nel browser di sistema. */
  openInBrowser(url: string): Promise<void>;
  /** Aggiorna (o installa) dsh all'ambiente con la versione scelta. */
  runUpdate(target: EnvTarget, version: string): Promise<UpdateResult>;
  /** Coda del log di un ambiente (percorso, coda). */
  readEnvLog(target: EnvTarget, lines?: number): Promise<[string, string]>;
  /** Diagnostica WSL passo-passo (mai un throw: vedi WslDiag.error). */
  diagnoseWsl(distro: string, port: number, nodeRuntime?: string | null): Promise<WslDiag>;
}

export type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

export class TauriEnvGateway implements EnvGateway {
  constructor(private readonly invokeFn: InvokeFn = invoke as unknown as InvokeFn) {}

  detectWindows(): Promise<EnvProbe> {
    return this.invokeFn<EnvProbe>("detect_windows");
  }
  listWslDistros(): Promise<WslDistro[]> {
    return this.invokeFn<WslDistro[]>("list_wsl_distros");
  }
  probeWsl(distro: string, nodeRuntime?: string | null): Promise<EnvProbe> {
    return this.invokeFn<EnvProbe>("probe_wsl", { distro, nodeRuntime: nodeRuntime ?? null });
  }
  listNodeRuntimes(distro: string): Promise<NodeRuntime[]> {
    return this.invokeFn<NodeRuntime[]>("list_node_runtimes", { distro });
  }
  isPortOpen(port: number): Promise<boolean> {
    return this.invokeFn<boolean>("is_port_open", { port });
  }
  findFreePort(from: number): Promise<number> {
    return this.invokeFn<number>("find_free_port", { from });
  }
  startEnv(target: EnvTarget): Promise<StartResult> {
    return this.invokeFn<StartResult>("start_env", { target });
  }
  stopEnv(target: EnvTarget): Promise<StopResult> {
    return this.invokeFn<StopResult>("stop_env", { target });
  }
  layoutTabs(input: LayoutInput): Promise<void> {
    return this.invokeFn<void>("layout_tabs", { input });
  }
  openInBrowser(url: string): Promise<void> {
    return this.invokeFn<void>("open_in_browser", { url });
  }
  runUpdate(target: EnvTarget, version: string): Promise<UpdateResult> {
    return this.invokeFn<UpdateResult>("run_update", { target, version });
  }
  readEnvLog(target: EnvTarget, lines?: number): Promise<[string, string]> {
    return this.invokeFn<[string, string]>("read_env_log", { target, lines });
  }
  diagnoseWsl(distro: string, port: number, nodeRuntime?: string | null): Promise<WslDiag> {
    return this.invokeFn<WslDiag>("diagnose_wsl", { distro, port, nodeRuntime: nodeRuntime ?? null });
  }
}

/** Gateway di produzione (singleton). */
export const defaultGateway = new TauriEnvGateway();