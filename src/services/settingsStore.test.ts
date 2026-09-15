import { describe, expect, it } from 'vitest';
import { loadAllSettings, loadCachedRegistry, MemorySettingsStorage, REGISTRY_CACHE_TTL_MS, saveAllSettings, saveCachedRegistry, settingsFor } from './settingsStore';

describe('loadAllSettings', () => {
  it('storage vuoto -> record vuoto', () => {
    expect(loadAllSettings(new MemorySettingsStorage())).toEqual({});
  });
  it('JSON corrotto -> record vuoto (non lancia)', () => {
    const s = new MemorySettingsStorage();
    s.setItem('dsh-manager.settings.v1', 'non-json{{{');
    expect(loadAllSettings(s)).toEqual({});
  });
  it('legge il payload salvato', () => {
    const s = new MemorySettingsStorage();
    s.setItem('dsh-manager.settings.v1', JSON.stringify({ windows: { port: 1, extraArgs: [], workspace: null, desiredVersion: null } }));
    expect(loadAllSettings(s).windows.port).toBe(1);
  });
});

describe('saveAllSettings', () => {
  it('persiste per id e rilegge', () => {
    const s = new MemorySettingsStorage();
    saveAllSettings(s, [{ id: 'windows', settings: { port: 3080, extraArgs: ['--x'], workspace: null, desiredVersion: '1.0.0' } }]);
    expect(loadAllSettings(s).windows).toMatchObject({ port: 3080, desiredVersion: '1.0.0' });
  });
});

describe('settingsFor', () => {
  it('restituisce i default quando manca la chiave', () => {
    const s = settingsFor(new MemorySettingsStorage(), 'windows', { port: 3080 });
    expect(s).toEqual({ port: 3080, extraArgs: [], workspace: null, desiredVersion: null });
  });
  it('usa la porta di fallback', () => {
    const s = settingsFor(new MemorySettingsStorage(), 'wsl:X', { port: 3102 });
    expect(s.port).toBe(3102);
  });
  it('le impostazioni salvate vincono sui fallback', () => {
    const storage = new MemorySettingsStorage();
    saveAllSettings(storage, [{ id: 'windows', settings: { port: 9999, extraArgs: [], workspace: null, desiredVersion: null } }]);
    expect(settingsFor(storage, 'windows', { port: 3080 }).port).toBe(9999);
  });
});

describe('registry cache (prima pittura senza rete)', () => {
  const data = { latest: '2.0.0', distTags: { latest: '2.0.0' }, versions: ['2.0.0', '1.0.0'] };
  it('storage vuoto -> null', () => {
    expect(loadCachedRegistry(new MemorySettingsStorage())).toBeNull();
  });
  it('salva e rilegge quando fresco', () => {
    const s = new MemorySettingsStorage();
    saveCachedRegistry(s, data, 1000);
    expect(loadCachedRegistry(s, 2000)?.latest).toBe('2.0.0');
  });
  it('scaduto -> null', () => {
    const s = new MemorySettingsStorage();
    saveCachedRegistry(s, data, 1000);
    expect(loadCachedRegistry(s, 1000 + REGISTRY_CACHE_TTL_MS + 1)).toBeNull();
  });
  it('JSON corrotto -> null senza lanciare', () => {
    const s = new MemorySettingsStorage();
    s.setItem('dsh-manager.registry-cache.v1', 'non-json{{{');
    expect(loadCachedRegistry(s)).toBeNull();
  });
});