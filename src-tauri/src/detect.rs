//! Rilevamento dsh (SRP): solo detection Windows/WSL.
//! Dipende dalle porte CommandRunner/FsAccess (DIP): nessun Command diretto,
//! quindi i test iniettano FakeRunner/FakeFs senza spawnare processi.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::model::{EnvProbe, WslDistro};
use crate::proc::{home_dir, CommandRunner, FsAccess};
use crate::util::{first_semver, parse_wsl_distros};

pub const WSL_BOOT_TIMEOUT: Duration = Duration::from_secs(120);

/// Timeout per le sonde WSL a distro calda (la VM e gia avviata: ogni spawn
/// costa ~0.1s). Il boot freddo della VM puo richiedere decine di secondi,
/// quindi la prima sonda di ogni distro usa WSL_BOOT_TIMEOUT.
pub const WSL_WARM_TIMEOUT: Duration = Duration::from_secs(15);

/// Timeout per `dsh --version` (locale e WSL): il binario e gia risolto,
/// deve solo stampare la versione. 15s erano troppi in avvio.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// Snapshot riusabile di una scansione PATH (una sola passata sul PATH per
/// rilevamento Windows: dsh + toolchain leggono lo stesso snapshot).
pub struct PathSnapshot {
    entries: Vec<PathBuf>,
}

impl PathSnapshot {
    pub fn capture() -> Self {
        let entries = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default();
        Self { entries }
    }

    pub fn find(&self, fs: &dyn FsAccess, names: &[&str]) -> Option<PathBuf> {
        for dir in &self.entries {
            for name in names {
                let c = dir.join(name);
                if fs.path_exists(&c) {
                    return Some(c);
                }
            }
        }
        None
    }
}

/// Snapshot vuoto (test): nessuna directory nel PATH.
#[cfg(test)]
pub fn empty_path_snapshot() -> PathSnapshot {
    PathSnapshot { entries: vec![] }
}

fn package_version_of(fs: &dyn FsAccess, pkg_json: &Path) -> Option<String> {
    let text = fs.read_to_string(pkg_json)?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("version").and_then(|x| x.as_str()).map(|s| s.to_string())
}

pub fn detect_windows_with(runner: &dyn CommandRunner, fs: &dyn FsAccess, home: Option<PathBuf>) -> EnvProbe {
    detect_windows_with_path(runner, fs, home, &PathSnapshot::capture())
}

/// Come `detect_windows_with` con snapshot PATH esplicito (riusabile tra
/// rilevamenti per non riscansionare il PATH a ogni sonda; testabile con
/// `empty_path_snapshot` senza dipendere dal PATH reale).
pub fn detect_windows_with_path(
    runner: &dyn CommandRunner,
    fs: &dyn FsAccess,
    home: Option<PathBuf>,
    path: &PathSnapshot,
) -> EnvProbe {
    let mut probe = EnvProbe {
        kind: "windows".to_string(),
        name: "Windows".to_string(),
        distro: None,
        installed: false,
        version: None,
        executable: None,
        dsh_home: std::env::var("DSH_HOME").ok(),
        error: None,
        has_npm: None,
    };

    let mut version: Option<String> = None;
    let mut exe: Option<PathBuf> = None;

    if let Some(home) = home.as_ref() {
        // Versione autorevole dal package.json dell'installazione globale npm
        let pkg_candidates = [
            home.join("AppData").join("Roaming").join("npm").join("node_modules").join("@deepseek-ai").join("dsh").join("package.json"),
        ];
        for p in pkg_candidates {
            if let Some(v) = package_version_of(fs, &p) {
                version = Some(v);
                break;
            }
        }
        let exe_candidates = [
            home.join("AppData").join("Roaming").join("npm").join("dsh.cmd"),
            home.join("AppData").join("Roaming").join("npm").join("dsh"),
        ];
        for c in exe_candidates {
            if fs.path_exists(&c) {
                exe = Some(c);
                break;
            }
        }
    }

    // Cerca anche nel PATH (dsh.exe / dsh.cmd / dsh): UNA passata sullo
    // snapshot, non una per candidato.
    if exe.is_none() {
        exe = path.find(fs, &["dsh.exe", "dsh.cmd", "dsh"]);
    }

    // Se non abbiamo la versione dal package.json, prova l'eseguibile
    // (timeout corto: il file esiste gia, deve solo stampare la versione).
    if version.is_none() {
        if let Some(exe_path) = &exe {
            let exe_str = exe_path.to_string_lossy().to_string();
            if let Ok((code, out, _)) = runner.run_capture(&exe_str, &["--version"], VERSION_TIMEOUT) {
                if code == 0 {
                    version = first_semver(&out);
                }
            }
        }
    }

    probe.version = version;
    probe.executable = exe.as_ref().map(|p| p.to_string_lossy().to_string());
    probe.installed = probe.version.is_some() || probe.executable.is_some();
    probe.has_npm = Some(windows_has_npm(fs, home.as_ref(), path));
    probe
}

/// Toolchain su Windows: npm deve essere installato dall'utente (il manager
/// non lo installa mai). Rilevato via filesystem (percorsi noti) + UNA
/// scansione PATH condivisa (snapshot), senza spawnare processi (veloce e
/// testabile).
fn windows_has_npm(fs: &dyn FsAccess, home: Option<&PathBuf>, path: &PathSnapshot) -> bool {
    fn found(fs: &dyn FsAccess, home_rel: Option<PathBuf>, path: &PathSnapshot, path_names: &[&str]) -> bool {
        if let Some(p) = home_rel {
            if fs.path_exists(&p) {
                return true;
            }
        }
        path.find(fs, path_names).is_some()
    }
    // Percorsi noti sotto home, uniti in un'unica passata fs per toolchain.
    let npm_home = home_candidates_present(fs, home, &[&["AppData", "Roaming", "npm", "npm.cmd"]]);
    npm_home || found(fs, None, path, &["npm.cmd", "npm"])
}

/// True se uno dei percorsi home-rel esiste (UNA passata fs per lista).
fn home_candidates_present(fs: &dyn FsAccess, home: Option<&PathBuf>, rels: &[&[&str]]) -> bool {
    let Some(h) = home else {
        return false;
    };
    rels.iter().any(|rel| {
        let mut p = (*h).clone();
        for part in *rel {
            p = p.join(part);
        }
        fs.path_exists(&p)
    })
}

/// Produzione: home reale.
pub fn detect_windows_sync() -> EnvProbe {
    detect_windows_with(&crate::proc::SystemRunner, &crate::proc::SystemRunner, home_dir())
}

/// Argomenti dopo la distro: esecuzione DIRETTA di un binario Linux senza
/// shell (`wsl -d D -- BIN ARGS...`). Niente `bash -lc`, niente script
/// composto: ogni comando e atomico (UN binario + argv semplici, senza
/// `$`, `;`, virgolette). Verificato live: `;` e `$HOME` passano, ma `$(…)`
/// perde l'output e `export` senza quote si rompe sulle parentesi del PATH
/// ereditato — quindi la shell e bandita del tutto, non solo le virgolette.
pub fn wsl_args_after(distro: &str) -> Vec<String> {
    vec!["-d".into(), distro.to_string(), "--".into()]
}

