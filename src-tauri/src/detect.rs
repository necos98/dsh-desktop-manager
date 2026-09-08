//! Rilevamento dsh (SRP): solo detection Windows/WSL.
//! Dipende dalle porte CommandRunner/FsAccess (DIP): nessun Command diretto,
//! quindi i test iniettano FakeRunner/FakeFs senza spawnare processi.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::model::{EnvProbe, WslDistro};
use crate::proc::{home_dir, CommandRunner, FsAccess};
use crate::util::{first_semver, parse_wsl_distros};

pub const WSL_BOOT_TIMEOUT: Duration = Duration::from_secs(120);

fn package_version_of(fs: &dyn FsAccess, pkg_json: &Path) -> Option<String> {
    let text = fs.read_to_string(pkg_json)?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("version").and_then(|x| x.as_str()).map(|s| s.to_string())
}

pub fn detect_windows_with(runner: &dyn CommandRunner, fs: &dyn FsAccess, home: Option<PathBuf>) -> EnvProbe {
    let mut probe = EnvProbe {
        kind: "windows".to_string(),
        name: "Windows".to_string(),
        distro: None,
        installed: false,
        version: None,
        executable: None,
        dsh_home: std::env::var("DSH_HOME").ok(),
        error: None,
        has_bun: None,
        has_npm: None,
    };

    let mut version: Option<String> = None;
    let mut exe: Option<PathBuf> = None;

    if let Some(home) = home.as_ref() {
        // Versione autorevole dal package.json dell'installazione globale
        let pkg_candidates = [
            home.join(".bun").join("install").join("global").join("node_modules").join("@deepseek-ai").join("dsh").join("package.json"),
            home.join("AppData").join("Roaming").join("npm").join("node_modules").join("@deepseek-ai").join("dsh").join("package.json"),
        ];
        for p in pkg_candidates {
            if let Some(v) = package_version_of(fs, &p) {
                version = Some(v);
                break;
            }
        }
        let exe_candidates = [
            home.join(".bun").join("bin").join("dsh.exe"),
            home.join(".bun").join("bin").join("dsh"),
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

    // Cerca anche nel PATH (dsh.exe / dsh.cmd / dsh)
    if exe.is_none() {
        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                for name in ["dsh.exe", "dsh.cmd", "dsh"] {
                    let c = dir.join(name);
                    if fs.path_exists(&c) {
                        exe = Some(c);
                        break;
                    }
                }
                if exe.is_some() {
                    break;
                }
            }
        }
    }

    // Se non abbiamo la versione dal package.json, prova l'eseguibile
    if version.is_none() {
        if let Some(exe_path) = &exe {
            let exe_str = exe_path.to_string_lossy().to_string();
            if let Ok((code, out, _)) = runner.run_capture(&exe_str, &["--version"], Duration::from_secs(15)) {
                if code == 0 {
                    version = first_semver(&out);
                }
            }
        }
    }

    probe.version = version;
    probe.executable = exe.as_ref().map(|p| p.to_string_lossy().to_string());
    probe.installed = probe.version.is_some() || probe.executable.is_some();
    let (has_bun, has_npm) = windows_toolchains(fs, home.as_ref());
    probe.has_bun = Some(has_bun);
    probe.has_npm = Some(has_npm);
    probe
}

