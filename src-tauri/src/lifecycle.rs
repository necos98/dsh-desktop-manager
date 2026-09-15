//! Ciclo di vita istanze (SRP): avvio/arresto/update + log/auth-url.
//! Dipende dalle porte CommandRunner/PortProber/FsAccess (DIP); gli spawn
//! reali restano dietro ProcessSpawner (test: FakeSpawner, nessun processo).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::detect::{
    probe_wsl_with_runtime, wsl_args_after, wsl_home_dir, wsl_port_check_argv, wsl_run_native,
    wsl_run_native_with_path, wsl_tool_path, wsl_tool_path_with, NativePath,
};
use crate::model::{EnvTarget, WslDiag};
use crate::proc::{find_free_port_inner, home_dir, CommandRunner, FsAccess, PortProber};
use crate::util::{extract_auth_url, stamp_compact};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(600);
const START_POLL: Duration = Duration::from_secs(60);
const AUTH_WAIT: Duration = Duration::from_secs(30);

/// Spawn di processi detached (Windows CREATE_NO_WINDOW / WSL via wsl.exe).
/// Separato da CommandRunner (che cattura l'output) per ISP.
/// Lo spawn WSL e ATOMICO: `wsl -d D --cd WS -- env PATH=..
/// setsid nohup BIN ARGS...` (argv separati, niente shell) + log catturato
/// lato Rust su file Windows (niente redirect shell nella distro).
/// `path` e il PATH risolto che contiene il binario (nvm versioni comprese),
/// dalla probe (`wsl_tool_path`).
pub trait ProcessSpawner: Send + Sync {
    fn spawn_windows_detached(&self, exe: &str, args: &[&str], workdir: &PathBuf, log_path: &PathBuf) -> Result<u32, String>;
    fn spawn_wsl_detached(&self, runner: &dyn CommandRunner, distro: &str, path: &str, ws: &str, bin: &str, args: &[&str], log_path: &PathBuf) -> Result<(), String>;
}

pub struct SystemSpawner;

impl ProcessSpawner for SystemSpawner {
    fn spawn_windows_detached(&self, exe: &str, args: &[&str], workdir: &PathBuf, log_path: &PathBuf) -> Result<u32, String> {
        let log = std::fs::File::create(log_path).map_err(|e| format!("log: {e}"))?;
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(workdir)
            .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(log));
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let child = cmd.spawn().map_err(|e| format!("spawn {exe}: {e}"))?;
        let pid = child.id();
        drop(child); // il processo continua; lo gestiamo via taskkill sul pid
        Ok(pid)
    }
    fn spawn_wsl_detached(&self, _runner: &dyn CommandRunner, distro: &str, path: &str, ws: &str, bin: &str, args: &[&str], log_path: &PathBuf) -> Result<(), String> {
        // wsl.exe --cd <ws> : la distro parte nella workdir (niente `cd`).
        // `nohup` SENZA `setsid` (verificato live): setsid stacca il dsh
        // dalla sessione e il relay wsl chiude i pipe -> log Windows vuoto
        // (0 byte), auth-url mai trovato, tab nuda che sembra bloccata.
        // Senza setsid, wsl.exe resta padre della sessione e inoltra stdout
        // al file finche dsh vive (un wsl.exe per istanza, leggero); lo spawn
        // ritorna subito via forget, all'exit il cleanup fa pkill.
        // `path` e quello risolto (contiene la dir del binario, nvm inclusa).
        let mut full: Vec<String> = vec!["-d".into(), distro.to_string(), "--cd".into(), ws.to_string(), "--".into()];
        full.push("env".into());
        full.push(format!("PATH={path}"));
        full.push("nohup".into());
        full.push(bin.to_string());
        full.extend(args.iter().map(|s| s.to_string()));
        let log = std::fs::File::create(log_path).map_err(|e| format!("log: {e}"))?;
        let mut cmd = Command::new("wsl.exe");
        cmd.args(&full)
            .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(log));
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let mut child = cmd.spawn().map_err(|e| format!("spawn wsl dsh: {e}"))?;
        // Non aspettiamo: il server gira detached (setsid). Chiudiamo gli
        // handle senza killare (il figlio e ri-genitorializzato a init).
        let _ = child.stdout.take();
        let _ = child.stderr.take();
        std::mem::forget(child);
        Ok(())
    }
}

pub fn ensure_log_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().unwrap_or_else(|| PathBuf::from(".")));
    let dir = base.join("dsh-desktop-manager").join("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn windows_log_path(port: u16) -> PathBuf {
    ensure_log_dir().join(format!("windows-{}.log", port))
}

/// Timestamp UTC corrente nel formato usato dai nomi dei log.
pub fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    stamp_compact(secs)
}

/// Nome del log di un update: `update-<kind>-<label>-<stamp>.log` (puro).
/// `label` e l'etichetta dell'ambiente (distro o "windows"): i caratteri
/// non sicuri diventano '-', cosi il nome non puo uscire dalla log dir.
pub fn update_log_file_name(kind: &str, label: &str, stamp: &str) -> String {
    format!("update-{}-{}-{}.log", safe_file_token(kind), safe_file_token(label), safe_file_token(stamp))
}

/// Token usato in un nome file: solo `[A-Za-z0-9_]`, ogni altro carattere
/// (compresi `.` e separatori di path) diventa '-', corse di '-' accorpate.
/// Cosi un nome distro ostile non puo uscire dalla log dir.
fn safe_file_token(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        let mapped = if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' };
        if mapped == '-' && out.ends_with('-') {
            continue;
        }
        out.push(mapped);
    }
    let cleaned = out.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "env".to_string()
    } else {
        cleaned
    }
}

/// Scrive il log di un update nella log dir: un file per esecuzione, cosi
/// l'output completo resta consultabile anche dopo la chiusura della UI.
/// Err = file non scritto (l'update resta comunque valido).
pub fn save_update_log(kind: &str, label: &str, output: &str) -> Result<PathBuf, String> {
    let path = ensure_log_dir().join(update_log_file_name(kind, label, &now_stamp()));
    write_log_file(&path, output).map(|()| path)
}

/// Scrittura su file del log (SRP: la scrittura resta nel modulo log).
pub fn write_log_file(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("log {}: {e}", path.display()))
}

fn read_windows_dsh_log(fs: &dyn FsAccess, port: u16) -> Option<String> {
    let path = windows_log_path(port);
    fs.read_to_string(&path).filter(|t| !t.is_empty())
}

fn read_wsl_dsh_log(_runner: &dyn CommandRunner, _distro: &str, fs: &dyn FsAccess, port: u16) -> Option<String> {
    read_wsl_dsh_log_tail(_runner, _distro, fs, port, 200)
}