/// PATH nativo fisso per i comandi WSL (passato via `env`, binario esterno:
/// niente shell, niente espansione). `home` e risolto da Rust via printenv
/// (vedi `wsl_home_dir`), non dalla shell. Copre dir di sistema + le
/// directory utente dei version manager (nvm/fnm/asdf) e affini: l'utente
/// non deve fare symlink ne toccare il PATH, funziona cosi com'e.
/// Mai /mnt/* (l'isolamento resta: solo percorsi Linux, mai interop).
pub fn wsl_native_path(home: &str) -> String {
    // nvm: versioni e default-alias (symlink che seguono la versione attiva).
    // fnm/asdf: symlinkmultis/+shims. local/npm-global: installazioni utente.
    // volta: ~/.volta/bin + shim. Ordine: version manager prima di sistema.
    format!(
        "{home}/.nvm/versions/node/current/bin:{home}/.nvm/current/bin:{home}/.fnm/current/bin:{home}/.asdf/shims:{home}/.asdf/bin:{home}/.volta/bin:{home}/.local/bin:{home}/.npm-global/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    )
}

/// Legge il default-alias nvm (`~/.nvm/alias/default`, file di testo con la
/// versione, es. `v24.20.0`). Comandi atomici (`cat`: binario esterno).
/// None = nessun alias (nvm senza default o assente). Pura su CommandRunner.
pub fn wsl_nvm_default_alias(runner: &dyn CommandRunner, distro: &str, home: &str) -> Option<String> {
    let mut args = wsl_args_after(distro);
    args.extend(["cat".into(), format!("{home}/.nvm/alias/default")]);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match runner.run_capture("wsl.exe", &arg_refs, WSL_WARM_TIMEOUT) {
        Ok((0, out, _)) => {
            let v = out.trim().to_string();
            if v.starts_with('v') && v.chars().any(|c| c.is_ascii_digit()) {
                return Some(v);
            }
            None
        }
        _ => None,
    }
}

/// Elenca i runtime Node della distro: versioni nvm (con flag default)
/// e node di sistema (se ha `node` nativo).
/// Comandi atomici; mai un Err lanciato (lista vuota se nulla trovato).
/// Pura rispetto a CommandRunner: testabile con fake.
pub fn list_node_runtimes_with(runner: &dyn CommandRunner, distro: &str) -> Vec<crate::model::NodeRuntime> {
    let home = match wsl_home_dir(runner, distro) {
        Ok(h) => h,
        Err(_) => return vec![],
    };
    let mut out = vec![];
    let default_alias = wsl_nvm_default_alias(runner, distro, &home);
    // Versioni nvm reali, DECRESCENTI (la piu nuova prima: scelta automatica
    // sensata quando l'utente non ha selezionato nulla).
    let mut versions = wsl_nvm_versions(runner, distro, &home);
    versions.sort_by(|a, b| cmp_semver_desc(a, b));
    for v in versions {
        let dir = format!("{home}/.nvm/versions/node/{v}/bin");
        let node_version = wsl_node_version(runner, distro, &home, &dir);
        let is_default = default_alias.as_deref() == Some(v.as_str());
        out.push(crate::model::NodeRuntime {
            id: dir,
            label: format!("nvm {v}{}", if is_default { " (default)" } else { "" }),
            node_version,
            is_default,
            source: "nvm".to_string(),
        });
    }
    // Node di sistema (PATH solo-sistema, fuori version manager).
    if let NativePath::Native(p) = wsl_system_node(runner, distro) {
        let node_version = wsl_node_version_for_bin(runner, distro, &home, &p);
        out.push(crate::model::NodeRuntime {
            id: parent_dir(&p),
            label: format!("sistema ({})", node_version.clone().unwrap_or_else(|| p.clone())),
            node_version,
            is_default: out.is_empty(),
            source: "system".to_string(),
        });
    }
    out
}

/// Confronto semver decrescente su `vMAJOR.MINOR.PATCH` (tollerante).
fn cmp_semver_desc(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0))
            .collect()
    }
    parts(b).cmp(&parts(a))
}

/// Versioni nvm grezze (`vX.Y.Z`), non ordinate. Pura su CommandRunner.
fn wsl_nvm_versions(runner: &dyn CommandRunner, distro: &str, home: &str) -> Vec<String> {
    wsl_nvm_bin_dirs(runner, distro, home)
        .into_iter()
        .filter_map(|d| {
            d.strip_prefix(&format!("{home}/.nvm/versions/node/"))
                .and_then(|s| s.strip_suffix("/bin"))
                .map(|s| s.to_string())
        })
        .collect()
}

/// `node --version` con PATH=dir (atomico). None se illeggibile.
fn wsl_node_version(runner: &dyn CommandRunner, distro: &str, home: &str, dir: &str) -> Option<String> {
    let path = format!("{dir}:{}", wsl_native_path(home));
    let mut full: Vec<String> = wsl_args_after(distro);
    full.push("env".into());
    full.push(format!("PATH={path}"));
    full.push("node".into());
    full.push("--version".into());
    let arg_refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
    match runner.run_capture("wsl.exe", &arg_refs, WSL_WARM_TIMEOUT) {
        Ok((0, out, _)) => {
            let v = out.trim().to_string();
            if v.starts_with('v') { Some(v) } else { None }
        }
        _ => None,
    }
}

/// PATH solo-sistema (niente dir utente/version manager): per il node
/// "di sistema" (`apt install nodejs`). Pura.
pub fn wsl_system_path() -> String {
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string()
}

/// Node di sistema: `node` risolto col PATH solo-sistema. Ok(Native) con
/// percorso, Ok(Missing/Interop) altrimenti. Pura su CommandRunner.
fn wsl_system_node(runner: &dyn CommandRunner, distro: &str) -> NativePath {
    let path = wsl_system_path();
    let mut full: Vec<String> = wsl_args_after(distro);
    full.push("env".into());
    full.push(format!("PATH={path}"));
    full.push("bash".into());
    full.push("-c".into());
    full.push("command -v node".to_string());
    let arg_refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
    match runner.run_capture("wsl.exe", &arg_refs, WSL_WARM_TIMEOUT) {
        Ok((0, out, _)) => native_or_interop(&out),
        _ => NativePath::Missing,
    }
}

/// Directory padre di un percorso (`/a/b/c` -> `/a/b`). Pura.
fn parent_dir(p: &str) -> String {
    p.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
}

/// `node --version` per percorso binario esatto. None se illeggibile.
fn wsl_node_version_for_bin(runner: &dyn CommandRunner, distro: &str, home: &str, bin: &str) -> Option<String> {
    let dir = parent_dir(bin);
    wsl_node_version(runner, distro, home, &dir)
}

/// Directory nvm con i binari node/npm per una versione (`…/bin`).
/// nvm installa sotto `$HOME/.nvm/versions/node/vX.Y.Z/bin`: il PATH fisso
/// copre solo i symlink `current`, quindi qui si enumerano le versioni
/// reali (via `printenv`+`ls`, comandi atomici) per risolvere npm/dsh anche
/// senza default-alias. Pura rispetto a CommandRunner.
pub fn wsl_nvm_bin_dirs(runner: &dyn CommandRunner, distro: &str, home: &str) -> Vec<String> {
    // `ls` atomico (niente shell): una riga per versione (es. v20.20.1).
    let mut args = wsl_args_after(distro);
    args.extend(["ls".into(), "-1".into(), format!("{home}/.nvm/versions/node")]);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let Ok((0, out, _)) = runner.run_capture("wsl.exe", &arg_refs, WSL_WARM_TIMEOUT) else {
        return vec![];
    };
    out.lines()
        .map(|l| l.trim())
        .filter(|v| v.starts_with('v') && v.chars().any(|c| c.is_ascii_digit()))
        .map(|v| format!("{home}/.nvm/versions/node/{v}/bin"))
        .collect()
}

/// HOME Linux della distro (`printenv HOME`: binario esterno, niente shell).
/// Pura rispetto a CommandRunner: testabile con FakeRunner.
pub fn wsl_home_dir(runner: &dyn CommandRunner, distro: &str) -> Result<String, String> {
    let mut args = wsl_args_after(distro);
    args.push("printenv".into());
    args.push("HOME".into());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, out, err) = runner.run_capture("wsl.exe", &arg_refs, WSL_WARM_TIMEOUT)?;
    let home = out.trim().to_string();
    if code == 0 && !home.is_empty() && home.starts_with('/') {
        return Ok(home);
    }
    let detail = if !err.trim().is_empty() { err.trim() } else { home.as_str() };
    Err(if detail.is_empty() {
        format!("Distro WSL \"{distro}\" non raggiungibile (HOME illeggibile).")
    } else {
        format!("Distro WSL \"{distro}\" non raggiungibile ({detail}).")
    })
}

