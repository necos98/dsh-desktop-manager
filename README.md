# DSH Desktop Manager

Gestore desktop per **DeepSeek Harness (DSH)** su Windows e distro **WSL**.

Apri l'app e per ogni ambiente (Windows, ogni distro WSL) vedi:
- se **dsh è installato** e **quale versione**;
- se c'è un **aggiornamento** (confronto con il registry npm `@deepseek-ai/dsh`);
- i pulsanti **Avvia / Ferma**, l'apertura della **GUI di DSH in una finestra WebView2** (motore Edge/Chromium) e l'apertura nel browser di sistema;
- la possibilità di **scegliere e pinnare una versione arbitraria** per ciascun ambiente (ogni ambiente può puntare a una versione diversa: latest stabile, rc, alpha o una versione esatta).

## Architettura

```
┌─ Frontend (TypeScript + Vite) ────────────────┐
│  lista ambienti · dettagli · azioni · registro │
└──────────────────┬────────────────────────────┘
                   │ invoke (comandi Tauri)
┌─ Backend (Rust / Tauri 2) ────────────────────┐
│  detect_windows()     → versione dsh su Windows│
│  list_wsl_distros()   → wsl -l -v (UTF-16)    │
│  probe_wsl(distro)    → versione dentro distro │
│  start_env/stop_env   → dsh web su porta       │
│  open_gui             → finestra WebView2      │
│  run_update           → bun add -g @deepseek-ai/dsh@ver │
└────────────────────────────────────────────────┘
   • ogni ambiente gira sulla propria porta (default: Windows 3080,
     distro WSL 3100+) → più DSH in parallelo e switch istantaneo
   • la versione "disponibile" arriva dal registry npm (CORS aperto)
```

## Requisiti di sviluppo

- Node ≥ 20 e npm
- Rust toolchain MSVC (stable) + Visual Studio 2022 Build Tools (componente C++)
- WebView2 Runtime (già incluso in Windows 11)

## Comandi

```bash
npm install          # dipendenze frontend + CLI Tauri
npm run tauri dev    # avvio in sviluppo (compila il backend Rust)
npm run tauri build  # build di produzione (installer NSIS in src-tauri/target/release/bundle)
```

## Note operative

- **Porte**: se la porta configurata è occupata (es. 3080 usata da un altro DSH),
  il manager sceglie automaticamente la prima libera e la salva nell'ambiente.
- **WSL2**: la GUI di un DSH lanciato in una distro è raggiungibile da Windows
  tramite il localhost-forwarding di WSL2 **solo se** dsh ascolta su `0.0.0.0`
  (o con networking WSL in modalità *mirror*). Se la pagina non si apre, aggiungi
  `--host 0.0.0.0` in *Argomenti extra* dell'ambiente e riavvia.
- **Arresto**: il manager ferma solo i processi che ha avviato lui (mai un DSH
  esterno, per sicurezza). Alla chiusura dell'app i processi avviati vengono arrestati.
- **Aggiornamento**: richiede `bun` (o `npm`) nell'ambiente di destinazione.
  Su Windows usa `bun add -g @deepseek-ai/dsh@<versione>`; dentro WSL esegue lo
  stesso comando nella distro.

## Versionamento e release

Il software è versionato in un unico punto (`tools/bump-version.mjs` allinea
`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`/
`Cargo.lock` e `CHANGELOG.md`). Ogni tag `v*` fa partire la GitHub Action
`Release`, che compila l'installer Windows (NSIS) e lo pubblica come
**GitHub Release scaricabile** (asset `.exe`/`.nsis.exe` nella pagina Release).

```bash
npm test
npm run release:patch   # fix: 0.1.0 -> 0.1.1 (commit + tag v0.1.1 + push)
npm run release:minor   # feature: 0.1.0 -> 0.2.0
npm run release:major   # breaking: 0.1.0 -> 1.0.0
npm run release -- 0.2.0-rc.1  # versione esatta (il suffisso -rc.1 la marca pre-release)
```

Il tag deve coincidere con la versione nei manifest (lo garantisce lo script);
la Action fallisce in modo esplicito se tag e manifest divergono.

## Struttura

```
dsh-desktop-manager/
├─ index.html / vite.config.ts / tsconfig.json
├─ src/                  # frontend (main.ts, api.ts, registry.ts, types.ts)
├─ tools/gen-icon.mjs    # genera le icone senza dipendenze
└─ src-tauri/
   ├─ tauri.conf.json
   ├─ capabilities/default.json
   └─ src/main.rs        # backend: rilevamento, WSL, avvio/stop, update, GUI
```
