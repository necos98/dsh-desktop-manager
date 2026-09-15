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
    nodeRuntime: e.kind === "wsl" ? (e.settings.nodeRuntime ?? null) : null,
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

/** Direzione del cambio versione verso la desiderata: upgrade, downgrade,
 *  reinstall (stessa versione) oppure install (dsh assente). Null quando non
 *  c'e una versione desiderata selezionabile (registry non raggiungibile).
 *  Pura: la UI abilita il pulsante per QUALSIASI direzione — il backend
 *  (bun/npm add|install -g con versione pinnata) sovrascrive in ogni caso. */
export type VersionChange = "install" | "upgrade" | "downgrade" | "reinstall";

export function versionChangeOf(e: EnvRow, registry: RegistryData | null): VersionChange | null {
  const target = desiredVersionOf(e, registry);
  if (!target) return null;
  if (!e.probe?.installed || !e.probe.version) return "install";
  const cmp = compareVersions(e.probe.version, target);
  if (cmp < 0) return "upgrade";
  if (cmp > 0) return "downgrade";
  return "reinstall";
}

/** Verbo italiano per la direzione del cambio (pulsanti + messaggi). */
export function versionVerb(change: VersionChange | null): string {
  switch (change) {
    case "downgrade":
      return "Downgrade";
    case "reinstall":
      return "Reinstallazione";
    case "upgrade":
      return "Aggiornamento";
    default:
      return "Installazione";
  }
}

export function guiUrl(port: number): string {
  return "http://127.0.0.1:" + port;
}

/** Stato toolchain di un ambiente: "ok" | "partial" (una sola) | "missing" (nessuna) | "unknown".
 *  Pura: la regola vive qui, la UI mostra solo l'avviso. Nelle distro WSL
 *  i booleani contano SOLO i binari nativi Linux (l'interop /mnt/* viene
 *  ignorata dal backend): il manager non installa mai toolchain — se
 *  mancano entrambe, deve farlo l'utente. */
export type ToolchainStatus = "ok" | "partial" | "missing" | "unknown";

export function toolchainStatus(probe: EnvProbe | null): ToolchainStatus {
  const bun = probe?.hasBun ?? null;
  const npm = probe?.hasNpm ?? null;
  if (bun === null && npm === null) return "unknown";
  if (bun === true || npm === true) {
    return bun === true && npm === true ? "ok" : "partial";
  }
  return "missing";
}

/** Avviso toolchain da mostrare vicino ai pulsanti Installa/Aggiorna.
 *  Null = nessun avviso (toolchain ok o non verificata). */
export function toolchainWarning(e: EnvRow): string | null {
  const native = e.kind === "wsl" ? " nativi" : "";
  const distro = e.kind === "wsl" ? ` nella distro "${e.distro ?? e.name}"` : " su Windows";
  switch (toolchainStatus(e.probe)) {
    case "missing":
      return (
        `Attenzione: ne bun ne npm${native} risultano installati${distro} (eventuali copie Windows via interop vengono ignorate: i mondi non condividono installazioni). ` +
        `Installa prima una toolchain nativa (es. bun da https://bun.sh oppure nodejs/npm della distro), poi installa dsh. ` +
        `Il manager non installa toolchain da solo.`
      );
    case "partial":
      return null;
    default:
      return null;
  }
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
  // Sonda non ancora arrivata (prima pittura progressiva): riga in attesa,
  // mai "Non installato" (sarebbe un falso negativo prima delle sonde).
  if (!e.probe) return { text: "Rilevamento…", cls: "busy" };
  if (!e.probe?.installed) return { text: "Non installato", cls: "warn" };
  if (!e.probe.version) {
    // dsh rilevato ma `dsh --version` non restituisce semver (tipico wrapper
    // interop Windows rotto nella distro): mai mostrare "vnull".
    return { text: "Installazione da verificare", cls: "warn" };
  }
  if (isUpdateAvailable(e, registry))
    return { text: "Aggiornamento -> v" + desiredVersionOf(e, registry), cls: "warn" };
  const change = versionChangeOf(e, registry);
  if (change === "downgrade")
    return { text: "Downgrade -> v" + desiredVersionOf(e, registry), cls: "warn" };
  if (change === "reinstall")
    return { text: "v" + e.probe.version + " (reinstallabile)", cls: "idle" };
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

/** Applica una probe arrivata in background a una riga (arricchimento
 *  progressivo): aggiorna probe/wslState, conserva settings/running/busy.
 *  Pura (nuovo array, stesse righe non toccate per riferimento). */
export function markRowProbed(
  envs: EnvRow[],
  id: string,
  probe: EnvProbe | null,
  wslState?: string,
): EnvRow[] {
  return envs.map((e) =>
    e.id === id
      ? { ...e, probe, wslState: wslState ?? e.wslState }
      : e,
  );
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