/// Coda del log WSL (ultime `lines` righe via `tail`, mai l'intero file).
/// None = log assente/illeggibile (il server potrebbe non essere mai partito).
///
/// Nota robustezza: `tail` potrebbe mancare in distro minimali; in quel caso
/// si ripiega su `cat` integrale (i log di `dsh web` restano piccoli).
/// Log WSL: il file vive su WINDOWS (%APPDATA%, scritto dal thread di spawn),
/// quindi si legge via FsAccess come quello Windows (niente `tail` via shell).
/// `runner`/`distro` restano nella firma per compatibilita (non usati).
fn read_wsl_dsh_log_tail(_runner: &dyn CommandRunner, _distro: &str, fs: &dyn FsAccess, port: u16, lines: usize) -> Option<String> {
    read_windows_dsh_log_tail(fs, wsl_log_port(port), lines)
}

/// Porta del file di log WSL su Windows: stesso file per avvio e lettura.
/// (La porta DSH resta quella dell'istanza; il file e condiviso per distro.)
fn wsl_log_port(port: u16) -> u16 {
    port
}

/// Percorso del log (Windows o WSL-spawnato): unico punto di verita.
fn dsh_log_path(port: u16) -> PathBuf {
    windows_log_path(port)
}

/// Log Windows limitato alle ultime `lines` righe (mai l'intero file).
fn read_windows_dsh_log_tail(fs: &dyn FsAccess, port: u16, lines: usize) -> Option<String> {
    let text = read_windows_dsh_log(fs, port)?;
    let lines = lines.clamp(1, 500);
    let all: Vec<&str> = text.lines().collect();
    if all.len() <= lines {
        return Some(text);
    }
    Some(all[all.len() - lines..].join("\n"))
}

/// Log da esporre alla UI (comando `read_env_log`): coda limitata + nome file.
/// Mai un Err lanciato per "log assente": messaggio dedicato cosi la UI
/// spiega che il server potrebbe non essere mai partito.
pub fn read_env_log_with(
    runner: &dyn CommandRunner,
    fs: &dyn FsAccess,
    kind: &str,
    distro: Option<&str>,
    port: u16,
    lines: usize,
) -> Result<(String, String), String> {
    if kind == "windows" {
        let path = windows_log_path(port).to_string_lossy().to_string();
        return match read_windows_dsh_log_tail(fs, port, lines) {
            Some(tail) => Ok((path, tail)),
            None => Err(format!(
                "Nessun log Windows per la porta {port} ({path}): il server potrebbe non essere mai partito. Avvia l'ambiente e riprova."
            )),
        };
    }
    let distro = distro.ok_or_else(|| "distro WSL mancante".to_string())?;
    let path = dsh_log_path(port).to_string_lossy().to_string();
    match read_wsl_dsh_log_tail(runner, distro, fs, port, lines) {
        Some(tail) => Ok((path, tail)),
        None => Err(format!(
            "Nessun log per la distro \"{distro}\" porta {port} ({path}): il server potrebbe non essere mai partito. Avvia l'ambiente e riprova, oppure esegui la Diagnostica WSL."
        )),
    }
}

/// Cerca nel log l'URL autenticato stampato da `dsh web`, fino a `timeout`.
/// Non fallisce mai: None significa "URL non comparso in tempo" (modalita' degradata).
/// `sleep_ms` e iniettabile (test: no-op) per non rallentare i test.
pub fn find_auth_url_with(
    runner: &dyn CommandRunner,
    fs: &dyn FsAccess,
    kind: &str,
    distro: Option<&str>,
    port: u16,
    timeout: Duration,
    sleep_ms: impl Fn(u64),
) -> Option<String> {
    let start = std::time::Instant::now();
    loop {
        let text = if kind == "windows" {
            read_windows_dsh_log(fs, port)
        } else {
            distro.and_then(|d| read_wsl_dsh_log(runner, d, fs, port))
        };
        if let Some(t) = text {
            if let Some(url) = extract_auth_url(&t) {
                return Some(url);
            }
        }
        if start.elapsed() >= timeout {
            return None;
        }
        sleep_ms(400);
    }
}

/// Avvio Windows: (pid, porta, reached). Dietro porte: nessun Command diretto.
pub fn start_windows_with(
    prober: &dyn PortProber,
    fs: &dyn FsAccess,
    spawner: &dyn ProcessSpawner,
    detect: impl FnOnce() -> Option<String>,
    t: &EnvTarget,
    sleep_ms: impl Fn(u64),
) -> Result<(Option<u32>, u16, bool), String> {
    start_windows_with_timeout(prober, fs, spawner, detect, t, START_POLL, sleep_ms)
}

/// Variante con timeout esplicito (test: millisecondi, nessun vero sleep).
pub fn start_windows_with_timeout(
    prober: &dyn PortProber,
    fs: &dyn FsAccess,
    spawner: &dyn ProcessSpawner,
    detect: impl FnOnce() -> Option<String>,
    t: &EnvTarget,
    poll_max: Duration,
    sleep_ms: impl Fn(u64),
) -> Result<(Option<u32>, u16, bool), String> {
    let exe = detect().ok_or_else(|| {
        "dsh non è installato su Windows (nessun eseguibile trovato).".to_string()
    })?;
    let ws = t
        .workspace
        .clone()
        .filter(|w| !w.is_empty())
        .map(PathBuf::from)
        .or_else(home_dir)
        .ok_or_else(|| "impossibile determinare la cartella di lavoro".to_string())?;

    let log_path = windows_log_path(t.port);
    let mut args: Vec<String> = vec!["web".into(), "--port".into(), t.port.to_string(), "--no-open".into()];
    args.extend(t.extra_args.iter().cloned());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let pid = spawner.spawn_windows_detached(&exe, &arg_refs, &ws, &log_path)?;
    let _ = fs; // fs serve per il log/auth-url nel chiamante, non nello spawn
    Ok((Some(pid), t.port, crate::proc::poll_port(prober, t.port, poll_max, sleep_ms)))
}

/// Preflight WSL prima di start/update: verifica in UN solo giro che la distro
/// risponda, che dsh sia installato e che npm sia presente.
/// Ritorna Ok(()) oppure un messaggio gia pronto per la UI
/// (l'utente deve installare dsh/npm da solo: il manager non lo fa).
/// `node_runtime`: dir scelta dall'utente (None = automatico). Dietro
/// CommandRunner: nessun processo reale nei test.
/// Solo test: la produzione passa il runtime esplicito via `preflight_wsl_with`.
#[cfg(test)]
pub fn preflight_wsl(runner: &dyn CommandRunner, distro: &str, need_dsh: bool) -> Result<(), String> {
    preflight_wsl_with(runner, distro, need_dsh, None)
}

