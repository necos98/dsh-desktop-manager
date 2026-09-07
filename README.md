# DSH Desktop Manager

Desktop companion for **DeepSeek Harness (DSH)** on Windows and WSL distros.

For each environment (Windows + every WSL distro) it shows whether DSH is installed, which version is running, and whether an update is available (via the `@deepseek-ai/dsh` npm registry). You can start/stop each DSH on its own port, open its GUI in an embedded WebView2 window or system browser, and pin a different version per environment (latest, rc, alpha, or exact).

## Quick start

```bash
npm install
npm run tauri dev    # dev run (builds the Rust backend)
npm run tauri build  # production installer (NSIS)
```

Requires Node >= 20, Rust MSVC toolchain + VS 2022 Build Tools (C++), and WebView2 Runtime (built into Windows 11).
