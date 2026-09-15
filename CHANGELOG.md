# Changelog

Formato ispirato a [Keep a Changelog](https://keepachangelog.com/it/1.0.0/).
Ogni versione pubblica corrisponde a un tag `v*` su GitHub, compilato e
pubblicato come Release scaricabile dalla Action `Release`.

## [Unreleased]

## [0.5.1] - 2026-09-15

### Fixed

- Un dsh installato via bun (dir `~/.bun/bin` nel PATH) non viene piu rilevato: l'ambiente risulta non installato.
- Update/install di dsh fallito: la UI mostra l'output completo catturato (stdout+stderr, es. spawn di npm non partito -> exit -1) in un blocco monospace con pulsante «Copia», invece della sola riga "fallito (exit -1)"; il messaggio indica il file di log scritto dal backend (`logs\update-<kind>-<label>-<yyyyMMdd-HHmmss>.log`) che contiene lo stesso output.

## [0.5.0] - 2026-09-15

### Added

- Avvio veloce: comando `scan_boot` (elenco distro + cache toolchain/dsh per distro) e sonda WSL veloce a singolo spawn (HOME e `command -v dsh/npm` in un unico `bash -c`), cosi la finestra e interattiva subito.
- Prima pittura progressiva: le righe note appaiono subito e la sonda completa arricchisce versione/stato in background (`staleProbeFor` nel frontend, gemella Rust `stale_probe_for`).
- Registry npm cachato (TTL 6 ore) con fetch a timeout: l'avvio non aspetta mai la rete.
- Strumentazione cold-boot: `BOOT <fase> +<ms>` lato frontend e log di fase lato Rust, per misurare le regressioni di avvio.

### Changed

- Rimosso ogni supporto bun: toolchain, installer e percorsi `~/.bun`; dsh si installa e si aggiorna solo con npm.
- Scansione del PATH condivisa (snapshot con TTL) fra i rilevamenti: una sola passata per finestra invece di una per sonda; controllo aggiornamenti rinviato a dopo la prima pittura.

## [0.4.0] - 2026-09-08

## [0.4.0] - 2026-09-08

### Added

- Runtime Node selezionabile per distro WSL (nvm decrescenti + sistema): comando `list_node_runtimes`, probe/start/update/diagnostica propagano la scelta, errore esplicito se la dir sparisce.
- Diagnostica WSL passo-passo (`diagnose_wsl`): checklist stato-distro/dsh/toolchain/porta-dentro/porta-Windows/log-tail, mai un throw (fallimenti in `WslDiag.error`).
- Direzioni di versione complete: upgrade, downgrade, reinstall e install con verbi italiani dedicati; installazione con versione pinnata e fallback npm.
- Pannello log per ambiente (`read_env_log`) e preflight WSL fail-fast prima di start/update.
- Avvisi toolchain nativa (bun/npm ignorano l'interop /mnt/*) quando ne bun ne npm sono presenti.

### Changed

- Comandi WSL atomici senza shell (`wsl -d D -- BIN ARGS...`, niente `bash -lc` composto) con lint anti-simboli-shell nei test.
- Spawn WSL senza setsid (relay vivo) e nessun prompt lampeggiante per i processi figli su Windows.
- Helper usati solo dai test (`wsl_which`, `wsl_env_prefix`, `wsl_command_has_shell_symbols`, `all_wsl_commands_for_lint`, `probe_wsl_with`, `preflight_wsl`, `diagnose_wsl_with`) marcati `#[cfg(test)]`: zero warning `dead_code` in `dev`.

## [0.3.0] - 2026-09-07

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
