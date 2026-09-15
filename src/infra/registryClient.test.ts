import { describe, expect, it, vi } from 'vitest';
import { NpmRegistryClient, parseRegistryPayload, tagOf } from './registryClient';
import type { RegistryData } from '../types';

describe('parseRegistryPayload', () => {
  it('normalizza dist-tags e ordina le versioni desc', () => {
    const data = parseRegistryPayload({ 'dist-tags': { latest: '2.0.0' }, versions: { '1.0.0': {}, '2.0.0': {}, '1.5.0': {} } });
    expect(data.latest).toBe('2.0.0');
    expect(data.versions).toEqual(['2.0.0', '1.5.0', '1.0.0']);
  });
  it('payload vuoto -> latest null e lista vuota', () => {
    expect(parseRegistryPayload({})).toEqual({ latest: null, distTags: {}, versions: [] });
  });
});

describe('tagOf', () => {
  const data: RegistryData = { latest: '2.0.0', distTags: { latest: '2.0.0', beta: '2.1.0-beta.1' }, versions: ['2.1.0-beta.1', '2.0.0', '1.0.0'] };
  it('latest ha precedenza', () => {
    expect(tagOf('2.0.0', data)).toBe('latest');
  });
  it('ritorna il tag corrispondente o null', () => {
    expect(tagOf('2.1.0-beta.1', data)).toBe('beta');
    expect(tagOf('1.0.0', data)).toBeNull();
  });
});

describe('NpmRegistryClient', () => {
  it('fetchRegistry usa il fetch iniettato (DIP)', async () => {
    const fetchFn = vi.fn(async () => ({ ok: true, json: async () => ({ 'dist-tags': { latest: '2.0.0' }, versions: { '2.0.0': {} } }) }));
    const client = new NpmRegistryClient(fetchFn as unknown as typeof fetch);
    const data = await client.fetchRegistry();
    expect(data.latest).toBe('2.0.0');
    expect(fetchFn).toHaveBeenCalledOnce();
  });
  it('lancia su HTTP non-ok', async () => {
    const fetchFn = vi.fn(async () => ({ ok: false, status: 500, json: async () => ({}) }));
    await expect(new NpmRegistryClient(fetchFn as unknown as typeof fetch).fetchRegistry()).rejects.toThrow('Registry npm: HTTP 500');
  });
  it('fetchRegistryWithTimeout non aspetta oltre il timeout', async () => {
    const hanging = vi.fn(() => new Promise<never>(() => undefined));
    const t0 = Date.now();
    await expect(
      new NpmRegistryClient(hanging as unknown as typeof fetch).fetchRegistryWithTimeout(50),
    ).rejects.toThrow('timeout');
    expect(Date.now() - t0).toBeLessThan(5000);
  });
  it('fetchRegistryWithTimeout ritorna i dati quando la rete risponde', async () => {
    const fetchFn = vi.fn(async () => ({ ok: true, json: async () => ({ 'dist-tags': { latest: '3.0.0' }, versions: { '3.0.0': {} } }) }));
    const data = await new NpmRegistryClient(fetchFn as unknown as typeof fetch).fetchRegistryWithTimeout(2000);
    expect(data.latest).toBe('3.0.0');
  });
});