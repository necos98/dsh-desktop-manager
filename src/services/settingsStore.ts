// Servizio: persistenza impostazioni per-ambiente (DIP).
// La logica dipende dalla porta SettingsStoragePort, non da localStorage:
// in produzione si inietta BrowserSettingsStorage, nei test MemorySettingsStorage (LSP).
import type { EnvSettings } from '../types';

export const SETTINGS_KEY = 'dsh-manager.settings.v1';

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