// Infrastruttura: client del registry npm (l'unico punto che tocca la rete
// per le versioni; il registry permette CORS). Il fetch e iniettabile per i test.
import { compareVersionsDesc } from '../domain/semver';
import type { RegistryData } from '../types';

export const REGISTRY_URL = 'https://registry.npmjs.org/@deepseek-ai/dsh';

export interface RegistryPayload {
  "dist-tags"?: Record<string, string>;
  versions?: Record<string, unknown>;
}

/** Funzione pura: normalizza il payload npm (testabile senza rete). */
export function parseRegistryPayload(json: RegistryPayload): RegistryData {
  const versions = Object.keys(json.versions ?? {}).sort(compareVersionsDesc);
  return {
    latest: json["dist-tags"]?.latest ?? null,
    distTags: json["dist-tags"] ?? {},
    versions,
  };
}

/** Etichette dist-tag utili da mostrare accanto a una versione (puro). */
export function tagOf(version: string, data: RegistryData): string | null {
  const tags = Object.entries(data.distTags)
    .filter(([, v]) => v === version)
    .map(([t]) => t);
  if (tags.length === 0) return null;
  return tags.includes("latest") ? "latest" : tags[0];
}

export class NpmRegistryClient {
  constructor(private readonly fetchFn?: typeof fetch) {}

  async fetchRegistry(): Promise<RegistryData> {
    const f = this.fetchFn ?? globalThis.fetch;
    const res = await f(REGISTRY_URL, { headers: { accept: "application/json" } });
    if (!res.ok) throw new Error("Registry npm: HTTP " + res.status);
    const json = (await res.json()) as RegistryPayload;
    return parseRegistryPayload(json);
  }

  /** Come `fetchRegistry` ma con timeout: l'avvio non aspetta mai la rete
   *  oltre `timeoutMs` (AbortController; allo scadere lancia un Error con
   *  causa "timeout"). Il fetch iniettato dei test che ignora il segnale
   *  resta valido: la corsa usa Promise.race, non dipende dal segnale. */
  async fetchRegistryWithTimeout(timeoutMs: number): Promise<RegistryData> {
    const f = this.fetchFn ?? globalThis.fetch;
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(new Error("timeout")), timeoutMs);
    try {
      const res = await Promise.race([
        f(REGISTRY_URL, { headers: { accept: "application/json" }, signal: ctrl.signal }),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error(`Registry npm: timeout dopo ${timeoutMs}ms`)), timeoutMs),
        ),
      ]);
      if (!res.ok) throw new Error("Registry npm: HTTP " + res.status);
      const json = (await res.json()) as RegistryPayload;
      return parseRegistryPayload(json);
    } finally {
      clearTimeout(timer);
    }
  }
}

const shared = new NpmRegistryClient();

/** Compat: stesso contratto storico di registry.ts. */
export function fetchRegistry(): Promise<RegistryData> {
  return shared.fetchRegistry();
}