/// Come `preflight_wsl` con runtime Node esplicito. Pura su porte.
pub fn preflight_wsl_with(
    runner: &dyn CommandRunner,
    distro: &str,
    need_dsh: bool,
    node_runtime: Option<&str>,
) -> Result<(), String> {
    let probe = probe_wsl_with_runtime(runner, distro, node_runtime).map_err(|e| {
        format!("Distro WSL \"{distro}\" non raggiungibile ({e}). Verifica che WSL sia installato e che la distro esista (`wsl -l -v`).")
    })?;
    if let Some(err) = probe.error {
        if probe.installed {
            // dsh nativo esiste ma non si avvia/stampa versione.
            // La distro risponde: non dire "non raggiungibile" (fuorviante).
            return Err(format!("dsh nella distro WSL \"{distro}\" e rotto: {err} Installa dsh nativamente nella distro (con npm della distro), non via interop Windows."));
        }
        if err.contains("interop") || err.contains("mondi non condividono") {
            // dsh esiste SOLO come shim Windows condiviso: la distro risponde,
            // ma i mondi non condividono installazioni -> come non installato.
            return Err(format!("dsh non trovato nella distro WSL \"{distro}\" ({err})"));
        }
        return Err(format!("Distro WSL \"{distro}\" non raggiungibile ({err}). Verifica che WSL sia installato e che la distro esista (`wsl -l -v`)."));
    }
    if need_dsh && !probe.installed {
        let toolchain_hint = if probe.has_npm == Some(false) {
            " Nella distro manca npm: installa Node/npm (es. `sudo apt install nodejs npm`), poi dsh."
        } else {
            " Installa dsh nella distro con il pulsante Installa (richiede npm nella distro)."
        };
        return Err(format!("dsh non trovato nella distro WSL \"{distro}\".{toolchain_hint}"));
    }
    Ok(())
}

/// Diagnostica WSL passo-passo per la UI (mai un Err lanciato: l'eventuale
/// fallimento fatale finisce in `WslDiag.error`). Ordine dei passi: elenco
/// distro -> probe dsh/toolchain -> porta nella distro -> porta da Windows ->
/// coda del log. Ogni passo fallito lascia i successivi a None ma non
/// interrompe la diagnosi.
/// Solo test: la produzione passa il runtime esplicito via `diagnose_wsl_with_runtime`.
#[cfg(test)]
pub fn diagnose_wsl_with(
    runner: &dyn CommandRunner,
    fs: &dyn FsAccess,
    prober: &dyn PortProber,
    distro: &str,
    port: u16,
    log_lines: usize,
) -> WslDiag {
    diagnose_wsl_with_runtime(runner, fs, prober, distro, port, log_lines, None)
}

/// Come `diagnose_wsl_with` con runtime Node esplicito (coerenza con
/// probe/start/update: la checklist descrive lo stesso mondo). Pura su porte.
pub fn diagnose_wsl_with_runtime(
    runner: &dyn CommandRunner,
    fs: &dyn FsAccess,
    prober: &dyn PortProber,
    distro: &str,
    port: u16,
    log_lines: usize,
    node_runtime: Option<&str>,
) -> WslDiag {
    let mut diag = WslDiag {
        distro: distro.to_string(),
        state: None,
        dsh_installed: false,
        dsh_version: None,
        has_npm: false,
        port_open_in_distro: None,
        port_open_from_windows: prober.is_open(port),
        log_tail: None,
        error: None,
    };
    // 1) la distro esiste? (stato da wsl -l -v)
    match runner.run_capture("wsl.exe", &["-l", "-v"], Duration::from_secs(30)) {
        Ok((code, out, _)) if code == 0 => {
            diag.state = crate::util::parse_wsl_distros(&out)
                .into_iter()
                .find(|d| d.name == distro)
                .map(|d| d.state);
            if diag.state.is_none() {
                diag.error = Some(format!("Distro \"{distro}\" non trovata in `wsl -l -v`."));
            }
        }
        Ok((_, out, err)) => {
            let detail = if !err.trim().is_empty() { err.trim() } else { out.trim() };
            diag.error = Some(if detail.is_empty() {
                "Elenco distro WSL fallito.".to_string()
            } else {
                format!("Elenco distro WSL fallito: {detail}")
            });
        }
        Err(e) => {
            diag.error = Some(format!("Elenco distro WSL fallito: {e}"));
        }
    }
    // 2) dsh + toolchain NATIVI (comandi atomici, `case /mnt/*` in Rust).
    //    Stesso runtime scelto della probe: la checklist descrive lo stesso
    //    mondo di start/update (niente lotteria tra versioni).
    match probe_wsl_with_runtime(runner, distro, node_runtime) {
        Ok(p) => {
            diag.dsh_installed = p.installed;
            diag.dsh_version = p.version;
            diag.has_npm = p.has_npm.unwrap_or(false);
            if diag.error.is_none() {
                diag.error = p.error;
            }
        }
        Err(e) => {
            if diag.error.is_none() {
                diag.error = Some(format!("Probe nella distro fallito: {e}"));
            }
        }
    }
    // 3) la porta risponde vista da DENTRO la distro? (distingue server giu
    //    da server su-ma-irraggiungibile-da-Windows, tipico di WSL2 NAT)
    //    Sonda atomica `bash -c` senza `$`/virgolette (verificata live).
    diag.port_open_in_distro = wsl_port_open(runner, distro, port);
    // 4) coda del log (su file Windows, scritto dal thread di spawn)
    diag.log_tail = read_wsl_dsh_log_tail(runner, distro, fs, port, log_lines);
    diag
}

/// Avvio WSL: (None, porta, reached). Comandi atomici, niente shell.
/// `fs` serve solo nei test (FakeFs isolato); in produzione SystemRunner.
pub fn start_wsl_with(
    runner: &dyn CommandRunner,
    prober: &dyn PortProber,
    _fs: &dyn FsAccess,
    spawner: &dyn ProcessSpawner,
    t: &EnvTarget,
    sleep_ms: impl Fn(u64),
) -> Result<(Option<u32>, u16, bool), String> {
    start_wsl_with_timeout(runner, prober, _fs, spawner, t, START_POLL, sleep_ms)
}