/// Esegue UN comando atomico nella distro con PATH nativo fisso:
/// `wsl -d D -- env PATH=<nativo> BIN ARGS...` (`env` e binario esterno,
/// niente shell). Ritorna (code, stdout, stderr). Il `case /mnt/*` vive in
/// Rust sul risultato (vedi `native_or_interop`): la shell non decide nulla.
pub fn wsl_run_native(
    runner: &dyn CommandRunner,
    distro: &str,
    home: &str,
    bin: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(i32, String, String), String> {
    wsl_run_native_with_path(runner, distro, &wsl_native_path(home), bin, args, timeout)
}

/// Come `wsl_run_native` ma con PATH esplicito (es. dir nvm che contiene il
/// tool, da `wsl_tool_path`). Pura rispetto a CommandRunner.
pub fn wsl_run_native_with_path(
    runner: &dyn CommandRunner,
    distro: &str,
    path: &str,
    bin: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(i32, String, String), String> {
    let mut full: Vec<String> = wsl_args_after(distro);
    full.push("env".into());
    full.push(format!("PATH={path}"));
    full.push(bin.to_string());
    full.extend(args.iter().map(|s| s.to_string()));
    let arg_refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
    runner.run_capture("wsl.exe", &arg_refs, timeout)
}

/// Classifica un percorso risolto: nativo, interop (/mnt/*) o assente.
/// Pura: il `case /mnt/*` che prima viveva negli script shell ora vive qui.
#[derive(Debug, PartialEq)]
pub enum NativePath {
    Native(String),
    Interop(String),
    Missing,
}

pub fn native_or_interop(resolved: &str) -> NativePath {
    let p = resolved.trim();
    if p.is_empty() {
        return NativePath::Missing;
    }
    // Solo la PRIMA riga: `command -v` stampa un percorso per riga.
    let first = p.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return NativePath::Missing;
    }
    if first.starts_with("/mnt/") {
        return NativePath::Interop(first.to_string());
    }
    NativePath::Native(first.to_string())
}

/// Risolve un binario nella distro e restituisce (percorso, PATH da usare
/// per eseguirlo): il PATH include la dir che lo contiene (nvm versioni
/// comprese). Cosi run/install usano lo stesso PATH che ha risolto.
/// Pura rispetto a CommandRunner.
/// `preferred_dir`: dir scelta dall'utente (runtime Node selezionato nella
/// UI). Se presente, viene provata PER PRIMA (in testa al PATH); se il
/// binario non c'e, errore esplicito (mai fallback silenzioso a un'altra
/// versione: niente lotteria). None = automatico (base + nvm decrescenti).
pub fn wsl_tool_path(
    runner: &dyn CommandRunner,
    distro: &str,
    home: &str,
    bin: &str,
) -> Result<(NativePath, String), String> {
    wsl_tool_path_with(runner, distro, home, bin, None)
}

/// Come `wsl_tool_path` con dir preferita esplicita (runtime scelto).
/// Pura rispetto a CommandRunner.
pub fn wsl_tool_path_with(
    runner: &dyn CommandRunner,
    distro: &str,
    home: &str,
    bin: &str,
    preferred_dir: Option<&str>,
) -> Result<(NativePath, String), String> {
    let base = wsl_native_path(home);
    if let Some(dir) = preferred_dir.filter(|d| !d.is_empty()) {
        // Scelta utente: SOLO questa dir (+ base per gli altri tool).
        // Sicurezza: solo percorsi Linux assoluti sotto home/sistema;
        // niente /mnt/* (mai interop, anche se scelto a mano).
        if dir.starts_with("/mnt/") {
            return Err(format!(
                "Runtime Node non valido ({dir}): i percorsi interop Windows non sono ammessi. Seleziona un runtime nativo della distro."
            ));
        }
        if !(dir.starts_with(home) || dir.starts_with("/usr/")) {
            return Err(format!(
                "Runtime Node non valido ({dir}): usa una dir bin sotto {home} o /usr."
            ));
        }
        let path = format!("{dir}:{base}");
        match wsl_which_in(runner, distro, &path, bin) {
            Ok(NativePath::Native(p)) => return Ok((NativePath::Native(p), path)),
            Ok(NativePath::Interop(p)) => return Ok((NativePath::Interop(p), path)),
            _ => {
                return Err(format!(
                    "Runtime Node selezionato ({dir}) non contiene `{bin}` nativo: il runtime non e piu disponibile? Riselezionalo dal pannello (o torna ad Automatico)."
                ));
            }
        }
    }
    if let Ok(found) = wsl_which_in(runner, distro, &base, bin) {
        if !matches!(found, NativePath::Missing) {
            return Ok((found, base));
        }
    }
    // nvm DECRESCENTI (la piu nuova prima): scelta automatica sensata.
    let mut dirs = wsl_nvm_bin_dirs(runner, distro, home);
    dirs.sort_by(|a, b| cmp_semver_desc(&nvm_version_of(a), &nvm_version_of(b)));
    for dir in dirs {
        let path = format!("{dir}:{base}");
        if let Ok(found) = wsl_which_in(runner, distro, &path, bin) {
            if !matches!(found, NativePath::Missing) {
                return Ok((found, path));
            }
        }
    }
    Ok((NativePath::Missing, base))
}

/// Versione nvm da una dir (`…/node/v24.20.0/bin` -> `v24.20.0`). Pura.
fn nvm_version_of(dir: &str) -> String {
    dir.rsplit('/')
        .nth(1)
        .unwrap_or("")
        .to_string()
}

/// `which` con PATH esplicito (usato da `wsl_tool_path` per base + fallback).
fn wsl_which_in(
    runner: &dyn CommandRunner,
    distro: &str,
    path: &str,
    bin: &str,
) -> Result<NativePath, String> {
    let mut full: Vec<String> = wsl_args_after(distro);
    full.push("env".into());
    full.push(format!("PATH={path}"));
    full.push("bash".into());
    full.push("-c".into());
    full.push(format!("command -v {bin}"));
    let arg_refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
    let (code, out, _) = runner.run_capture("wsl.exe", &arg_refs, WSL_WARM_TIMEOUT)?;
    if code != 0 {
        return Ok(NativePath::Missing);
    }
    Ok(native_or_interop(&out))
}

/// Risolve un binario nella distro (PATH nativo) e lo classifica.
/// Pura rispetto a CommandRunner: `command` e builtin, quindi si usa
/// `bash -c` con UN comando atomico (niente `$`, `;`, virgolette: passa live).
/// `bash` senza `-l`: niente profili utente, PATH solo da `env` (nativo).
/// Fallback nvm: se il PATH fisso non risolve, si provano le dir versione
/// (`~/.nvm/versions/node/v*/bin`) con PATH esteso per quel tentativo.
/// Solo test: la produzione usa `wsl_tool_path` (serve anche il PATH risolto).
#[cfg(test)]
pub fn wsl_which(runner: &dyn CommandRunner, distro: &str, home: &str, bin: &str) -> Result<NativePath, String> {
    Ok(wsl_tool_path(runner, distro, home, bin)?.0)
}

/// Snapshot seriale di una distro per la prima pittura dell'avvio (dal
/// comando `scan_boot`: elenco + cache HOME/toolchain/dsh nota). Il frontend
/// la riusa per dipingere subito le righe; la sonda completa aggiorna poi
/// versione e stato reali. `dsh_native_path`: percorso classificato nativo
/// (mai /mnt/*); `dsh_version`: versione nota dall'ultima sonda completa.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CachedDistro {
    pub name: String,
    pub state: String,
    pub home: String,
    pub has_npm: bool,
    pub dsh_native_path: Option<String>,
    pub dsh_version: Option<String>,
}

