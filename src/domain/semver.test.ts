import { describe, expect, it } from 'vitest';
import { compareVersions, compareVersionsDesc, parseVersion } from './semver';

describe('parseVersion', () => {
  it('parsa major/minor/patch senza pre-release', () => {
    expect(parseVersion('1.2.3')).toEqual({ major: 1, minor: 2, patch: 3, pre: [], raw: '1.2.3' });
  });

  it('parsa la pre-release in parti', () => {
    expect(parseVersion('1.0.0-alpha.1')?.pre).toEqual(['alpha', '1']);
  });

  it('rifiuta stringhe non semver', () => {
    expect(parseVersion('foo')).toBeNull();
    expect(parseVersion('1.2')).toBeNull();
    expect(parseVersion('')).toBeNull();
  });

  it('tollera spazi e una v iniziale? no: solo formato pulito', () => {
    expect(parseVersion(' 2.0.0 ')).not.toBeNull();
  });
});

describe('compareVersions', () => {
  it('ordina major/minor/patch', () => {
    expect(compareVersions('1.0.0', '2.0.0')).toBeLessThan(0);
    expect(compareVersions('0.1.0', '0.2.0')).toBeLessThan(0);
    expect(compareVersions('0.0.1', '0.0.2')).toBeLessThan(0);
    expect(compareVersions('1.2.3', '1.2.3')).toBe(0);
    expect(compareVersions('2.0.0', '1.9.9')).toBeGreaterThan(0);
  });

  it('la release viene dopo la pre-release', () => {
    expect(compareVersions('1.0.0-alpha', '1.0.0')).toBeLessThan(0);
    expect(compareVersions('1.0.0', '1.0.0-alpha')).toBeGreaterThan(0);
  });

  it('ordina le pre-release numeriche e alfabetiche', () => {
    expect(compareVersions('1.0.0-alpha.1', '1.0.0-alpha.2')).toBeLessThan(0);
    expect(compareVersions('1.0.0-beta', '1.0.0-alpha')).toBeGreaterThan(0);
    expect(compareVersions('1.0.0-alpha.1', '1.0.0-alpha.1')).toBe(0);
  });

  it('i numerici precedono gli alfanumerici nella pre-release', () => {
    expect(compareVersions('1.0.0-1', '1.0.0-alpha')).toBeLessThan(0);
  });

  it('una pre-release piu lunga vince a parita di prefisso', () => {
    expect(compareVersions('1.0.0-alpha', '1.0.0-alpha.1')).toBeLessThan(0);
    expect(compareVersions('1.0.0-alpha.1', '1.0.0-alpha')).toBeGreaterThan(0);
  });

  it('gestisce versioni non semver con fallback lessicografico', () => {
    expect(compareVersions('foo', 'foo')).toBe(0);
    expect(typeof compareVersions('foo', 'bar')).toBe('number');
  });
});

describe('compareVersionsDesc', () => {
  it('ordina in modo decrescente', () => {
    const versions = ['1.0.0', '2.0.0', '1.5.0', '0.9.9'];
    expect([...versions].sort(compareVersionsDesc)).toEqual(['2.0.0', '1.5.0', '1.0.0', '0.9.9']);
  });

  it('mette le pre-release prima della release solo a pari core? no: release prima', () => {
    expect(compareVersionsDesc('1.0.0', '1.0.0-alpha')).toBeLessThan(0);
  });
});