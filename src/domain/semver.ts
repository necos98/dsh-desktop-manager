// Dominio: confronto versioni semver (puro, nessun I/O).
// Estratto da registry.ts (SRP): cambia solo se cambia la semantica di versione.

export interface ParsedVersion {
  major: number;
  minor: number;
  patch: number;
  pre: string[]; // parti della pre-release
  raw: string;
}

export function parseVersion(v: string): ParsedVersion | null {
  const m = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(v.trim());
  if (!m) return null;
  return {
    major: Number(m[1]),
    minor: Number(m[2]),
    patch: Number(m[3]),
    pre: m[4] ? m[4].split(".") : [],
    raw: v.trim(),
  };
}

/** Confronta due versioni semver; ritorna <0 se a<b. La pre-release viene prima della release. */
export function compareVersions(a: string, b: string): number {
  const pa = parseVersion(a);
  const pb = parseVersion(b);
  if (!pa || !pb) return a.localeCompare(b);
  for (const k of ["major", "minor", "patch"] as const) {
    if (pa[k] !== pb[k]) return pa[k] - pb[k];
  }
  if (pa.pre.length === 0 && pb.pre.length === 0) return 0;
  if (pa.pre.length === 0) return 1; // release > pre-release
  if (pb.pre.length === 0) return -1;
  const len = Math.max(pa.pre.length, pb.pre.length);
  for (let i = 0; i < len; i++) {
    const sa = pa.pre[i];
    const sb = pb.pre[i];
    if (sa === undefined) return -1;
    if (sb === undefined) return 1;
    const na = Number(sa);
    const nb = Number(sb);
    const isNa = Number.isNaN(na);
    const isNb = Number.isNaN(nb);
    if (!isNa && !isNb) {
      if (na !== nb) return na - nb;
    } else if (isNa && isNb) {
      if (sa !== sb) return sa < sb ? -1 : 1;
    } else {
      return isNa ? 1 : -1; // numerici prima di alfanumerici
    }
  }
  return 0;
}

export function compareVersionsDesc(a: string, b: string): number {
  return compareVersions(b, a);
}