/// Argv (dopo `--`) della sonda veloce a singolo spawn: `env PATH=<nativo
/// minimale> bash -c 'SCRIPT'` dove SCRIPT stampa 4 righe etichettate
/// (HOME, DSH, NPM + sentinella di fine). `;` e sintassi bash interna
/// (verificato live come sicuro); niente `$` (niente espansione: `command -v`
/// stampa percorsi assoluti), niente virgolette, niente `$(…)`.
///
/// Il PATH qui e quello nativo dei version manager (vedi `wsl_native_path`):
/// la sonda veloce NON enumera nvm (`ls` resterebbe un secondo spawn) — le
/// versioni nvm restano appannaggio della sonda completa in background.
pub fn fast_probe_argv(home: &str) -> Vec<String> {
    let path = wsl_native_path(home);
    let script = "echo HOME:$HOME;command -v dsh;echo DSH;command -v npm;echo NPM";
    vec![
        "env".to_string(),
        format!("PATH={path}"),
        "bash".to_string(),
        "-c".to_string(),
        script.to_string(),
    ]
}

/// Output parsato della sonda veloce: HOME (grezzo) + percorsi risolti
/// (grezzi, da classificare con `native_or_interop`). Puro.
#[derive(Debug, Default, PartialEq)]
pub struct FastProbeOutput {
    pub home: Option<String>,
    pub dsh: Option<String>,
    pub npm: Option<String>,
}

/// Smista le righe della sonda veloce: `HOME:<path>` per la prima riga, poi
/// un percorso per `command -v` seguito dalla sua etichetta (DSH/NPM).
/// Righe vuote e rumore ignorati; etichette sconosciute ignorate. Puro.
pub fn parse_fast_probe_output(out: &str) -> FastProbeOutput {
    let mut parsed = FastProbeOutput::default();
    let mut last_path: Option<String> = None;
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(home) = t.strip_prefix("HOME:") {
            let h = home.trim().to_string();
            if h.starts_with('/') {
                parsed.home = Some(h);
            }
            continue;
        }
        match t {
            "DSH" => {
                parsed.dsh = last_path.take();
            }
            "NPM" => {
                parsed.npm = last_path.take();
            }
            _ => {
                // Percorso candidato (o rumore): tiene solo l'ultimo prima
                // dell'etichetta — `command -v` stampa al massimo una riga.
                last_path = Some(t.to_string());
            }
        }
    }
    parsed
}

/// Lint anti-simboli-shell: NESSUN comando WSL puo contenere `$`, virgolette
/// (singole/doppie), backtick, pipe `|` o `&` (background).
/// Verificato live cosa si rompe davvero: `$(…)` perde l'output (GOT vuoto),
/// le virgolette vengono strip-pate via argv, `export` senza quote si rompe
/// sulle parentesi del PATH ereditato. Passano invece (verificato live):
/// `;`, `case/esac` con pattern `/mnt/*)`, `(exec …)`, `<>`, `2>/dev/null`
/// (sintassi bash interna, non testo ereditato con parentesi).
/// I comandi atomici (`wsl -d D -- BIN ARGS`) non hanno bisogno dei vietati:
/// se un nuovo comando li introduce, il test fallisce subito. Pura.
/// Solo test: la produzione non costruisce comandi da validare a runtime.
#[cfg(test)]
pub fn wsl_command_has_shell_symbols(args: &[String]) -> Option<String> {
    const FORBIDDEN: &[&str] = &["$", "'", "\"", "`", "|", "&"];
    for a in args {
        for f in FORBIDDEN {
            if a.contains(f) {
                return Some(a.clone());
            }
        }
    }
    None
}

/// Argv atomico documentativo della probe (`command -v dsh` via `bash -c`,
/// PATH nativo da `env`). La probe reale (HOME + 3 which + version) vive in
/// `probe_wsl_with`. Usato dal lint: se la forma cambia, il lint lo segue.
#[allow(dead_code)]
fn probe_command_example() -> Vec<String> {
    vec!["bash".to_string(), "-c".to_string(), "command -v dsh".to_string()]
}

/// Tutti i comandi WSL atomici eseguiti dal manager, per il lint
/// anti-simboli-shell (`wsl_command_has_shell_symbols`): ogni argv di ogni
/// comando deve esserne privo. Se un nuovo comando introduce `$`, `;`,
/// virgolette, redirect o pipe, il test fallisce subito.
/// Solo test: inventario dei comandi per il lint anti-simboli-shell.
#[cfg(test)]
pub fn all_wsl_commands_for_lint(distro: &str) -> Vec<Vec<String>> {
    let home = "/home/testuser";
    let mut cmds: Vec<Vec<String>> = vec![];
    let mut push = |bin: &str, args: &[&str]| {
        let mut full = wsl_args_after(distro);
        full.push("env".into());
        full.push(format!("PATH={}", wsl_native_path(home)));
        full.push(bin.to_string());
        full.extend(args.iter().map(|s| s.to_string()));
        cmds.push(full);
    };
    push("printenv", &["HOME"]);
    push("bash", &["-c", "command -v dsh"]);
    push("dsh", &["--version"]);
    push("bash", &["-c", "command -v npm"]);
    push("tail", &["-n", "50", "/tmp/dsh-desktop-manager-3100.log"]);
    push("pkill", &["-f", "dsh.web.--port.3100"]);
    push("npm", &["install", "-g", "@deepseek-ai/dsh@1.0.0"]);
    push("bash", &["-c", "if (exec 3<>/dev/tcp/127.0.0.1/3100) 2>/dev/null;then echo OPEN;else echo CLOSED;fi"]);
    cmds
}

/// Argv completi (dopo `--`) per la sonda porta dentro la distro:
/// `bash -c SCRIPT` dove SCRIPT non usa PATH, `$`, virgolette ne `$(…)`
/// (i tre costrutti rotti verificati live). Solo `;`, `if/fi`, `(exec …)`,
/// `<>`, `2>/dev/null`: sintassi interna, immune dal PATH ereditato.
/// Stampa OPEN o CLOSED. Il chiamante antepone `wsl_args_after(distro)`.
pub fn wsl_port_check_argv(port: u16) -> Vec<String> {
    vec![
        "bash".to_string(),
        "-c".to_string(),
        format!("if (exec 3<>/dev/tcp/127.0.0.1/{port}) 2>/dev/null;then echo OPEN;else echo CLOSED;fi"),
    ]
}

/// Sonda WSL a distro STATA (nessun spawn): per la prima pittura dell'avvio.
/// HOME e toolchain provengono dalla cache (`CachedDistro`); dsh vale solo
/// se la versione e nota. La sonda completa (`probe_wsl_with_runtime`)
/// arricchisce la riga in background. Pura (nessun I/O): testabile senza fake.
/// Nota: la produzione dipinge dal frontend (`staleProbeFor` in
/// environmentService.ts, stessa regola); questa gemella Rust resta per i
/// test di parita e per futuri usi backend.
#[cfg(test)]
pub fn stale_probe_for(
    distro: &str,
    cached: Option<&CachedDistro>,
    node_runtime: Option<&str>,
) -> EnvProbe {
    let mut probe = EnvProbe {
        kind: "wsl".to_string(),
        name: distro.to_string(),
        distro: Some(distro.to_string()),
        installed: false,
        version: None,
        executable: None,
        dsh_home: None,
        error: None,
        has_npm: None,
    };
    let Some(c) = cached else {
        return probe;
    };
    probe.has_npm = Some(c.has_npm);
    if node_runtime.is_some() {
        // Runtime scelto esplicitamente: lo stato va riverificato (il mondo
        // potrebbe essere cambiato) — niente pittura ottimistica.
        return probe;
    }
    if let Some(version) = &c.dsh_version {
        if c.dsh_native_path.is_some() {
            probe.installed = true;
            probe.version = Some(version.clone());
            probe.executable = c.dsh_native_path.clone().map(|p| format!("dsh nativo ({p})"));
        }
    }
    probe
}

