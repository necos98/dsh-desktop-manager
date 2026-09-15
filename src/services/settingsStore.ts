// Servizio: persistenza impostazioni per-ambiente (DIP).
// La logica dipende dalla porta SettingsStoragePort, non da localStorage:
// in produzione si inietta BrowserSettingsStorage, nei test MemorySettingsStorage (LSP).
import type { EnvSettings, RegistryData } from '../types';

export const SETTINGS_KEY = 'dsh-manager.settings.v1';
/** Ultimo registry npm noto (prima pittura senza rete: la prima pittura
 *  dell'avvio non aspetta mai il fetch). Scade dopo REGISTRY_CACHE_TTL_MS. */
export const REGISTRY_CACHE_KEY = 'dsh-manager.registry-cache.v1';
export const REGISTRY_CACHE_TTL_MS = 6 * 60 * 60 * 1000; // 6 ore

/** Porta minima di storage chiave/valore (ISP: solo cio che serve allo store). */
export interface SettingsStoragePort {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export class BrowserSettingsStorage implements SettingsStoragePort {
  getItem(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  }
  setItem(key: string, value: string): void {
    try {
      localStorage.setItem(key, value);
    } catch {
      /* storage non disponibile: le impostazioni restano in memoria */
    }
  }
}

export class MemorySettingsStorage implements SettingsStoragePort {
  private data = new Map<string, string>();
  getItem(key: string): string | null {
    return this.data.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }
}

export function loadAllSettings(storage: SettingsStoragePort): Record<string, EnvSettings> {
  try {
    const raw = storage.getItem(SETTINGS_KEY);
    if (raw) return JSON.parse(raw) as Record<string, EnvSettings>;
  } catch {
    /* JSON corrotto: si riparte dai default */
  }
  return {};
}

export function saveAllSettings(
  storage: SettingsStoragePort,
  envs: Array<{ id: string; settings: EnvSettings }>,
): void {
  const all: Record<string, EnvSettings> = {};
  for (const e of envs) all[e.id] = e.settings;
  storage.setItem(SETTINGS_KEY, JSON.stringify(all));
}

export function settingsFor(
  storage: SettingsStoragePort,
  id: string,
  fallback: Partial<EnvSettings>,
): EnvSettings {
  const all = loadAllSettings(storage);
  return (
    all[id] ?? {
      port: fallback.port ?? 3080,
      extraArgs: [],
      workspace: null,
      desiredVersion: null,
    }
  );
}

interface RegistryCachePayload {
  savedAt: number;
  data: RegistryData;
}

/** Legge il registry cachato se fresco (null = assente/scaduto/corroto).
 *  `nowMs` iniettabile per i test. Puro sullo storage. */
export function loadCachedRegistry(
  storage: SettingsStoragePort,
  nowMs: number = Date.now(),
): RegistryData | null {
  try {
    const raw = storage.getItem(REGISTRY_CACHE_KEY);
    if (!raw) return null;
    const payload = JSON.parse(raw) as RegistryCachePayload;
    if (!payload || typeof payload.savedAt !== 'number' || !payload.data) return null;
    if (nowMs - payload.savedAt > REGISTRY_CACHE_TTL_MS) return null;
    if (!Array.isArray(payload.data.versions)) return null;
    return payload.data;
  } catch {
    return null;
  }
}

/** Salva il registry per la prossima prima pittura (best-effort). */
export function saveCachedRegistry(
  storage: SettingsStoragePort,
  data: RegistryData,
  nowMs: number = Date.now(),
): void {
  try {
    const payload: RegistryCachePayload = { savedAt: nowMs, data };
    storage.setItem(REGISTRY_CACHE_KEY, JSON.stringify(payload));
  } catch {
    /* storage non disponibile: la cache resta in memoria */
  }
}