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
}

export interface WslDistro {
  name: string;
  state: string; // Running | Stopped | ...
  wslVersion?: string | null;
}

export interface EnvTarget {
  kind: string; // "windows" | "wsl"
  name: string;
  distro?: string | null;
  port: number;
  extraArgs: string[];
  workspace?: string | null;
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
