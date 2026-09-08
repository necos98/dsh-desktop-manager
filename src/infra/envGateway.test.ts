import { describe, expect, it, vi } from 'vitest';
import { TauriEnvGateway, type InvokeFn } from './envGateway';

function gateway(calls: Array<{ cmd: string; args?: Record<string, unknown> }>, impl?: (cmd: string) => unknown): TauriEnvGateway {
  const invokeFn: InvokeFn = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ cmd, args });
    return (impl ? impl(cmd) : undefined) as T;
  };
  return new TauriEnvGateway(invokeFn);
}

describe('TauriEnvGateway (porta verso il backend)', () => {
  it('ogni metodo invoca il comando con gli args giusti (contratto invoke)', async () => {
    const calls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
    const gw = gateway(calls, (cmd) => (cmd === "find_free_port" ? 4000 : cmd === "is_port_open" ? true : {}));
    await gw.detectWindows();
    await gw.listWslDistros();
    await gw.probeWsl('Ubuntu');
    await gw.isPortOpen(3080);
    await gw.findFreePort(3080);
    await gw.startEnv({ kind: 'windows', name: 'W', distro: null, port: 3080, extraArgs: [], workspace: null });
    await gw.stopEnv({ kind: 'windows', name: 'W', distro: null, port: 3080, extraArgs: [], workspace: null });
    await gw.layoutTabs({ tabbar: 44, width: 1, height: 1, activeEnv: null, tabs: [] });
    await gw.openInBrowser('http://x');
    await gw.runUpdate({ kind: 'windows', name: 'W', distro: null, port: 3080, extraArgs: [], workspace: null }, '1.0.0');
    await gw.readEnvLog({ kind: 'wsl', name: 'U', distro: 'U', port: 3100, extraArgs: [], workspace: null }, 50);
    await gw.diagnoseWsl('U', 3100);
    await gw.probeWsl('U', '/home/u/.nvm/versions/node/v24.20.0/bin');
    await gw.listNodeRuntimes('U');
    await gw.diagnoseWsl('U', 3100, '/home/u/.nvm/versions/node/v24.20.0/bin');
    expect(calls.map((c) => c.cmd)).toEqual(["detect_windows", "list_wsl_distros", "probe_wsl", "is_port_open", "find_free_port", "start_env", "stop_env", "layout_tabs", "open_in_browser", "run_update", "read_env_log", "diagnose_wsl", "probe_wsl", "list_node_runtimes", "diagnose_wsl"]);
    expect(calls.find((c) => c.cmd === 'probe_wsl')?.args).toEqual({ distro: 'Ubuntu', nodeRuntime: null });
    expect(calls.filter((c) => c.cmd === 'probe_wsl')[1]?.args).toEqual({ distro: 'U', nodeRuntime: '/home/u/.nvm/versions/node/v24.20.0/bin' });
    expect(calls.find((c) => c.cmd === 'run_update')?.args).toMatchObject({ version: '1.0.0' });
  });

  it('propaga gli errori del backend', async () => {
    const gw = new TauriEnvGateway((async () => { throw new Error("wsl.exe exit -1"); }) as InvokeFn);
    await expect(gw.listWslDistros()).rejects.toThrow("wsl.exe exit -1");
  });

  it('openInBrowser e mockabile (spy sul contratto)', async () => {
    const fn = vi.fn(async () => undefined);
    await new TauriEnvGateway(fn as unknown as InvokeFn).openInBrowser("http://127.0.0.1:1");
    expect(fn).toHaveBeenCalledWith("open_in_browser", { url: "http://127.0.0.1:1" });
  });
});