/// Variante con timeout esplicito (test: millisecondi, nessun vero sleep).
pub fn start_wsl_with_timeout(
    runner: &dyn CommandRunner,
    prober: &dyn PortProber,
    _fs: &dyn FsAccess,
    spawner: &dyn ProcessSpawner,
    t: &EnvTarget,
    poll_max: Duration,
    sleep_ms: impl Fn(u64),
) -> Result<(Option<u32>, u16, bool), String> {
    let distro = t.distro.clone().ok_or_else(|| "distro WSL mancante".to_string())?;
    let rt = t.node_runtime.as_deref();
    // Fail-fast PRIMA dello spawn: se la distro non risponde o dsh (nativo)
    // manca, l'errore spiega subito cosa installare invece di un timeout da 60s.
    // Stesso runtime scelto in preflight, guardia e spawn: un solo dsh.
    preflight_wsl_with(runner, &distro, true, rt)?;
    let home = wsl_home_dir(runner, &distro)?;
    // PATH che contiene dsh (runtime scelto per primo, poi nvm decrescenti).
    // Percorso ASSOLUTO nello spawn: probe e spawn usano lo stesso file.
    let tool_res = wsl_tool_path_with(runner, &distro, &home, "dsh", rt)?;
    let (dsh_native, dsh_path): (NativePath, String) = tool_res;
    let dsh_path = match &dsh_native {
        NativePath::Native(_) => dsh_path,
        NativePath::Interop(p) => {
            return Err(format!(
                "dsh trovato solo via interop Windows ({p}): ignorato, i mondi non condividono installazioni. Installa dsh nativo nella distro."
            ));
        }
        NativePath::Missing => {
            return Err(format!(
                "dsh nativo assente nella distro WSL \"{distro}\": installalo nella distro con il pulsante Installa (richiede npm nativi della distro)."
            ));
        }
    };
    let dsh_bin = match &dsh_native {
        NativePath::Native(p) => p.clone(),
        _ => dsh_path.clone(),
    };
    // Workdir: solo percorsi Linux assoluti; tutto il resto -> home.
    // (Niente `cd` shell: la workdir passa via `wsl --cd`, niente quote.)
    let ws = t.workspace.clone().filter(|w| w.starts_with('/')).unwrap_or_else(|| home.clone());
    for a in t.extra_args.iter() {
        // Argomenti extra sicuri: niente simboli shell (verificato live che
        // `;`, `$`, quote e redirect rompono o iniettano via `bash -lc`).
        // Chi usa --host 0.0.0.0 resta valido; il resto viene rifiutato.
        if a.chars().any(|c| matches!(c, '$' | ';' | '\'' | '"' | '`' | '>' | '<' | '|' | '&' | '(' | ')' | '\\' | '{' | '}')) {
            return Err(format!(
                "Argomento extra non sicuro per WSL (simboli shell non ammessi): {a}. Usa solo flag semplici come --host 0.0.0.0."
            ));
        }
    }
    let mut spawn_args: Vec<String> = vec!["web".into(), "--port".into(), t.port.to_string(), "--no-open".into()];
    spawn_args.extend(t.extra_args.iter().cloned());
    let spawn_refs: Vec<&str> = spawn_args.iter().map(|s| s.as_str()).collect();
    let log_path = dsh_log_path(t.port);
    // PERCORSO ASSOLUTO (fix probe-vs-start): non `dsh` per nome (lotteria
    // PATH tra versioni), ma il file esatto classificato nativo dalla probe.
    spawner.spawn_wsl_detached(runner, &distro, &dsh_path, &ws, &dsh_bin, &spawn_refs, &log_path).map_err(|e| {
        format!("Avvio nella distro \"{distro}\" fallito ({e}). Apri la Diagnostica WSL dal pannello per il dettaglio passo-passo.")
    })?;
    let reached = crate::proc::poll_port(prober, t.port, poll_max, sleep_ms);
    if !reached {
        // Timeout con contesto: la UI mostra questo messaggio + log + diagnostica.
        // Suggerimento rete WSL2 quando la porta risponde dentro la distro
        // ma non da Windows (NAT di default: serve --host 0.0.0.0 o mirror).
        let in_distro = wsl_port_open(runner, &distro, t.port);
        let net_hint = if in_distro == Some(true) {
            " Il server risponde DENTRO la distro ma non da Windows: tipico WSL2 in modalita NAT — aggiungi `--host 0.0.0.0` negli argomenti extra e riavvia."
        } else {
            " Controlla il log dell'ambiente (pulsante Mostra log) o esegui la Diagnostica WSL."
        };
        return Err(format!(
            "Timeout: il server dsh nella distro \"{distro}\" non risponde sulla porta {} entro {}s.{net_hint}",
            t.port,
            poll_max.as_secs()
        ));
    }
    Ok((None, t.port, true))
}

/// Sonda porta vista da DENTRO la distro (Some) oppure sonda fallita (None).
/// Script `bash -c` sensoriale senza `$`/virgolette/`$(…)` (verificato live).
/// PATH irrilevante (/dev/tcp e `echo` sono builtin): niente `env`.
fn wsl_port_open(runner: &dyn CommandRunner, distro: &str, port: u16) -> Option<bool> {
    let mut args = wsl_args_after(distro);
    for a in wsl_port_check_argv(port) {
        args.push(a);
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(30)) {
        Ok((_, out, _)) if out.contains("OPEN") => Some(true),
        Ok((_, out, _)) if out.contains("CLOSED") => Some(false),
        _ => None,
    }
}

/// Arresto WSL via pkill nella distro (comando atomico, niente shell).
/// Il pattern usa `.` al posto degli spazi (`pkill -f` = ERE: il punto
/// matcha lo spazio) perche i pattern con spazi richiederebbero virgolette
/// (vietate: strip-pate via argv). `dsh.web.--port.3100` matcha la riga
/// `dsh web --port 3100` e solo quella (niente falsi positivi pratici).
pub fn stop_wsl_with(runner: &dyn CommandRunner, t: &EnvTarget) -> Result<String, String> {
    let distro = t.distro.clone().ok_or_else(|| "distro WSL mancante".to_string())?;
    let home = wsl_home_dir(runner, &distro)?;
    let pattern = format!("dsh.web.--port.{}", t.port);
    let (code, _out, err) = wsl_run_native(runner, &distro, &home, "pkill", &["-f", &pattern], Duration::from_secs(30))?;
    if code != 0 && !err.trim().is_empty() {
        return Err(err.trim().to_string());
    }
    Ok(format!("Comando di arresto inviato nella distro {distro} (porta {}).", t.port))
}

