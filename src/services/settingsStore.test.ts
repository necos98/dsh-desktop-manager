import { describe, expect, it } from 'vitest';
import { loadAllSettings, MemorySettingsStorage, saveAllSettings, settingsFor } from './settingsStore';

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