/// Toolchain su Windows: bun e npm devono essere installati dall'utente
/// (il manager non li installa mai). Rilevati via filesystem (percorsi
/// noti) + scansione PATH, senza spawnare processi (veloce e testabile).
fn windows_toolchains(fs: &dyn FsAccess, home: Option<&PathBuf>) -> (bool, bool) {
    fn found(fs: &dyn FsAccess, home: Option<&PathBuf>, home_rel: &[&str], path_names: &[&str]) -> bool {
        if let Some(h) = home {
            let mut p = h.clone();
            for part in home_rel {
                p = p.join(part);
            }
            if fs.path_exists(&p) {
                return true;
            }
        }
        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                for name in path_names {
                    if fs.path_exists(&dir.join(name)) {
                        return true;
                    }
                }
            }
        }
        false
    }
    let has_bun = found(fs, home, &["bun", "bin", "bun.exe"], &["bun.exe", "bun"])
        || found(fs, home, &[".bun", "bin", "bun.exe"], &["bun.exe", "bun"]);
    let has_npm = found(
        fs,
        home,
        &["AppData", "Roaming", "npm", "npm.cmd"],
        &["npm.cmd", "npm"],
    );
    (has_bun, has_npm)
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
/// (vedi `wsl_home_dir`), non dalla shell. Copre dir di sistema + bun + le
/// directory utente dei version manager (nvm/fnm/asdf) e affini: l'utente
/// non deve fare symlink ne toccare il PATH, funziona cosi com'e.
/// Mai /mnt/* (l'isolamento resta: solo percorsi Linux, mai interop).
pub fn wsl_native_path(home: &str) -> String {
    // nvm: versioni e default-alias (symlink che seguono la versione attiva).
    // fnm/asdf: symlinkmultis/+shims. local/npm-global: installazioni utente.
    // volta: ~/.volta/bin + shim. Ordine: version manager prima di sistema.
    format!(
        "{home}/.bun/bin:{home}/.nvm/versions/node/current/bin:{home}/.nvm/current/bin:{home}/.fnm/current/bin:{home}/.asdf/shims:{home}/.asdf/bin:{home}/.volta/bin:{home}/.local/bin:{home}/.npm-global/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    )
}

/// Legge il default-alias nvm (`~/.nvm/alias/default`, file di testo con la
/// versione, es. `v24.20.0`). Comandi atomici (`cat`: binario esterno).
/// None = nessun alias (nvm senza default o assente). Pura su CommandRunner.
pub fn wsl_nvm_default_alias(runner: &dyn CommandRunner, distro: &str, home: &str) -> Option<String> {
    let mut args = wsl_args_after(distro);
    args.extend(["cat".into(), format!("{home}/.nvm/alias/default")]);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(30)) {
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

/// Elenca i runtime Node della distro: versioni nvm (con flag default),
/// node di sistema (se ha `node` nativo), bun (include runtime JS compat).
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
    if let NativePath::Native(p) = wsl_system_node(runner, distro, &home) {
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
    match runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(30)) {
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
fn wsl_system_node(runner: &dyn CommandRunner, distro: &str, home: &str) -> NativePath {
    let path = wsl_system_path();
    let mut full: Vec<String> = wsl_args_after(distro);
    full.push("env".into());
    full.push(format!("PATH={path}"));
    full.push(format!("BUN_INSTALL={home}/.bun"));
    full.push("bash".into());
    full.push("-c".into());
    full.push("command -v node".to_string());
    let arg_refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
    match runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(30)) {
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
    let Ok((0, out, _)) = runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(30)) else {
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
    let (code, out, err) = runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(30))?;
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
    wsl_run_native_with_path(runner, distro, home, &wsl_native_path(home), bin, args, timeout)
}

/// Come `wsl_run_native` ma con PATH esplicito (es. dir nvm che contiene il
/// tool, da `wsl_tool_path`). Pura rispetto a CommandRunner.
pub fn wsl_run_native_with_path(
    runner: &dyn CommandRunner,
    distro: &str,
    home: &str,
    path: &str,
    bin: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<(i32, String, String), String> {
    let mut full: Vec<String> = wsl_args_after(distro);
    full.push("env".into());
    full.push(format!("PATH={path}"));
    full.push(format!("BUN_INSTALL={home}/.bun"));
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
        match wsl_which_in(runner, distro, &path, home, bin) {
            Ok(NativePath::Native(p)) => return Ok((NativePath::Native(p), path)),
            Ok(NativePath::Interop(p)) => return Ok((NativePath::Interop(p), path)),
            _ => {
                return Err(format!(
                    "Runtime Node selezionato ({dir}) non contiene `{bin}` nativo: il runtime non e piu disponibile? Riselezionalo dal pannello (o torna ad Automatico)."
                ));
            }
        }
    }
    if let Ok(found) = wsl_which_in(runner, distro, &base, home, bin) {
        if !matches!(found, NativePath::Missing) {
            return Ok((found, base));
        }
    }
    // nvm DECRESCENTI (la piu nuova prima): scelta automatica sensata.
    let mut dirs = wsl_nvm_bin_dirs(runner, distro, home);
    dirs.sort_by(|a, b| cmp_semver_desc(&nvm_version_of(a), &nvm_version_of(b)));
    for dir in dirs {
        let path = format!("{dir}:{base}");
        if let Ok(found) = wsl_which_in(runner, distro, &path, home, bin) {
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
    home: &str,
    bin: &str,
) -> Result<NativePath, String> {
    let mut full: Vec<String> = wsl_args_after(distro);
    full.push("env".into());
    full.push(format!("PATH={path}"));
    full.push(format!("BUN_INSTALL={home}/.bun"));
    full.push("bash".into());
    full.push("-c".into());
    full.push(format!("command -v {bin}"));
    let arg_refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
    // Timeout lungo: al primo avvio la distro fredda impiega decine di secondi.
    let (code, out, _) = runner.run_capture("wsl.exe", &arg_refs, WSL_BOOT_TIMEOUT)?;
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

/// Prefisso ambiente STORICO (non piu usato dai comandi atomici, tenuto per
/// compatibilita dei test esterni): l'isolamento ora avviene via `env`
/// (vedi `wsl_run_native`) + classificazione in Rust (`native_or_interop`).
/// DEPRECATO: non usare per nuovi comandi, non passa il lint anti-simboli.
/// Solo test (`#[cfg(test)]`): la produzione isola via `env` (vedi `wsl_run_native`).
#[cfg(test)]
pub fn wsl_env_prefix() -> String {
    r#"export PATH=$HOME/.bun/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin;export BUN_INSTALL=$HOME/.bun"#.to_string()
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
        full.push(format!("BUN_INSTALL={home}/.bun"));
        full.push(bin.to_string());
        full.extend(args.iter().map(|s| s.to_string()));
        cmds.push(full);
    };
    push("printenv", &["HOME"]);
    push("bash", &["-c", "command -v dsh"]);
    push("dsh", &["--version"]);
    push("bash", &["-c", "command -v bun"]);
    push("bash", &["-c", "command -v npm"]);
    push("tail", &["-n", "50", "/tmp/dsh-desktop-manager-3100.log"]);
    push("pkill", &["-f", "dsh.web.--port.3100"]);
    push("bun", &["add", "-g", "@deepseek-ai/dsh@1.0.0"]);
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

/// Probe WSL in comandi ATOMICI (niente shell, niente script composto).
/// Passi: HOME via printenv -> which dsh/bun/npm via `bash -c command -v`
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
        has_bun: None,
        has_npm: None,
    };
    // 1) HOME della distro (serve per il PATH nativo via `env`).
    let home = wsl_home_dir(runner, distro)?;
    // 2) dsh col RUNTIME SCELTO (se presente); bun/npm sempre in automatico
    //    (toolchain indipendenti: la dir node non deve contenerle).
    //    Runtime sparito -> Err esplicito (mai fallback silenzioso).
    //    Il PATH per `dsh --version` include la dir che lo contiene.
    let (dsh, dsh_path) = wsl_tool_path_with(runner, distro, &home, "dsh", node_runtime)?;
    let (bun, _) = wsl_tool_path(runner, distro, &home, "bun")?;
    let (npm, _) = wsl_tool_path(runner, distro, &home, "npm")?;
    probe.has_bun = Some(matches!(bun, NativePath::Native(_)));
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
                "dsh trovato solo via interop Windows ({p}): ignorato, i mondi non condividono installazioni. Installa dsh nativo nella distro (con bun o npm della distro)."
            ));
            return Ok(probe);
        }
        NativePath::Native(path) => {
            // 3) Versione dal binario NATIVO (stesso PATH che lo ha risolto).
            probe.installed = true;
            probe.executable = Some(format!("dsh nativo ({path})"));
            match wsl_run_native_with_path(runner, distro, &home, &dsh_path, "dsh", &["--version"], Duration::from_secs(15)) {
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
    let (code, out, err) = runner.run_capture("wsl.exe", &["-l", "-v"], Duration::from_secs(30))?;
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
            native_or_interop("/home/u/.bun/bin/dsh\n"),
            NativePath::Native("/home/u/.bun/bin/dsh".into())
        );
        // Solo la prima riga conta (difesa contro output rumorosi).
        assert_eq!(
            native_or_interop("/home/u/.bun/bin/dsh\n/mnt/c/x\n"),
            NativePath::Native("/home/u/.bun/bin/dsh".into())
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
    fn wsl_native_path_has_no_mnt() {
        let p = wsl_native_path("/home/u");
        assert!(p.starts_with("/home/u/.bun/bin:"));
        assert!(p.contains("/usr/local/bin"));
        assert!(!p.contains("/mnt/"));
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
        bun: Option<String>,
        npm: Option<String>,
        version: Option<(i32, String, String)>,
    }
    impl AtomProbe {
        fn native() -> Self {
            Self {
                home: Some("/home/u".into()),
                dsh: Some("/home/u/.bun/bin/dsh".into()),
                bun: Some("/home/u/.bun/bin/bun".into()),
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
                let hit = match bin { "dsh" => &self.dsh, "bun" => &self.bun, "npm" => &self.npm, _ => &None };
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
        // invisibile (dsh/bun/npm -> Missing), sonda CLOSED, probe coerente.
        // Stessi argv di SystemRunner (wsl_args_after + env + binario).
        let r = crate::proc::SystemRunner;
        let home = wsl_home_dir(&r, "Ubuntu-24.04").expect("home live");
        println!("HOME={home:?}");
        for bin in ["dsh", "bun", "npm"] {
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
        println!("PROBE installed={} version={:?} bun={:?} npm={:?} err={:?}", probe.installed, probe.version, probe.has_bun, probe.has_npm, probe.error);
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
    fn wsl_env_prefix_is_fixed_minimal() {
        // Isolamento mondi con PATH FISSO (niente filtraggio, niente quote):
        // solo ~/.bun/bin + directory di sistema Linux, mai /mnt/*.
        let p = wsl_env_prefix();
        assert!(p.contains("$HOME/.bun/bin"));
        assert!(p.contains("BUN_INSTALL"));
        assert!(p.contains("/usr/local/bin"));
        assert!(!p.contains("/mnt/"));
        assert!(!p.contains('\''));
        assert!(!p.contains('"'));
    }
    #[test]
    fn probe_interop_only_is_not_installed() {
        // dsh SOLO via interop Windows: mondi separati -> non installato,
        // con spiegazione del percorso condiviso ignorato.
        let mut r = AtomProbe::native();
        r.dsh = Some("/mnt/c/Users/x/AppData/Roaming/npm/dsh".into());
        r.npm = Some("/usr/bin/npm".into());
        r.bun = None;
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert!(probe.version.is_none());
        let err = probe.error.unwrap();
        assert!(err.contains("/mnt/c/Users/x/AppData/Roaming/npm/dsh"), "{err}");
        assert!(err.contains("interop"), "{err}");
        // npm nativo resta rilevato: toolchain indipendente da dsh.
        assert_eq!(probe.has_npm, Some(true));
        assert_eq!(probe.has_bun, Some(false));
    }
    #[test]
    fn probe_native_ok_reports_native_path() {
        let probe = probe_wsl_with(&AtomProbe::native(), "U").unwrap();
        assert!(probe.installed);
        assert_eq!(probe.version.as_deref(), Some("1.2.3"));
        assert!(probe.executable.unwrap().contains("/home/u/.bun/bin/dsh"));
        assert_eq!(probe.has_bun, Some(true));
        assert_eq!(probe.has_npm, Some(false));
    }
    #[test]
    fn probe_toolchain_interop_only_is_missing() {
        // bun/npm SOLO via interop: ignorati -> entrambi mancanti.
        let mut r = AtomProbe::native();
        r.dsh = None;
        r.bun = Some("/mnt/c/Program Files/bun/bun.exe".into());
        r.npm = Some("/mnt/c/Program Files/nodejs/npm".into());
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert_eq!(probe.has_bun, Some(false));
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
        assert_eq!(probe.has_bun, Some(true));
        assert_eq!(probe.has_npm, Some(false));
    }
    #[test]
    fn probe_no_toolchain_reports_both_missing() {
        let mut r = AtomProbe::native();
        r.dsh = None;
        r.bun = None;
        r.npm = None;
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert_eq!(probe.has_bun, Some(false));
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
        let home = PathBuf::from("/home/u");
        let pkg = "/home/u/.bun/install/global/node_modules/@deepseek-ai/dsh/package.json";
        let fs = FakeFs::with(pkg, "{\"version\": \"1.2.3\"}");
        let runner = FakeRunner::default();
        let probe = detect_windows_with(&runner, &fs, Some(home));
        assert!(probe.installed);
        assert_eq!(probe.version.as_deref(), Some("1.2.3"));
    }
    #[test]
    fn detect_windows_falls_back_to_version_flag() {
        let fs = FakeFs::with("/home/u/.bun/bin/dsh.exe", "");
        let home = PathBuf::from("/home/u");
        let exe = PathBuf::from("/home/u").join(".bun").join("bin").join("dsh.exe").to_string_lossy().into_owned();
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
