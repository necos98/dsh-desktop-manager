// Servizio auto-update dell'app manager (Tauri v2, tauri-plugin-updater).
//
// Incapsula check/download/install dell'aggiornamento del manager stesso
// (da NON confondere con run_update del backend, che aggiorna dsh negli
// ambienti). Import dinamico del plugin: sotto 'vite dev' nel browser il
// modulo nativo non esiste e l'import deve fallire senza rompere la UI.
//
// Il flusso con progress reale e in due fasi:
//   1. checkForManagerUpdate() -> UpdateInfo | null
//   2. downloadAndInstallUpdate(info, onProgress) -> installa e riavvia
// Per semplicita' d'uso resta anche installManagerUpdate(info), che scarica,
// installa e riavvia con un'unica chiamata (senza progress).

export interface UpdateInfo {
  /** Versione disponibile (es. "0.3.0"). */
  version: string;
  /** Versione corrente dell'app. */
  currentVersion: string;
  /** Note di rilascio (possono essere vuote). */
  body: string | null;
  /** Data di pubblicazione (ISO), se presente. */
  date: string | null;
}

/** Avanzamento download in byte (contentLength puo' mancare). */
export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

/** Stato macchina per la UI: idle -> checking -> available|up-to-date|error ... */
export type UpdaterPhase =
  | "idle"
  | "checking"
  | "available"
  | "up-to-date"
  | "downloading"
  | "installing"
  | "error";

type CheckFn = () => Promise<{
  version: string;
  currentVersion: string;
  body?: string;
  date?: string;
  downloadAndInstall: (
    onEvent?: (e: { event: string; data: { contentLength?: number; chunkLength?: number } }) => void,
  ) => Promise<void>;
  close: () => Promise<void>;
} | null>;

let checkOverride: CheckFn | null = null;

/** Inietta un check finto (test / storybook). */
export function __setUpdateCheckForTests(fn: CheckFn | null): void {
  checkOverride = fn;
}

async function loadCheck(): Promise<CheckFn> {
  if (checkOverride) return checkOverride;
  const mod = await import("@tauri-apps/plugin-updater");
  return mod.check as CheckFn;
}

function toInfo(u: {
  version: string;
  currentVersion: string;
  body?: string;
  date?: string;
}): UpdateInfo {
  return {
    version: u.version,
    currentVersion: u.currentVersion,
    body: u.body ?? null,
    date: u.date ?? null,
  };
}

/**
 * Controlla se esiste un aggiornamento del manager.
 * Ritorna null se l'app e' aggiornata; lancia in caso di errore di rete/config.
 * Fuori da Tauri (dev browser) ritorna null senza errori.
 */
export async function checkForManagerUpdate(): Promise<UpdateInfo | null> {
  let check: CheckFn;
  try {
    check = await loadCheck();
  } catch {
    // Non siamo dentro Tauri (es. vite dev nel browser): nessun update possibile.
    return null;
  }
  const update = await check();
  if (!update) return null;
  const info = toInfo(update);
  await update.close().catch(() => undefined);
  return info;
}

/**
 * Scarica e installa l'aggiornamento noto (da un check precedente),
 * riportando il progresso. Al termine installa e riavvia l'app.
 */
export async function downloadAndInstallUpdate(
  onProgress?: (p: UpdateProgress) => void,
): Promise<void> {
  const check = await loadCheck();
  const update = await check();
  if (!update) return;
  let downloaded = 0;
  await update.downloadAndInstall((e) => {
    if (!onProgress) return;
    if (e.event === "Started") {
      downloaded = 0;
      onProgress({ downloaded: 0, total: e.data.contentLength ?? null });
    } else if (e.event === "Progress") {
      downloaded += e.data.chunkLength ?? 0;
      onProgress({ downloaded, total: null });
    } else if (e.event === "Finished") {
      onProgress({ downloaded, total: downloaded });
    }
  });
}

/** Variante semplice: scarica, installa e riavvia (senza progress). */
export async function installManagerUpdate(): Promise<void> {
  await downloadAndInstallUpdate();
}