/// Installazione/aggiornamento via npm: su Windows `npm install -g pkg`
/// (npm dal PATH), nella distro WSL il npm NATIVO della distro. Vale per
/// QUALSIASI direzione (upgrade, downgrade, reinstall): il package manager
/// sovrascrive la versione pinnata. Ritorna (exit code, stdout+stderr).
pub fn run_update_with(runner: &dyn CommandRunner, t: &EnvTarget, version: &str) -> (i32, String) {
    let pkg = format!("@deepseek-ai/dsh@{version}");
    if t.kind == "windows" {
        // Unico installer: npm. L'errore riportato e quello di npm (il piu
        // pertinente per l'utente: il manager non tenta altre toolchain).
        let args = ["install", "-g", pkg.as_str()];
        return match runner.run_capture("npm", &args, UPDATE_TIMEOUT) {
            Ok((code, out, err)) => (code, format!("{out}\n{err}")),
            Err(e) => (-1, e),
        };
    } else {
        match t.distro.clone() {
            Some(d) => {
                // Fail-fast: senza npm l'installazione fallirebbe con un
                // errore oscuro dopo minuti — meglio dirlo subito. Il manager
                // NON installa mai npm: deve farlo l'utente nella distro.
                // Stesso runtime scelto della probe (mai fallback silenzioso).
                let rt = t.node_runtime.as_deref();
                match probe_wsl_with_runtime(runner, &d, rt) {
                    Ok(p) if p.error.is_some() && !p.installed => {
                        return (-1, format!("Distro WSL \"{d}\" non raggiungibile ({}). Verifica che WSL sia installato e che la distro esista (`wsl -l -v`).", p.error.unwrap_or_default()));
                    }
                    Ok(p) if p.error.is_some() => {
                        return (-1, format!("dsh nella distro WSL \"{d}\" e rotto: {} Installa dsh nativamente nella distro (con npm della distro), non via interop Windows.", p.error.unwrap_or_default()));
                    }
                    Ok(p) if p.has_npm != Some(true) => {
                        return (-1, format!("Nella distro WSL \"{d}\" manca npm: installa prima Node/npm (es. `sudo apt install nodejs npm`), poi riprova. Il manager non installa toolchain da solo."));
                    }
                    Err(e) => {
                        return (-1, format!("Distro WSL \"{d}\" non raggiungibile ({e}). Verifica che WSL sia installato e che la distro esista (`wsl -l -v`)."));
                    }
                    _ => {}
                }
                // Installazione col toolchain NATIVO in automatico (il runtime
                // scelto vale per dsh, non per l'installer: npm e una toolchain
                // indipendente). PATH nativo via `env`.
                let home = match wsl_home_dir(runner, &d) {
                    Ok(h) => h,
                    Err(e) => return (-1, e),
                };
                // Il PATH per eseguire include la dir che contiene il tool
                // (nvm decrescenti comprese): funziona cosi com'e.
                let try_install = |tool: &str, args: &[&str]| -> Option<(i32, String)> {
                    match wsl_tool_path(runner, &d, &home, tool) {
                        Ok((NativePath::Native(_), path)) => match wsl_run_native_with_path(runner, &d, &path, tool, args, UPDATE_TIMEOUT) {
                            Ok((code, out, err)) => Some((code, format!("{out}\n{err}"))),
                            Err(e) => Some((-1, e)),
                        },
                        _ => None,
                    }
                };
                if let Some((code, out)) = try_install("npm", &["install", "-g", &pkg]) {
                    return (code, out);
                }
                (3, format!("Nella distro WSL \"{d}\" manca npm nativo: installa prima Node/npm, poi riprova. Il manager non installa toolchain da solo."))
            }
            None => (-1, "distro WSL mancante".to_string()),
        }
    }
}

/// Prima porta libera (wrapper testabile: porta PortProber).
pub fn find_free_port_with(prober: &dyn PortProber, from: u16) -> u16 {
    find_free_port_inner(prober, from)
}

