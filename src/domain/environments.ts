// Dominio: modello degli ambienti e regole pure (nessun I/O, nessun DOM).
// Tutte le funzioni sono pure tranne buildLayoutInput, che aggiorna authSentAt
// (finestra di grazia ereditata dal comportamento storico: documentata sotto).
import { compareVersions } from './semver';
import type {
  EnvProbe,
  EnvSettings,
  EnvTarget,
  LayoutInput,
  RegistryData,
} from '../types';

export interface EnvRow {
  id: string;
  kind: "windows" | "wsl";
  name: string;
  distro?: string;
  wslState?: string; // Running | Stopped | ... (solo WSL, da wsl -l -v)
  settings: EnvSettings;
  probe: EnvProbe | null;
  running: boolean;
  busy: boolean;
  note?: string;
  /** URL con ?token= dell'istanza corrente (solo memoria, mai persistito). */
  authUrl: string | null;
  /** Quando (ms epoch) l'URL autenticato e' stato inviato alla webview; null = mai. */
  authSentAt: number | null;
}

/** Stato "browser": schermata impostazioni oppure tab di un ambiente. */
export type ViewMode = { view: "settings" } | { view: "env"; envId: string };

export const TABBAR_H = 44;
export const WINDOWS_DEFAULT_PORT = 3080;
export const WSL_PORT_BASE = 3100;
/** Finestra di grazia dopo l'invio dell'URL autenticato (evita di navigare
 *  all'URL nudo prima che il 303 abbia coniato il cookie). */
export const AUTH_GRACE_MS = 10_000;

export function targetOf(e: EnvRow): EnvTarget {
  return {
    kind: e.kind,
    name: e.name,
    distro: e.distro ?? null,
    port: e.settings.port,
    extraArgs: e.settings.extraArgs,
    workspace: e.settings.workspace ?? null,
  };
}

export function desiredVersionOf(e: EnvRow, registry: RegistryData | null): string | null {
  const d = e.settings.desiredVersion;
  if (d && d !== "latest") return d;
  return registry?.latest ?? null;
}

export function isUpdateAvailable(e: EnvRow, registry: RegistryData | null): boolean {
  if (!e.probe?.installed || !e.probe.version) return false;
  const target = desiredVersionOf(e, registry);
  if (!target) return false;
  return compareVersions(e.probe.version, target) < 0;
}

export function guiUrl(port: number): string {
  return "http://127.0.0.1:" + port;
}

export function envById(envs: EnvRow[], id: string): EnvRow | undefined {
  return envs.find((e) => e.id === id);
}

export function runningEnvIds(envs: EnvRow[]): string[] {
  return envs.filter((e) => e.running).map((e) => e.id);
}

/**
 * Un ambiente compare nella barra tab solo se e utilizzabile:
 * dsh installato e, per le distro WSL, distro attiva (Running)
 * oppure istanza gia in esecuzione.
 */
export function showTabEnv(e: EnvRow): boolean {
  if (!e.probe?.installed) return false;
  if (e.kind === "windows") return true;
  return e.running || e.wslState === "Running";
}

export function badgeStatus(
  e: EnvRow,
  registry: RegistryData | null,
): { text: string; cls: string } {
  if (e.running) return { text: "In esecuzione :" + e.settings.port, cls: "ok" };
  if (e.busy) return { text: "operazione in corso", cls: "busy" };
  if (!e.probe?.installed) return { text: "Non installato", cls: "warn" };
  if (isUpdateAvailable(e, registry))
    return { text: "Aggiornamento -> v" + desiredVersionOf(e, registry), cls: "warn" };
  return { text: "v" + e.probe.version, cls: "idle" };
}

export interface UpsertInput {
  id: string;
  kind: "windows" | "wsl";
  name: string;
  distro?: string;
  wslState?: string;
  probe: EnvProbe | null;
  /** Impostazioni risolte dallo store (solo per nuove righe; quelle esistenti le conservano). */
  settings: EnvSettings;
}

/** Inserisce o aggiorna una riga ambiente (le righe esistenti conservano settings/running/busy). */
export function upsertEnv(envs: EnvRow[], input: UpsertInput): EnvRow[] {
  const existing = envs.some((e) => e.id === input.id);
  if (existing) {
    return envs.map((e) =>
      e.id === input.id
        ? { ...e, probe: input.probe, name: input.name, wslState: input.wslState }
        : e,
    );
  }
  return [
    ...envs,
    {
      id: input.id,
      kind: input.kind,
      name: input.name,
      distro: input.distro,
      wslState: input.wslState,
      settings: input.settings,
      probe: input.probe,
      running: false,
      busy: false,
      authUrl: null,
      authSentAt: null,
    },
  ];
}

/** Rimuove le righe i cui id non sono piu rilevati (es. distro WSL eliminate). */
export function pruneEnvs(envs: EnvRow[], seen: Set<string>): EnvRow[] {
  return envs.filter((e) => seen.has(e.id));
}

/** Porta di default per una distro WSL in base all'indice in `wsl -l -v`. */
export function wslDefaultPort(index: number): number {
  return WSL_PORT_BASE + index;
}

/**
 * URL da usare per la tab di un ambiente: quello autenticato (?token=) solo
 * dentro la finestra di grazia, altrimenti quello nudo (il cookie e gia coniato).
 */
export function tabUrlFor(e: EnvRow, nowMs: number): string {
  if (e.authUrl && (e.authSentAt === null || nowMs - e.authSentAt < AUTH_GRACE_MS)) {
    return e.authUrl;
  }
  return guiUrl(e.settings.port);
}

export interface LayoutDims {
  tabbar: number;
  width: number;
  height: number;
}

/**
 * Costruisce il payload per layout_tabs (una voce per ambiente in esecuzione).
 * Effetto collaterale voluto: quando usa l'URL autenticato aggiorna authSentAt,
 * estendendo la finestra di grazia (stesso comportamento storico di main.ts).
 */
export function buildLayoutInput(
  envs: EnvRow[],
  activeEnvId: string | null,
  dims: LayoutDims,
  nowMs: number,
): LayoutInput {
  return {
    tabbar: dims.tabbar,
    width: dims.width,
    height: dims.height,
    activeEnv: activeEnvId,
    tabs: envs
      .filter((e) => e.running)
      .map((e) => {
        let url = guiUrl(e.settings.port);
        if (e.authUrl && (e.authSentAt === null || nowMs - e.authSentAt < AUTH_GRACE_MS)) {
          url = e.authUrl;
          e.authSentAt = nowMs;
        }
        return { envId: e.id, url };
      }),
  };
}