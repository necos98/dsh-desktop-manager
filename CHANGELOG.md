# Changelog

Formato ispirato a [Keep a Changelog](https://keepachangelog.com/it/1.0.0/).
Ogni versione pubblica corrisponde a un tag `v*` su GitHub, compilato e
pubblicato come Release scaricabile dalla Action `Release`.

## [Unreleased]

### Added

- Auto-update del manager direttamente dall'app (tauri-plugin-updater): controllo
  automatico all'avvio, pulsante nella tabbar e sezione "Aggiornamento manager"
  con download, installazione e riavvio. Artefatti firmati (minisign) e pubblicati
  come `.sig` + `latest.json` nelle GitHub Releases.
- Icona Windows multi-risoluzione (ICO con entry 16-256px) per taskbar, barra del
  titolo e anteprime nitide.

## [0.2.1] - 2026-09-07

### Changed

- Icona app, anteprima e taskbar: ora usano in toto `./icon.png` (balena), senza reinterpretazioni; rigenerati `icon.ico`/`icon.png`/`32x32`/`128x128`/`256x256` da quello stesso file e favicon web da `/icon.png`.

## [0.2.0] - 2026-09-07

### Added

- Icona app ridisegnata: balena bianca in stile emoji su sfondo oceano (+ favicon con emoji balena nella scheda browser).
- Link esterni delle webview (non GUI locale) aperti nel browser di sistema; `window.open` / target=_blank negati dopo l'apertura.

## [0.1.0] - 2026-09-07

### Added

- Prima release pubblica: gestore desktop DSH (Windows + distro WSL).
- Rilevamento installazioni dsh, avvio/stop, GUI in WebView2, pin delle versioni.
- Installer Windows (NSIS) compilato e pubblicato automaticamente a ogni tag `v*`.