/// Auth-URL dal log con timeout di produzione (sleep reale).
pub fn find_auth_url_sync(runner: &dyn CommandRunner, fs: &dyn FsAccess, kind: &str, distro: Option<&str>, port: u16) -> Option<String> {
    find_auth_url_with(runner, fs, kind, distro, port, AUTH_WAIT, |ms| std::thread::sleep(Duration::from_millis(ms)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EnvTarget;
    use crate::proc::test_support::*;
    use std::path::PathBuf;
    use std::time::Duration;

    struct FakeSpawner { pid: u32, fail: Option<String>, last_spawn: std::sync::Mutex<Option<(String, String, String, Vec<String>)>> }
    impl FakeSpawner {
        fn ok(pid: u32) -> Self { Self { pid, fail: None, last_spawn: std::sync::Mutex::new(None) } }
    }
    impl ProcessSpawner for FakeSpawner {
        fn spawn_windows_detached(&self, _exe: &str, _args: &[&str], _workdir: &PathBuf, _log: &PathBuf) -> Result<u32, String> {
            match &self.fail { Some(e) => Err(e.clone()), None => Ok(self.pid) }
        }
        fn spawn_wsl_detached(&self, _runner: &dyn CommandRunner, distro: &str, path: &str, ws: &str, bin: &str, args: &[&str], _log: &PathBuf) -> Result<(), String> {
            *self.last_spawn.lock().unwrap() = Some((distro.to_string(), path.to_string(), bin.to_string(), args.iter().map(|s| s.to_string()).collect()));
            let _ = ws;
            match &self.fail { Some(e) => Err(e.clone()), None => Ok(()) }
        }
    }
    /// Runner atomico programmabile: risponde a printenv/which/version/tail/
    /// pkill/porta per argv (niente script composti). Default: distro con
    /// dsh+npm nativi, versione 1.0.0, porta chiusa, log vuoto.
    #[derive(Default)]
    struct AtomRunner {
        home: Option<String>,
        dsh: Option<String>,
        npm: Option<String>,
        version_out: Option<(i32, String, String)>,
        port_open: bool,
        npm_install: Option<(i32, String)>,
        list: Option<String>,
    }
    impl AtomRunner {
        fn native() -> Self {
            Self {
                home: Some("/home/u".into()),
                dsh: Some("/home/u/.local/bin/dsh".into()),
                npm: Some("/usr/bin/npm".into()),
                version_out: Some((0, "dsh version 1.0.0".into(), String::new())),
                port_open: false,
                npm_install: Some((0, "installed".into())),
                list: Some("NAME S V\nU Running 2\n".into()),
            }
        }
        fn no_dsh() -> Self {
            Self { home: Some("/home/u".into()), dsh: None, npm: Some("/usr/bin/npm".into()),
                version_out: None, port_open: false, npm_install: None,
                list: Some("NAME S V\nU Running 2\n".into()) }
        }
        fn interop_dsh() -> Self {
            Self { home: Some("/home/u".into()), dsh: Some("/mnt/c/x/dsh".into()), npm: Some("/usr/bin/npm".into()),
                version_out: None, port_open: false, npm_install: None,
                list: Some("NAME S V\nU Running 2\n".into()) }
        }
    }
    impl CommandRunner for AtomRunner {
        fn run_capture(&self, prog: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
            // wsl -l -v (stato distro)
            if a == ["-l", "-v"] {
                let out = self.list.clone().unwrap_or_default();
                return Ok((0, out, String::new()));
            }
            // printenv HOME
            if a.last() == Some(&"HOME") && a.contains(&"printenv") {
                return match &self.home {
                    Some(h) => Ok((0, format!("{h}\n"), String::new())),
                    None => Ok((1, String::new(), "no home".into())),
                };
            }
            // bash -c "command -v BIN" (PATH gia nativo via env)
            // + sonda /dev/tcp (script con /dev/tcp, niente command -v).
            if a.contains(&"bash") && a.contains(&"-c") {
                if a.iter().any(|x| x.contains("/dev/tcp")) {
                    return Ok((0, if self.port_open { "OPEN".into() } else { "CLOSED".into() }, String::new()));
                }
                let cmd = a.iter().find(|x| x.starts_with("command -v")).cloned().unwrap_or_default();
                let bin = cmd.split_whitespace().last().unwrap_or("");
                // Sonda porta: script con /dev/tcp (unico bash -c non-which).
                if cmd.contains("/dev/tcp") {
                    return Ok((0, if self.port_open { "OPEN".into() } else { "CLOSED".into() }, String::new()));
                }
                let hit = match bin { "dsh" => &self.dsh, "npm" => &self.npm, _ => &None };
                return match hit {
                    Some(p) => Ok((0, format!("{p}\n"), String::new())),
                    None => Ok((1, String::new(), String::new())),
                };
            }
            // dsh --version diretto
            if a.contains(&"dsh") && a.contains(&"--version") {
                return match &self.version_out {
                    Some((c, o, e)) => Ok((*c, o.clone(), e.clone())),
                    None => Ok((1, String::new(), "no version".into())),
                };
            }
            // npm install -g
            if a.contains(&"npm") && a.contains(&"install") {
                return match &self.npm_install {
                    Some((c, o)) => Ok((*c, o.clone(), String::new())),
                    None => Ok((1, String::new(), "npm assente".into())),
                };
            }
            // tail / pkill
            if a.contains(&"tail") || a.contains(&"pkill") {
                return Ok((0, String::new(), String::new()));
            }
            let _ = prog;
            Err(format!("comando atomico non stubbato: {a:?}"))
        }
    }
    fn target(kind: &str, distro: Option<&str>, port: u16) -> EnvTarget {
        EnvTarget { kind: kind.into(), name: kind.into(), distro: distro.map(|s| s.into()), port, extra_args: vec![], workspace: None, node_runtime: None }
    }
    #[test]
    fn start_windows_returns_pid_when_port_pending() {
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(4242);
        let t = target("windows", None, 3080);
        let (pid, port, reached) = start_windows_with_timeout(&prober, &fs, &sp, || Some("C:/dsh.exe".into()), &t, Duration::from_millis(1), |_| {}).unwrap();
        assert_eq!(pid, Some(4242));
        assert_eq!(port, 3080);
        assert!(!reached);
    }
    #[test]
    fn start_windows_errors_without_exe() {
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(1);
        let t = target("windows", None, 3080);
        assert!(start_windows_with_timeout(&prober, &fs, &sp, || None, &t, Duration::from_millis(1), |_| {}).is_err());
    }
    #[test]
    fn start_windows_reached_when_port_open() {
        let prober = FakeProber { open: vec![3080] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(7);
        let t = target("windows", None, 3080);
        let (_, _, reached) = start_windows_with_timeout(&prober, &fs, &sp, || Some("e".into()), &t, Duration::from_millis(1), |_| {}).unwrap();
        assert!(reached);
    }
    #[test]
    fn start_wsl_spawns_atomically() {
        // Comandi atomici + PERCORSO ASSOLUTO: probe e spawn usano lo stesso
        // file (niente lotteria PATH tra versioni). Niente simboli shell.
        let runner = AtomRunner::native();
        let prober = FakeProber { open: vec![3100] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let t = target("wsl", Some("Ubuntu"), 3100);
        let (pid, port, reached) = start_wsl_with_timeout(&runner, &prober, &fs, &sp, &t, Duration::from_millis(1), |_| {}).unwrap();
        assert_eq!(pid, None);
        assert_eq!(port, 3100);
        assert!(reached);
        let (distro, path_home, bin, args) = sp.last_spawn.lock().unwrap().clone().unwrap();
        assert_eq!(distro, "Ubuntu");
        assert!(path_home.contains("/home/u"), "{path_home}");
        assert_eq!(bin, "/home/u/.local/bin/dsh", "spawn per percorso assoluto, non per nome");
        assert!(args.contains(&"web".to_string()));
        assert!(args.contains(&"3100".to_string()));
        for a in &args {
            assert!(!a.contains('$') && !a.contains(';') && !a.contains('"') && !a.contains('\''), "simbolo shell in argv: {a}");
        }
    }
    #[test]
    fn start_wsl_refuses_interop_without_spawn() {
        // Solo shim /mnt/*: nessun processo lanciato (mondi separati).
        let runner = AtomRunner::interop_dsh();
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let e = start_wsl_with_timeout(&runner, &prober, &fs, &sp, &target("wsl", Some("U"), 3100), Duration::from_millis(1), |_| {}).unwrap_err();
        assert!(e.contains("/mnt/c/x/dsh"), "{e}");
        assert!(sp.last_spawn.lock().unwrap().is_none());
    }
    #[test]
    fn start_wsl_timeout_is_error_with_log_hint() {
        // Probe ok ma porta mai aperta -> Err con hint.
        let runner = AtomRunner::native();
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let e = start_wsl_with_timeout(&runner, &prober, &fs, &sp, &target("wsl", Some("Ubuntu"), 3100), Duration::from_millis(1), |_| {}).unwrap_err();
        assert!(e.contains("Timeout"), "{e}");
        assert!(e.contains("Mostra log") || e.contains("Diagnostica"), "{e}");
    }
    #[test]
    fn start_wsl_rejects_unsafe_extra_args() {
        // Argomenti extra con simboli shell: rifiutati prima dello spawn.
        let runner = AtomRunner::native();
        let prober = FakeProber { open: vec![3100] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let mut t = target("wsl", Some("U"), 3100);
        t.extra_args = vec!["--host".into(), "0.0.0.0;rm -rf ~".into()];
        let e = start_wsl_with_timeout(&runner, &prober, &fs, &sp, &t, Duration::from_millis(1), |_| {}).unwrap_err();
        assert!(e.contains("non sicuro"), "{e}");
        assert!(sp.last_spawn.lock().unwrap().is_none());
    }
    #[test]
    fn start_wsl_requires_distro() {
        let runner = AtomRunner::native();
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        assert!(start_wsl_with_timeout(&runner, &prober, &fs, &sp, &target("wsl", None, 3100), Duration::from_millis(1), |_| {}).is_err());
    }
    #[test]
    fn preflight_ok_when_dsh_and_npm_present() {
        assert!(preflight_wsl(&AtomRunner::native(), "U", true).is_ok());
    }
    #[test]
    fn preflight_fails_fast_when_dsh_missing() {
        let e = preflight_wsl(&AtomRunner::no_dsh(), "U", true).unwrap_err();
        assert!(e.contains("dsh non trovato"), "{e}");
    }
    #[test]
    fn preflight_hints_npm_when_missing() {
        let mut r = AtomRunner::no_dsh();
        r.npm = None;
        let e = preflight_wsl(&r, "U", true).unwrap_err();
        assert!(e.contains("dsh non trovato"), "{e}");
        assert!(e.contains("npm"), "{e}");
    }
    #[test]
    fn preflight_broken_wrapper_names_dsh_not_distro() {
        // dsh NATIVO esiste ma --version fallisce: messaggio dedicato, NON
        // "distro non raggiungibile" (la distro risponde).
        let mut r = AtomRunner::native();
        r.version_out = Some((1, String::new(), "permesso negato".into()));
        let e = preflight_wsl(&r, "U", true).unwrap_err();
        assert!(e.contains("rotto"), "{e}");
        assert!(!e.contains("non raggiungibile"), "{e}");
    }
    #[test]
    fn preflight_interop_only_is_not_installed() {
        // dsh SOLO via interop: mondi separati -> "non trovato" + hint nativo.
        let e = preflight_wsl(&AtomRunner::interop_dsh(), "U", true).unwrap_err();
        assert!(e.contains("non trovato"), "{e}");
        assert!(e.contains("/mnt/c/x/dsh"), "{e}");
    }
    #[test]
    fn preflight_runner_error_names_distro() {
        let r = FakeRunner::with_error("wsl.exe", &["-d"], "timeout");
        let e = preflight_wsl(&r, "U", true).unwrap_err();
        assert!(e.contains("\"U\""), "{e}");
    }
    #[test]
    fn start_wsl_uses_chosen_runtime() {
        // Runtime scelto (v24): probe, guardia e spawn usano quella dir —
        // anche se esistesse un dsh altrove, vince la scelta utente.
        struct TwoVersions;
        impl CommandRunner for TwoVersions {
            fn run_capture(&self, _p: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                if a.contains(&"printenv") {
                    return Ok((0, "/home/u\n".into(), String::new()));
                }
                if a.contains(&"ls") {
                    return Ok((0, "v20.20.1\nv24.20.0\n".into(), String::new()));
                }
                if a.contains(&"bash") {
                    let has24 = a.iter().any(|x| x.contains("v24.20.0"));
                    if a.iter().any(|x| x.contains("command -v")) {
                        if has24 {
                            return Ok((0, "/home/u/.nvm/versions/node/v24.20.0/bin/dsh\n".into(), String::new()));
                        }
                        return Ok((1, String::new(), String::new()));
                    }
                }
                if a.contains(&"dsh") {
                    return Ok((0, "dsh version 1.0.0".into(), String::new()));
                }
                if a.contains(&"tail") || a.contains(&"pkill") {
                    return Ok((0, String::new(), String::new()));
                }
                if a == ["-l", "-v"] {
                    return Ok((0, "NAME S V\nU Running 2\n".into(), String::new()));
                }
                Err(format!("non stubbato: {a:?}"))
            }
        }
        let prober = FakeProber { open: vec![3100] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let mut t = target("wsl", Some("U"), 3100);
        t.node_runtime = Some("/home/u/.nvm/versions/node/v24.20.0/bin".into());
        let (_, _, reached) = start_wsl_with_timeout(&TwoVersions, &prober, &fs, &sp, &t, Duration::from_millis(1), |_| {}).unwrap();
        assert!(reached);
        let (_, _, bin, _) = sp.last_spawn.lock().unwrap().clone().unwrap();
        assert!(bin.contains("v24.20.0"), "{bin}");
    }
    #[test]
    fn start_wsl_missing_chosen_runtime_is_explicit_error() {
        // Scelta che non contiene dsh: errore esplicito, MAI fallback
        // silenzioso a un'altra versione (niente lotteria).
        struct Gone;
        impl CommandRunner for Gone {
            fn run_capture(&self, _p: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                if a.contains(&"printenv") {
                    return Ok((0, "/home/u\n".into(), String::new()));
                }
                if a.contains(&"bash") {
                    return Ok((1, String::new(), String::new()));
                }
                Err(format!("non stubbato: {a:?}"))
            }
        }
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let mut t = target("wsl", Some("U"), 3100);
        t.node_runtime = Some("/home/u/.nvm/versions/node/v99.99.99/bin".into());
        let e = start_wsl_with_timeout(&Gone, &prober, &fs, &sp, &t, Duration::from_millis(1), |_| {}).unwrap_err();
        assert!(e.contains("v99.99.99") || e.contains("non") , "{e}");
        assert!(sp.last_spawn.lock().unwrap().is_none());
    }
    #[test]
    fn start_wsl_fails_fast_without_dsh() {
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let e = start_wsl_with_timeout(&AtomRunner::no_dsh(), &prober, &fs, &sp, &target("wsl", Some("U"), 3100), Duration::from_millis(1), |_| {}).unwrap_err();
        assert!(e.contains("dsh non trovato") || e.contains("nativo assente"), "{e}");
        assert!(sp.last_spawn.lock().unwrap().is_none());
    }
    #[test]
    fn start_wsl_timeout_hints_nat_when_open_in_distro() {
        let mut r = AtomRunner::native();
        r.port_open = true;
        let prober = FakeProber { open: vec![] };
        let fs = FakeFs::default();
        let sp = FakeSpawner::ok(0);
        let e = start_wsl_with_timeout(&r, &prober, &fs, &sp, &target("wsl", Some("U"), 3100), Duration::from_millis(1), |_| {}).unwrap_err();
        assert!(e.contains("--host 0.0.0.0"), "{e}");
    }
    #[test]
    fn update_wsl_refuses_interop_toolchain() {
        // npm SOLO via interop: come assente (mondi separati).
        let mut r = AtomRunner::no_dsh();
        r.npm = Some("/mnt/c/npm".into());
        let (code, msg) = run_update_with(&r, &target("wsl", Some("U"), 3100), "1.0.0");
        assert_eq!(code, -1);
        assert!(msg.contains("npm"), "{msg}");
    }
    #[test]
    fn update_wsl_uses_npm_natively() {
        let (code, out) = run_update_with(&AtomRunner::native(), &target("wsl", Some("U"), 3100), "1.0.0");
        assert_eq!(code, 0);
        assert!(out.contains("installed"), "{out}");
    }
    #[test]
    fn diagnose_reports_interop_without_throwing() {
        let d = diagnose_wsl_with(&AtomRunner::interop_dsh(), &FakeFs::default(), &FakeProber { open: vec![] }, "U", 3100, 10);
        assert!(!d.dsh_installed);
        assert!(d.error.unwrap_or_default().contains("/mnt/c/x/dsh"));
    }
    #[test]
    fn update_wsl_refuses_without_npm() {
        let mut r = AtomRunner::no_dsh();
        r.npm = None;
        let (code, out) = run_update_with(&r, &target("wsl", Some("U"), 3100), "1.0.0");
        assert_eq!(code, -1);
        assert!(out.contains("npm"), "{out}");
    }
    #[test]
    fn read_env_log_wsl_missing_reports_diagnostics_hint() {
        let r = FakeRunner::default();
        let e = read_env_log_with(&r, &FakeFs::default(), "wsl", Some("U"), 3100, 50).unwrap_err();
        assert!(e.contains("Diagnostica WSL"), "{e}");
    }
    #[test]
    fn diagnose_reports_missing_distro_without_throwing() {
        // Stato-distro e probe sono indipendenti: la checklist mostra
        // l'errore sullo stato ma la probe riesce comunque (dsh nativo).
        let mut r = AtomRunner::native();
        r.list = Some("NAME S V\nOther Running 2\n".into());
        let d = diagnose_wsl_with(&r, &FakeFs::default(), &FakeProber { open: vec![] }, "U", 3100, 10);
        assert!(d.error.unwrap_or_default().contains("non trovata"));
        assert!(d.dsh_installed);
    }
    #[test]
    fn stop_wsl_ok_message() {
        let t = target("wsl", Some("Debian"), 3101);
        let msg = stop_wsl_with(&AtomRunner::native(), &t).unwrap();
        assert!(msg.contains("Debian"));
        assert!(msg.contains("3101"));
    }
    #[test]
    fn stop_wsl_atomic_argv_has_no_shell_symbols() {
        // Il pkill viaggia come argv atomici (pattern con punti, niente spazi).
        struct Capture;
        impl CommandRunner for Capture {
            fn run_capture(&self, _p: &str, a: &[&str], _t: Duration) -> Result<(i32, String, String), String> {
                if a.contains(&"printenv") {
                    return Ok((0, "/home/u\n".into(), String::new()));
                }
                for x in a {
                    assert!(!x.contains('$') && !x.contains('\'') && !x.contains('"') && !x.contains('|') && !x.contains('&'), "simbolo in argv stop: {x}");
                }
                assert!(a.contains(&"pkill"));
                assert!(a.iter().any(|x| x.contains("dsh.web.--port.3101")));
                Ok((0, String::new(), String::new()))
            }
        }
        let msg = stop_wsl_with(&Capture, &target("wsl", Some("Debian"), 3101)).unwrap();
        assert!(msg.contains("3101"));
    }
    #[test]
    fn stop_wsl_requires_distro() {
        let any = AcceptAll;
        assert!(stop_wsl_with(&any, &target("wsl", None, 3100)).is_err());
    }
    #[test]
    fn update_windows_uses_npm() {
        let r = FakeRunner::default();
        r.outputs.lock().unwrap().insert("npm install -g @deepseek-ai/dsh@2.0.0".into(), (0, "ok".into(), String::new()));
        let (code, out) = run_update_with(&r, &target("windows", None, 3080), "2.0.0");
        assert_eq!(code, 0);
        assert!(out.contains("ok"));
    }
    #[test]
    fn update_windows_surfaces_npm_error() {
        // npm fallisce: l'errore esce com'e (nessun altro installer tentato).
        let r = FakeRunner::default();
        r.outputs.lock().unwrap().insert("npm install -g @deepseek-ai/dsh@1.0.0".into(), (1, String::new(), "err-npm".into()));
        let (code, out) = run_update_with(&r, &target("windows", None, 3080), "1.0.0");
        assert_eq!(code, 1);
        assert!(out.contains("err-npm"), "{out}");
    }
    #[test]
    fn update_windows_surfaces_missing_npm_as_error() {
        // npm assente (Err di spawn): errore esplicito, nessun fallback.
        let r = FakeRunner::default();
        r.errors.lock().unwrap().insert("npm install -g @deepseek-ai/dsh@0.0.1-rc.1".into(), "npm non trovato".into());
        let (code, out) = run_update_with(&r, &target("windows", None, 3080), "0.0.1-rc.1");
        assert_eq!(code, -1);
        assert!(out.contains("npm non trovato"), "{out}");
    }
    #[test]
    fn update_wsl_reports_missing_distro() {
        let r = FakeRunner::default();
        let (code, _) = run_update_with(&r, &target("wsl", None, 3100), "1.0.0");
        assert_eq!(code, -1);
    }
    #[test]
    fn update_log_name_is_stable_and_safe() {
        assert_eq!(
            update_log_file_name("windows", "windows", "20260102-030405"),
            "update-windows-windows-20260102-030405.log"
        );
        assert_eq!(
            update_log_file_name("wsl", "Ubuntu-22.04", "20260102-030405"),
            "update-wsl-Ubuntu-22-04-20260102-030405.log"
        );
        // Nome distro ostile: nessun separatore, quindi nessuna uscita dalla log dir.
        let hostile = update_log_file_name("wsl", "../../etc/passwd", "20260102-030405");
        assert_eq!(hostile, "update-wsl-etc-passwd-20260102-030405.log");
        assert!(!hostile.contains('/') && !hostile.contains('\\') && !hostile.contains(".."), "{hostile}");
    }
    #[test]
    fn update_log_name_falls_back_when_label_is_empty() {
        assert_eq!(
            update_log_file_name("wsl", "   ", "20260102-030405"),
            "update-wsl-env-20260102-030405.log"
        );
    }
    #[test]
    fn save_update_log_writes_the_exact_output() {
        let dir = std::env::temp_dir().join(format!("dsh-log-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(update_log_file_name("windows", "windows", "20260102-030405"));
        // Output di spawn fallito: deve finire nel file cosi com'e.
        let output = "errore esecuzione npm: Impossibile trovare il file specificato. (os error 2)\n";
        write_log_file(&path, output).expect("log scritto");
        let written = std::fs::read_to_string(&path).expect("log leggibile");
        assert_eq!(written, output);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
    #[test]
    fn find_auth_url_returns_none_on_timeout() {
        let r = FakeRunner::default();
        let fs = FakeFs::default();
        assert!(find_auth_url_with(&r, &fs, "windows", None, 39999, Duration::from_millis(1), |_| {}).is_none());
    }
    #[test]
    fn find_auth_url_reads_token_from_fake_log() {
        let log = windows_log_path(39998);
        let fs = FakeFs::with(&log.to_string_lossy(), "dsh web: http://127.0.0.1:39998/?token=abc\n");
        let r = FakeRunner::default();
        assert_eq!(find_auth_url_with(&r, &fs, "windows", None, 39998, Duration::from_secs(5), |_| {}).as_deref(), Some("http://127.0.0.1:39998/?token=abc"));
    }
    struct AcceptAll;
    impl CommandRunner for AcceptAll {
        fn run_capture(&self, _p: &str, _a: &[&str], _t: Duration) -> Result<(i32, String, String), String> { Ok((0, String::new(), String::new())) }
    }
}

