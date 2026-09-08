// Facciata compatibile: re-esporta la porta EnvGateway come funzioni,
// cosi il codice esistente (main.ts) non cambia contratto d'uso.
import { defaultGateway } from './infra/envGateway';
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
} from './types';

/** Verifica dsh sull'ambiente Windows (PATH, bun/npm global). */
export function detectWindows(): Promise<EnvProbe> {
  return defaultGateway.detectWindows();
}

/** Elenca le distro WSL disponibili. */
export function listWslDistros(): Promise<WslDistro[]> {
  return defaultGateway.listWslDistros();
}

/** Verifica dsh dentro una specifica distro WSL (runtime scelto o null). */
export function probeWsl(distro: string, nodeRuntime?: string | null): Promise<EnvProbe> {
  return defaultGateway.probeWsl(distro, nodeRuntime);
}

/** Runtime Node disponibili nella distro (nvm decrescenti + sistema). */
export function listNodeRuntimes(distro: string): Promise<NodeRuntime[]> {
  return defaultGateway.listNodeRuntimes(distro);
}

/** True se la porta risulta aperta su 127.0.0.1. */
export function isPortOpen(port: number): Promise<boolean> {
  return defaultGateway.isPortOpen(port);
}

/** Trova la prima porta libera partendo da from. */
export function findFreePort(from: number): Promise<number> {
  return defaultGateway.findFreePort(from);
}

/** Avvia dsh web sull'ambiente indicato (porta dedicata). */
export function startEnv(target: EnvTarget): Promise<StartResult> {
  return defaultGateway.startEnv(target);
}

/** Ferma l'istanza di dsh avviata dal manager sull'ambiente indicato. */
export function stopEnv(target: EnvTarget): Promise<StopResult> {
  return defaultGateway.stopEnv(target);
}

/**
 * Crea/posiziona le webview figlie che mostrano le GUI di DSH
 * (una per ambiente in esecuzione), sotto la barra delle tab.
 */
export function layoutTabs(input: LayoutInput): Promise<void> {
  return defaultGateway.layoutTabs(input);
}

/** Apre l'URL nel browser di sistema. */
export function openInBrowser(url: string): Promise<void> {
  return defaultGateway.openInBrowser(url);
}

/** Aggiorna (o installa) dsh all'ambiente con la versione scelta. */
export function runUpdate(target: EnvTarget, version: string): Promise<UpdateResult> {
  return defaultGateway.runUpdate(target, version);
}

/** Coda del log di un ambiente (percorso, coda). */
export function readEnvLog(target: EnvTarget, lines?: number): Promise<[string, string]> {
  return defaultGateway.readEnvLog(target, lines);
}

/** Diagnostica WSL passo-passo (mai un throw: vedi WslDiag.error). */
export function diagnoseWsl(distro: string, port: number, nodeRuntime?: string | null): Promise<WslDiag> {
  return defaultGateway.diagnoseWsl(distro, port, nodeRuntime);
}

export { defaultGateway };