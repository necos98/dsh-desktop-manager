// Tipi condivisi: rispecchiano le struct serializzate dal backend Rust
// (rename_all = "camelCase").

export interface EnvProbe {
  kind: string; // "windows" | "wsl"
  name: string;
  distro?: string | null;
  installed: boolean;
  version?: string | null;
  executable?: string | null;
  dshHome?: string | null;
  error?: string | null;
  /** Toolchain rilevata (None = non verificata). Il manager non installa
   *  mai bun/npm: se mancano entrambe, deve installarle l'utente. */
  hasBun?: boolean | null;
  hasNpm?: boolean | null;
}

/** Un runtime Node rilevato nella distro (comando list_node_runtimes).
 *  `id` e la dir bin (es. /home/u/.nvm/versions/node/v24.20.0/bin). */
export interface NodeRuntime {
  id: string;
  label: string;
  nodeVersion?: string | null;
  isDefault: boolean;
  source: string; // "nvm" | "system"
}

/** Diagnostica WSL passo-passo (comando diagnose_wsl): mai un throw,
 *  l'eventuale fallimento fatale finisce in `error`. */
export interface WslDiag {
  distro: string;
  state?: string | null;
  dshInstalled: boolean;
  dshVersion?: string | null;
  hasBun: boolean;
  hasNpm: boolean;
  portOpenInDistro?: boolean | null;
  portOpenFromWindows: boolean;
  logTail?: string | null;
  error?: string | null;
}

export interface WslDistro {
  name: string;
  state: string; // Running | Stopped | ...
  wslVersion?: string | null;
}

/** Snapshot per-distro per la prima pittura (comando scan_boot): HOME e
 *  toolchain senza sonde pesanti. `dsh_native_path`: percorso classificato
 *  nativo (mai /mnt/*); `dsh_version`: versione nota dall'ultima sonda. */
export interface CachedDistro {
  name: string;
  state: string;
  home: string;
  hasBun: boolean;
  hasNpm: boolean;
  dshNativePath?: string | null;
  dshVersion?: string | null;
}

/** Elenco distro + cache per la prima pittura (comando scan_boot). */
export interface BootScan {
  distros: WslDistro[];
  cached: CachedDistro[];
}

export interface EnvTarget {
  kind: string; // "windows" | "wsl"
  name: string;
  distro?: string | null;
  port: number;
  extraArgs: string[];
  workspace?: string | null;
  /** Runtime Node scelto (solo WSL): dir bin o versione nvm. Null = automatico. */
  nodeRuntime?: string | null;
}

export interface StartResult {
  ok: boolean;
  message: string;
  pid?: number | null;
  port: number;
  reached: boolean;
  /** URL con ?token= stampato da `dsh web` (solo memoria, mai persistito). */
  authUrl?: string | null;
}

export interface StopResult {
  ok: boolean;
  message: string;
}

export interface UpdateResult {
  ok: boolean;
  exitCode: number;
  output: string;
}

export interface RegistryData {
  latest: string | null;
  distTags: Record<string, string>;
  versions: string[];
}

/** Stato persistito di un ambiente (localStorage). */
export interface EnvSettings {
  port: number;
  extraArgs: string[];
  workspace?: string | null;
  desiredVersion?: string | null; // versione "desiderata" (pinnata) o null = latest
  nodeRuntime?: string | null; // runtime Node scelto (solo WSL) o null = automatico
}

export type Settings = Record<string, EnvSettings>;

/** Una tab di DSH da mostrare nel manager (webview incorporata). */
export interface TabSpec {
  envId: string;
  url: string;
}

/** Payload per posizionare/creare le webview figlie (layout tipo browser). */
export interface LayoutInput {
  tabbar: number; // altezza della barra tab in px
  width: number; // larghezza logica finestra
  height: number; // altezza logica finestra
  activeEnv: string | null; // env della tab attiva (null = schermata impostazioni)
  tabs: TabSpec[]; // solo ambienti in esecuzione
}