/// Sonda WSL veloce in UN solo spawn (avvio + refresh periodico): `printenv
/// HOME` e `command -v dsh/npm` viaggiano in un unico `bash -c` con
/// `;`, poi Rust smista le 4 righe di output. La classificazione /mnt/* e
/// identica alla sonda completa: a parita di mondo, stesso verdetto.
///
/// Risponde alla domanda "dsh c'e e che versione ha", non "con quale PATH
/// esatto va eseguito": per start/update resta la sonda completa (serve il
/// PATH risolto con le dir nvm; qui `ls` resta fuori per restare a 1 spawn).
/// `dsh --version` resta uno spawn separato (~0.1s a distro calda) SOLO se
/// dsh risulta nativo.
///
/// Pura rispetto a CommandRunner: testabile con fake.
pub fn probe_wsl_fast_with(
    runner: &dyn CommandRunner,
    distro: &str,
    timeout: Duration,
) -> Result<EnvProbe, String> {
    let mut probe = EnvProbe {
        kind: "wsl".to_string(),
        name: distro.to_string(),
        distro: Some(distro.to_string()),
        installed: false,
        version: None,
        executable: None,
        dsh_home: None,
        error: None,
        has_npm: None,
    };
    // UN solo spawn: HOME + 2 which (PATH fisso via `env`, niente shell
    // oltre al `bash -c` con `;` — `;` e verificato live come sicuro).
    let mut args = wsl_args_after(distro);
    args.extend(fast_probe_argv("_DSH_HOME_SENTINEL_"));
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, out, err) = runner.run_capture("wsl.exe", &arg_refs, timeout)?;
    if code != 0 {
        let detail = if !err.trim().is_empty() {
            err.trim().to_string()
        } else if !out.trim().is_empty() {
            out.trim().to_string()
        } else {
            format!("wsl.exe exit {code}")
        };
        return Err(format!("Distro WSL \"{distro}\" non raggiungibile ({detail})."));
    }
    let parsed = parse_fast_probe_output(&out);
    let home = match parsed.home {
        Some(h) => h,
        None => return Err(format!("Distro WSL \"{distro}\" non raggiungibile (HOME illeggibile).")),
    };
    let dsh = parsed.dsh.map(|p| native_or_interop(&p)).unwrap_or(NativePath::Missing);
    let npm = parsed.npm.map(|p| native_or_interop(&p)).unwrap_or(NativePath::Missing);
    probe.has_npm = Some(matches!(npm, NativePath::Native(_)));
    match dsh {
        NativePath::Missing => Ok(probe),
        NativePath::Interop(p) => {
            probe.installed = false;
            probe.error = Some(format!(
                "dsh trovato solo via interop Windows ({p}): ignorato, i mondi non condividono installazioni. Installa dsh nativo nella distro (con npm della distro)."
            ));
            Ok(probe)
        }
        NativePath::Native(path) => {
            probe.installed = true;
            probe.executable = Some(format!("dsh nativo ({path})"));
            let native_path = wsl_native_path(&home);
            match wsl_run_native_with_path(runner, distro, &native_path, "dsh", &["--version"], VERSION_TIMEOUT) {
                Ok((code, out, _err)) if code == 0 => {
                    probe.version = first_semver(&out);
                    if probe.version.is_none() {
                        probe.error = Some(
                            "dsh nativo trovato ma `dsh --version` non restituisce una versione".to_string(),
                        );
                    }
                    Ok(probe)
                }
                Ok((_, out, err)) => {
                    let detail = format!("{out}\n{err}");
                    let detail = detail.trim();
                    probe.error = Some(if detail.is_empty() {
                        "dsh nativo trovato ma `dsh --version` non restituisce una versione".to_string()
                    } else {
                        format!("dsh nativo trovato ma `dsh --version` fallisce: {detail}")
                    });
                    Ok(probe)
                }
                Err(e) => {
                    probe.error = Some(format!("dsh nativo trovato ma non eseguibile: {e}"));
                    Ok(probe)
                }
            }
        }
    }
}
/// Probe WSL completa in comandi ATOMICI (niente shell, niente script composto).
/// Passi: HOME via printenv -> which dsh/npm via `bash -c command -v`
/// (UN comando, verificato live) -> classifica /mnt/* in Rust -> se dsh e
/// nativo, `dsh --version` DIRETTO (argv separati, niente shell).
/// Isolamento garantito per costruzione: nessun `$`, `;`, virgoletta,
/// redirect o pipe attraversa mai `wsl.exe`. Dietro CommandRunner (DIP).
/// `node_runtime`: dir scelta dall'utente (vedi EnvTarget). Propagata a
/// `wsl_tool_path_with`: se punta a dir sparita -> Err esplicito.
/// Solo test: la produzione passa il runtime esplicito via `probe_wsl_with_runtime`.
#[cfg(test)]
pub fn probe_wsl_with(runner: &dyn CommandRunner, distro: &str) -> Result<EnvProbe, String> {
    probe_wsl_with_runtime(runner, distro, None)
}

/// Come `probe_wsl_with` con runtime Node esplicito. Pura su porte.
pub fn probe_wsl_with_runtime(
    runner: &dyn CommandRunner,
    distro: &str,
    node_runtime: Option<&str>,
) -> Result<EnvProbe, String> {
    let mut probe = EnvProbe {
        kind: "wsl".to_string(),
        name: distro.to_string(),
        distro: Some(distro.to_string()),
        installed: false,
        version: None,
        executable: None,
        dsh_home: None,
        error: None,
        has_npm: None,
    };
    // 1) HOME della distro (serve per il PATH nativo via `env`).
    let home = wsl_home_dir(runner, distro)?;
    // 2) dsh col RUNTIME SCELTO (se presente); npm sempre in automatico
    //    (toolchain indipendente: la dir node non deve contenerlo).
    //    Runtime sparito -> Err esplicito (mai fallback silenzioso).
    //    Il PATH per `dsh --version` include la dir che lo contiene.
    let (dsh, dsh_path) = wsl_tool_path_with(runner, distro, &home, "dsh", node_runtime)?;
    let (npm, _) = wsl_tool_path(runner, distro, &home, "npm")?;
    probe.has_npm = Some(matches!(npm, NativePath::Native(_)));
    match dsh {
        NativePath::Missing => {
            probe.installed = false;
            return Ok(probe);
        }
        NativePath::Interop(p) => {
            // dsh esiste SOLO come shim Windows condiviso: mondi separati
            // -> come non installato, ma la UI spiega perche.
            probe.installed = false;
            probe.error = Some(format!(
                "dsh trovato solo via interop Windows ({p}): ignorato, i mondi non condividono installazioni. Installa dsh nativo nella distro (con npm della distro)."
            ));
            return Ok(probe);
        }
        NativePath::Native(path) => {
            // 3) Versione dal binario NATIVO (stesso PATH che lo ha risolto).
            probe.installed = true;
            probe.executable = Some(format!("dsh nativo ({path})"));
            match wsl_run_native_with_path(runner, distro, &dsh_path, "dsh", &["--version"], VERSION_TIMEOUT) {
                Ok((code, out, err)) if code == 0 => {
                    probe.version = first_semver(&out);
                    if probe.version.is_none() {
                        let detail = format!("{out}\n{err}");
                        let detail = detail.trim();
                        probe.error = Some(if detail.is_empty() {
                            "dsh nativo trovato ma `dsh --version` non restituisce una versione".to_string()
                        } else {
                            format!("dsh nativo trovato ma `dsh --version` fallisce: {detail}")
                        });
                    }
                }
                Ok((_, out, err)) => {
                    let detail = format!("{out}\n{err}");
                    let detail = detail.trim();
                    probe.error = Some(if detail.is_empty() {
                        "dsh nativo trovato ma `dsh --version` non restituisce una versione".to_string()
                    } else {
                        format!("dsh nativo trovato ma `dsh --version` fallisce: {detail}")
                    });
                }
                Err(e) => {
                    probe.error = Some(format!("dsh nativo trovato ma non eseguibile: {e}"));
                }
            }
            return Ok(probe);
        }
    }
}

pub fn list_wsl_distros_with(runner: &dyn CommandRunner) -> Result<Vec<WslDistro>, String> {
    let (code, out, err) = runner.run_capture("wsl.exe", &["-l", "-v"], WSL_WARM_TIMEOUT)?;
    if code != 0 {
        // Come in probe: wsl.exe scrive gli errori su stdout.
        let detail = if !err.trim().is_empty() {
            err.trim().to_string()
        } else if !out.trim().is_empty() {
            out.trim().to_string()
        } else {
            format!("wsl.exe exit {code}")
        };
        return Err(detail);
    }
    Ok(parse_wsl_distros(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EnvTarget;
    use crate::proc::test_support::*;
    use std::path::PathBuf;

    fn win_target() -> EnvTarget {
        EnvTarget { kind: "windows".into(), name: "Windows".into(), distro: None, port: 3080, extra_args: vec![], workspace: None, node_runtime: None }
    }

    /// Fake per la sonda veloce: UN solo spawn `bash -c` + eventuale
    /// `dsh --version` (stessi argv della produzione via `fast_probe_argv`).
    struct FastProbe {
        home: String,
        dsh: Option<String>,
        npm: Option<String>,
        version_out: Option<String>,
        spawns: std::sync::Mutex<usize>,
    }
    impl FastProbe {
        fn native() -> Self {
            Self {
                home: "/home/u".into(),
                dsh: Some("/home/u/.local/bin/dsh".into()),
                npm: None,
                version_out: Some("dsh version 1.2.3".into()),
                spawns: std::sync::Mutex::new(0),
            }
        }
        fn output(&self) -> String {
            let line = |o: &Option<String>| o.clone().unwrap_or_default();
            format!(
                "HOME:{}\n{}\nDSH\n{}\nNPM\n",
                self.home,
                line(&self.dsh),
                line(&self.npm),
            )
        }
    }
    impl CommandRunner for FastProbe {
        fn run_capture(&self, _prog: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
            *self.spawns.lock().unwrap() += 1;
            if a.contains(&"dsh") && !a.contains(&"bash") {
                return match &self.version_out {
                    Some(o) => Ok((0, format!("{o}\n"), String::new())),
                    None => Ok((1, String::new(), "no version".into())),
                };
            }
            assert!(a.contains(&"bash"), "sonda veloce: un solo spawn bash, non {a:?}");
            Ok((0, self.output(), String::new()))
        }
    }

    #[test]
    fn fast_probe_matches_full_probe_verdict() {
        // Stesso mondo, stesso verdetto: nativo installato con versione.
        let fast = probe_wsl_fast_with(&FastProbe::native(), "U", WSL_WARM_TIMEOUT).unwrap();
        let full = probe_wsl_with(&AtomProbe::native(), "U").unwrap();
        assert!(fast.installed && full.installed);
        assert_eq!(fast.version.as_deref(), Some("1.2.3"));
        assert_eq!(fast.version, full.version);
        assert_eq!(fast.executable, full.executable);
        assert_eq!(fast.has_npm, full.has_npm);
    }
    #[test]
    fn fast_probe_two_spawns_when_native() {
        let r = FastProbe::native();
        let probe = probe_wsl_fast_with(&r, "U", WSL_WARM_TIMEOUT).unwrap();
        assert!(probe.installed);
        assert_eq!(*r.spawns.lock().unwrap(), 2);
    }
    #[test]
    fn fast_probe_one_spawn_when_missing() {
        let mut r = FastProbe::native();
        r.dsh = None;
        r.npm = None;
        let probe = probe_wsl_fast_with(&r, "U", WSL_WARM_TIMEOUT).unwrap();
        assert!(!probe.installed);
        assert_eq!(probe.has_npm, Some(false));
        assert_eq!(*r.spawns.lock().unwrap(), 1);
    }
    #[test]
    fn fast_probe_interop_only_is_not_installed() {
        let mut r = FastProbe::native();
        r.dsh = Some("/mnt/c/Users/x/AppData/Roaming/npm/dsh".into());
        let probe = probe_wsl_fast_with(&r, "U", WSL_WARM_TIMEOUT).unwrap();
        assert!(!probe.installed);
        let err = probe.error.unwrap();
        assert!(err.contains("interop"), "{err}");
        // Classificazione identica alla sonda completa sullo stesso mondo.
        let mut full = AtomProbe::native();
        full.dsh = Some("/mnt/c/Users/x/AppData/Roaming/npm/dsh".into());
        let full_probe = probe_wsl_with(&full, "U").unwrap();
        assert!(!full_probe.installed);
        assert_eq!(probe.has_npm, full_probe.has_npm);
    }
    #[test]
    fn fast_probe_home_unreadable_is_error() {
        struct NoHome;
        impl CommandRunner for NoHome {
            fn run_capture(&self, _p: &str, _a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                Ok((0, "DSH\nNPM\n".into(), String::new()))
            }
        }
        assert!(probe_wsl_fast_with(&NoHome, "Nope", WSL_WARM_TIMEOUT).is_err());
    }
    #[test]
    fn parse_fast_probe_output_splits_rows() {
        let out = "HOME:/home/u\n/home/u/.local/bin/dsh\nDSH\n\nNPM\n/usr/bin/npm\nNPM\n";
        let p = parse_fast_probe_output(out);
        assert_eq!(p.home.as_deref(), Some("/home/u"));
        assert_eq!(p.dsh.as_deref(), Some("/home/u/.local/bin/dsh"));
        assert_eq!(p.npm.as_deref(), Some("/usr/bin/npm"));
    }
    #[test]
    fn parse_fast_probe_output_ignores_noise() {
        let p = parse_fast_probe_output("rumore\nHOME:relativo\nDSH\n");
        assert_eq!(p.home, None);
        assert_eq!(p.dsh.as_deref(), Some("rumore"));
    }
    #[test]
    fn stale_probe_paints_cached_version_without_runtime_choice() {
        let cached = CachedDistro {
            name: "U".into(),
            state: "Running".into(),
            home: "/home/u".into(),
            has_npm: false,
            dsh_native_path: Some("/home/u/.local/bin/dsh".into()),
            dsh_version: Some("1.2.3".into()),
        };
        let probe = stale_probe_for("U", Some(&cached), None);
        assert!(probe.installed);
        assert_eq!(probe.version.as_deref(), Some("1.2.3"));
        assert_eq!(probe.has_npm, Some(false));
        // Runtime scelto: niente pittura ottimistica (va riverificato).
        let cautious = stale_probe_for("U", Some(&cached), Some("/home/u/.nvm/x/bin"));
        assert!(!cautious.installed);
        assert_eq!(cautious.has_npm, Some(false));
        // Senza cache: riga vuota in attesa (mai "Non installato" falso).
        let empty = stale_probe_for("U", None, None);
        assert!(!empty.installed);
        assert!(empty.version.is_none());
        assert_eq!(empty.has_npm, None);
    }
    #[test]
    fn detect_windows_single_pass_path_snapshot() {
        // PATH finto con dsh + npm: UNA passata li trova tutti.
        let dir = PathBuf::from("/tools");
        let mut fs = FakeFs::default();
        fs.files.insert(dir.join("dsh.exe"), String::new());
        fs.files.insert(dir.join("npm.cmd"), String::new());
        let snap = PathSnapshot { entries: vec![dir] };
        let probe = detect_windows_with_path(&FakeRunner::default(), &fs, None, &snap);
        assert!(probe.installed);
        assert!(probe.executable.unwrap().contains("dsh.exe"));
        assert_eq!(probe.has_npm, Some(true));
        // Snapshot vuoto: niente PATH reale toccato, tutto mancante.
        let probe2 = detect_windows_with_path(&FakeRunner::default(), &FakeFs::default(), None, &empty_path_snapshot());
        assert!(!probe2.installed);
        assert_eq!(probe2.has_npm, Some(false));
    }

    #[test]
    fn wsl_args_shape() {
        // Esecuzione diretta SENZA shell: dopo `--` va il binario Linux.
        assert_eq!(wsl_args_after("Ubuntu"), vec!["-d", "Ubuntu", "--"]);
    }
    #[test]
    fn native_or_interop_classifies_paths() {
        assert_eq!(native_or_interop(""), NativePath::Missing);
        assert_eq!(native_or_interop("   \n"), NativePath::Missing);
        assert_eq!(
            native_or_interop("/mnt/c/Users/x/AppData/Roaming/npm/dsh\n"),
            NativePath::Interop("/mnt/c/Users/x/AppData/Roaming/npm/dsh".into())
        );
        assert_eq!(
            native_or_interop("/home/u/.local/bin/dsh\n"),
            NativePath::Native("/home/u/.local/bin/dsh".into())
        );
        // Solo la prima riga conta (difesa contro output rumorosi).
        assert_eq!(
            native_or_interop("/home/u/.local/bin/dsh\n/mnt/c/x\n"),
            NativePath::Native("/home/u/.local/bin/dsh".into())
        );
    }
    #[test]
    fn wsl_nvm_bin_dirs_lists_versions() {
        // `ls` atomico su ~/.nvm/versions/node: una dir bin per versione.
        struct Ls;
        impl CommandRunner for Ls {
            fn run_capture(&self, _p: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                assert!(a.contains(&"ls"));
                Ok((0, "v20.20.1\nv22.5.0\nfile.txt\n".into(), String::new()))
            }
        }
        let dirs = wsl_nvm_bin_dirs(&Ls, "U", "/home/u");
        assert_eq!(dirs, vec![
            "/home/u/.nvm/versions/node/v20.20.1/bin".to_string(),
            "/home/u/.nvm/versions/node/v22.5.0/bin".to_string(),
        ]);
    }
    #[test]
    fn wsl_nvm_bin_dirs_empty_when_no_nvm() {
        struct Fail;
        impl CommandRunner for Fail {
            fn run_capture(&self, _p: &str, _a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                Err("no ls".into())
            }
        }
        assert!(wsl_nvm_bin_dirs(&Fail, "U", "/home/u").is_empty());
    }
    #[test]
    fn wsl_tool_path_prefers_descending_nvm() {
        // Senza scelta: vince la piu nuova (decrescente), non la prima di ls.
        struct TwoOld;
        impl CommandRunner for TwoOld {
            fn run_capture(&self, _p: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                if a.contains(&"printenv") {
                    return Ok((0, "/home/u\n".into(), String::new()));
                }
                if a.contains(&"ls") {
                    return Ok((0, "v20.20.1\nv24.20.0\n".into(), String::new()));
                }
                if a.contains(&"bash") {
                    // PATH con v24 -> trovato; altrimenti assente.
                    if a.iter().any(|x| x.contains("v24.20.0")) {
                        return Ok((0, "/home/u/.nvm/versions/node/v24.20.0/bin/dsh\n".into(), String::new()));
                    }
                    return Ok((1, String::new(), String::new()));
                }
                Err(format!("non stubbato: {a:?}"))
            }
        }
        let (found, path) = wsl_tool_path(&TwoOld, "U", "/home/u", "dsh").unwrap();
        assert_eq!(found, NativePath::Native("/home/u/.nvm/versions/node/v24.20.0/bin/dsh".into()));
        assert!(path.contains("v24.20.0"), "{path}");
    }
    #[test]
    fn wsl_tool_path_rejects_mnt_preferred() {
        let r = AtomProbe::native();
        let e = wsl_tool_path_with(&r, "U", "/home/u", "dsh", Some("/mnt/c/x/bin")).unwrap_err();
        assert!(e.contains("interop"), "{e}");
    }
    #[test]
    fn list_node_runtimes_sorts_desc_and_marks_default() {
        struct Rts;
        impl CommandRunner for Rts {
            fn run_capture(&self, _p: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                if a.contains(&"printenv") {
                    return Ok((0, "/home/u\n".into(), String::new()));
                }
                if a.contains(&"cat") {
                    return Ok((0, "v20.20.1\n".into(), String::new()));
                }
                if a.contains(&"ls") {
                    return Ok((0, "v20.20.1\nv24.20.0\n".into(), String::new()));
                }
                if a.contains(&"node") {
                    return Ok((0, "v24.20.0\n".into(), String::new()));
                }
                if a.contains(&"bash") {
                    return Ok((1, String::new(), String::new()));
                }
                Err(format!("non stubbato: {a:?}"))
            }
        }
        let list = list_node_runtimes_with(&Rts, "U");
        assert_eq!(list.len(), 2);
        assert!(list[0].id.contains("v24.20.0"), "{list:?}");
        assert!(!list[0].is_default);
        assert!(list[1].is_default);
        assert!(list[1].label.contains("default"));
    }
    #[test]
    fn wsl_tool_path_falls_back_to_nvm_version() {
        // npm SOLO in ~/.nvm/versions/node/v20.20.1/bin: risolto via fallback,
        // col PATH che contiene quella dir (niente symlink utente).
        struct NvmOnly;
        impl CommandRunner for NvmOnly {
            fn run_capture(&self, _p: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                if a.contains(&"printenv") {
                    return Ok((0, "/home/jacob\n".into(), String::new()));
                }
                if a.contains(&"ls") {
                    return Ok((0, "v20.20.1\n".into(), String::new()));
                }
                if a.contains(&"bash") {
                    let has_nvm = a.iter().any(|x| x.contains(".nvm/versions/node/v20.20.1/bin"));
                    if has_nvm {
                        return Ok((0, "/home/jacob/.nvm/versions/node/v20.20.1/bin/npm\n".into(), String::new()));
                    }
                    return Ok((1, String::new(), String::new()));
                }
                Err(format!("non stubbato: {a:?}"))
            }
        }
        let (found, path) = wsl_tool_path(&NvmOnly, "U", "/home/jacob", "npm").unwrap();
        assert_eq!(found, NativePath::Native("/home/jacob/.nvm/versions/node/v20.20.1/bin/npm".into()));
        assert!(path.contains(".nvm/versions/node/v20.20.1/bin"), "{path}");
        assert!(!path.contains("/mnt/"));
    }
    #[test]
    fn wsl_native_path_starts_with_nvm_current() {
        let p = wsl_native_path("/home/u");
        assert!(p.starts_with("/home/u/.nvm/versions/node/current/bin:"));
        assert!(p.contains("/usr/local/bin"));
        assert!(!p.contains("/mnt/"));
    }
    #[test]
    fn fast_probe_argv_is_env_path_plus_bash() {
        // Isolamento: UN solo `env` (PATH) davanti a `bash -c`, quindi
        // nessuna variabile ambiente oltre al PATH entra nella distro.
        let argv = fast_probe_argv("/home/u");
        assert_eq!(argv[0], "env");
        assert_eq!(argv[1], format!("PATH={}", wsl_native_path("/home/u")));
        assert_eq!(argv[2], "bash");
        assert_eq!(argv[3], "-c");
        assert_eq!(argv.len(), 5);
    }
    #[test]
    fn no_shell_symbols_in_wsl_commands() {
        // REGRESSIONE anti-bug (tre casi reali verificati live su distro):
        // `$(…)` perde l'output, `export` senza quote si rompe sulle
        // parentesi del PATH ereditato, le virgolette vengono strip-pate.
        // I comandi atomici non ne hanno bisogno: se uno ne introduce,
        // il test fallisce subito.
        for cmd in all_wsl_commands_for_lint("Ubuntu") {
            assert!(
                wsl_command_has_shell_symbols(&cmd).is_none(),
                "simbolo shell in comando WSL: {cmd:?}"
            );
        }
    }
    /// Runner atomico per la probe: printenv/which/version per argv.
    struct AtomProbe {
        home: Option<String>,
        dsh: Option<String>,
        npm: Option<String>,
        version: Option<(i32, String, String)>,
    }
    impl AtomProbe {
        fn native() -> Self {
            Self {
                home: Some("/home/u".into()),
                dsh: Some("/home/u/.local/bin/dsh".into()),
                npm: None,
                version: Some((0, "dsh version 1.2.3".into(), String::new())),
            }
        }
    }
    impl CommandRunner for AtomProbe {
        fn run_capture(&self, _prog: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
            if a.contains(&"printenv") {
                return match &self.home {
                    Some(h) => Ok((0, format!("{h}\n"), String::new())),
                    None => Ok((1, String::new(), "no home".into())),
                };
            }
            if a.contains(&"bash") {
                let cmd = a.iter().find(|x| x.starts_with("command -v")).cloned().unwrap_or_default();
                let bin = cmd.split_whitespace().last().unwrap_or("");
                let hit = match bin { "dsh" => &self.dsh, "npm" => &self.npm, _ => &None };
                return match hit {
                    Some(p) => Ok((0, format!("{p}\n"), String::new())),
                    None => Ok((1, String::new(), String::new())),
                };
            }
            if a.contains(&"dsh") {
                return match &self.version {
                    Some((c, o, e)) => Ok((*c, o.clone(), e.clone())),
                    None => Ok((1, String::new(), "no version".into())),
                };
            }
            Err(format!("non stubbato: {a:?}"))
        }
    }
    #[test]
    #[ignore]
    fn live_atomic_commands() {
        // Test LIVE del disegno atomico (`cargo test live_atomic_commands --
        // --ignored --nocapture`): printenv, which, versione, sonda porta e
        // probe completa sulla distro reale. Verificato su Ubuntu-24.04 col
        // PATH dev pieno di parentesi: zero syntax error, shim /mnt/c
        // invisibile (dsh/npm -> Missing), sonda CLOSED, probe coerente.
        // Stessi argv di SystemRunner (wsl_args_after + env + binario).
        let r = crate::proc::SystemRunner;
        let home = wsl_home_dir(&r, "Ubuntu-24.04").expect("home live");
        println!("HOME={home:?}");
        for bin in ["dsh", "npm"] {
            let w = wsl_which(&r, "Ubuntu-24.04", &home, bin).expect("which live");
            println!("{bin} -> {w:?}");
        }
        let mut args = wsl_args_after("Ubuntu-24.04");
        for a in wsl_port_check_argv(31999) {
            args.push(a);
        }
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = crate::proc::SystemRunner;
        let res = crate::proc::CommandRunner::run_capture(&out, "wsl.exe", &refs, std::time::Duration::from_secs(30)).expect("sonda live");
        println!("PORTA={res:?}");
        let probe = probe_wsl_with(&r, "Ubuntu-24.04").expect("probe live");
        println!("PROBE installed={} version={:?} npm={:?} err={:?}", probe.installed, probe.version, probe.has_npm, probe.error);
        let rts = list_node_runtimes_with(&r, "Ubuntu-24.04");
        for rt in &rts {
            println!("RUNTIME id={} label={} node={:?} default={} src={}", rt.id, rt.label, rt.node_version, rt.is_default, rt.source);
        }
        // Probe col runtime v24 scelto esplicitamente (come fara la UI).
        if let Some(v24) = rts.iter().find(|x| x.id.contains("v24.")) {
            let p2 = probe_wsl_with_runtime(&r, "Ubuntu-24.04", Some(&v24.id)).expect("probe-24 live");
            println!("PROBE-24 installed={} version={:?} exe={:?} err={:?}", p2.installed, p2.version, p2.executable, p2.error);
        }
    }
    #[test]
    fn probe_interop_only_is_not_installed() {
        // dsh SOLO via interop Windows: mondi separati -> non installato,
        // con spiegazione del percorso condiviso ignorato.
        let mut r = AtomProbe::native();
        r.dsh = Some("/mnt/c/Users/x/AppData/Roaming/npm/dsh".into());
        r.npm = Some("/usr/bin/npm".into());
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert!(probe.version.is_none());
        let err = probe.error.unwrap();
        assert!(err.contains("/mnt/c/Users/x/AppData/Roaming/npm/dsh"), "{err}");
        assert!(err.contains("interop"), "{err}");
        // npm nativo resta rilevato: toolchain indipendente da dsh.
        assert_eq!(probe.has_npm, Some(true));
    }
    #[test]
    fn probe_native_ok_reports_native_path() {
        let probe = probe_wsl_with(&AtomProbe::native(), "U").unwrap();
        assert!(probe.installed);
        assert_eq!(probe.version.as_deref(), Some("1.2.3"));
        assert!(probe.executable.unwrap().contains("/home/u/.local/bin/dsh"));
        assert_eq!(probe.has_npm, Some(false));
    }
    #[test]
    fn probe_toolchain_interop_only_is_missing() {
        // npm SOLO via interop: ignorato -> mancante.
        let mut r = AtomProbe::native();
        r.dsh = None;
        r.npm = Some("/mnt/c/Program Files/nodejs/npm".into());
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert_eq!(probe.has_npm, Some(false));
    }
    #[test]
    fn probe_not_found_marks_uninstalled() {
        let mut r = AtomProbe::native();
        r.dsh = None;
        r.npm = None;
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert_eq!(probe.distro.as_deref(), Some("U"));
        assert_eq!(probe.has_npm, Some(false));
    }
    #[test]
    fn probe_no_toolchain_reports_missing() {
        let mut r = AtomProbe::native();
        r.dsh = None;
        r.npm = None;
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert_eq!(probe.has_npm, Some(false));
    }
    #[test]
    fn probe_ok_without_version_keeps_error_detail() {
        // dsh NATIVO esiste ma --version fallisce: resta installato.
        let mut r = AtomProbe::native();
        r.version = Some((1, String::new(), "permesso negato".into()));
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(probe.installed);
        assert!(probe.version.is_none());
        assert!(probe.error.unwrap().contains("permesso negato"));
    }
    #[test]
    fn probe_home_unreadable_is_error() {
        let mut r = AtomProbe::native();
        r.home = None;
        assert!(probe_wsl_with(&r, "Nope").is_err());
    }
    #[test]
    fn probe_runner_error_propagates() {
        let r = FakeRunner::with_error("wsl.exe", &["-d"], "timeout");
        assert!(probe_wsl_with(&r, "X").is_err());
    }
    #[test]
    fn wsl_home_dir_reads_printenv() {
        assert_eq!(wsl_home_dir(&AtomProbe::native(), "U").as_deref(), Ok("/home/u"));
        let mut no_home = AtomProbe::native();
        no_home.home = None;
        assert!(wsl_home_dir(&no_home, "U").is_err());
    }
    #[test]
    fn detect_windows_from_fake_package_json() {
        // Percorsi costruiti con join: stessa forma di `detect_windows_with`
        // (separatori nativi, altrimenti il fake non li trova su Windows).
        let home = PathBuf::from("/home/u");
        let pkg = home.join("AppData").join("Roaming").join("npm").join("node_modules").join("@deepseek-ai").join("dsh").join("package.json");
        let fs = FakeFs::with(&pkg.to_string_lossy(), "{\"version\": \"1.2.3\"}");
        let runner = FakeRunner::default();
        let probe = detect_windows_with(&runner, &fs, Some(home));
        assert!(probe.installed);
        assert_eq!(probe.version.as_deref(), Some("1.2.3"));
    }
    #[test]
    fn detect_windows_falls_back_to_version_flag() {
        let home = PathBuf::from("/home/u");
        let exe = home.join("AppData").join("Roaming").join("npm").join("dsh.cmd");
        let exe = exe.to_string_lossy().into_owned();
        let fs = FakeFs::with(&exe, "");
        let runner = FakeRunner::with_output(&exe, &["--version"], 0, "dsh version 2.0.0", "");
        let probe = detect_windows_with(&runner, &fs, Some(home));
        assert!(probe.installed);
        assert_eq!(probe.version.as_deref(), Some("2.0.0"));
    }
    #[test]
    fn list_distros_parses_fake_output() {
        let out = "NAME S V\n* Ubuntu Running 2\n";
        let r = FakeRunner::with_output("wsl.exe", &["-l", "-v"], 0, out, "");
        let list = list_wsl_distros_with(&r).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Ubuntu");
    }
    #[test]
    fn list_distros_reports_stderr_on_failure() {
        let r = FakeRunner::with_output("wsl.exe", &["-l", "-v"], 1, "", "wsl assente");
        assert_eq!(list_wsl_distros_with(&r).unwrap_err(), "wsl assente");
    }
    #[test]
    fn list_distros_propagates_runner_error() {
        let r = FakeRunner::with_error("wsl.exe", &["-l", "-v"], "timeout");
        assert_eq!(list_wsl_distros_with(&r).unwrap_err(), "timeout");
    }
    #[test]
    fn win_target_helper_builds() {
        assert_eq!(win_target().port, 3080);
    }
}
