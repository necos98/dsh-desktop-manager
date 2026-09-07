//! Ciclo di vita istanze (SRP): avvio/arresto/update + log/auth-url.
//! Dipende dalle porte CommandRunner/PortProber/FsAccess (DIP); gli spawn
//! reali restano dietro ProcessSpawner (test: FakeSpawner, nessun processo).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::detect::wsl_args_after;
use crate::model::EnvTarget;
use crate::proc::{find_free_port_inner, home_dir, CommandRunner, FsAccess, PortProber};
use crate::util::extract_auth_url;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(600);
const START_POLL: Duration = Duration::from_secs(60);
const AUTH_WAIT: Duration = Duration::from_secs(30);

/// Spawn di processi detached (Windows CREATE_NO_WINDOW / WSL via wsl.exe).
/// Separato da CommandRunner (che cattura l'output) per ISP.
pub trait ProcessSpawner: Send + Sync {
    fn spawn_windows_detached(&self, exe: &str, args: &[&str], workdir: &PathBuf, log_path: &PathBuf) -> Result<u32, String>;
    fn spawn_wsl_detached(&self, runner: &dyn CommandRunner, distro: &str, script: &str) -> Result<(), String>;
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
    fn spawn_wsl_detached(&self, runner: &dyn CommandRunner, distro: &str, script: &str) -> Result<(), String> {
        let mut args = wsl_args_after(distro);
        args.push(script.to_string());
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (code, _out, err) = runner.run_capture("wsl.exe", &arg_refs, crate::detect::WSL_BOOT_TIMEOUT)?;
        if code != 0 {
            return Err(if err.trim().is_empty() { format!("wsl exit {code}") } else { err.trim().to_string() });
        }
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

fn read_windows_dsh_log(fs: &dyn FsAccess, port: u16) -> Option<String> {
    let path = windows_log_path(port);
    fs.read_to_string(&path).filter(|t| !t.is_empty())
}

fn read_wsl_dsh_log(runner: &dyn CommandRunner, distro: &str, port: u16) -> Option<String> {
    let script = format!("cat /tmp/dsh-desktop-manager-{}.log 2>/dev/null", port);
    let mut args = wsl_args_after(distro);
    args.push(script);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(15)) {
        Ok((_, out, _)) if !out.trim().is_empty() => Some(out),
        _ => None,
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
            distro.and_then(|d| read_wsl_dsh_log(runner, d, port))
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

/// Avvio WSL: (None, porta, reached). Script con PATH quotato (vedi detect.rs).
pub fn start_wsl_with(
    runner: &dyn CommandRunner,
    prober: &dyn PortProber,
    spawner: &dyn ProcessSpawner,
    t: &EnvTarget,
    sleep_ms: impl Fn(u64),
) -> Result<(Option<u32>, u16, bool), String> {
    start_wsl_with_timeout(runner, prober, spawner, t, START_POLL, sleep_ms)
}

/// Variante con timeout esplicito (test: millisecondi, nessun vero sleep).
pub fn start_wsl_with_timeout(
    runner: &dyn CommandRunner,
    prober: &dyn PortProber,
    spawner: &dyn ProcessSpawner,
    t: &EnvTarget,
    poll_max: Duration,
    sleep_ms: impl Fn(u64),
) -> Result<(Option<u32>, u16, bool), String> {
    let distro = t.distro.clone().ok_or_else(|| "distro WSL mancante".to_string())?;
    let ws = t
        .workspace
        .clone()
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| "~".to_string());
    let extra = t.extra_args.join(" ");
    let script = format!(
        r#"export PATH="$HOME/.bun/bin:$PATH";export BUN_INSTALL="$HOME/.bun";cd {ws} 2>/dev/null || cd ~;nohup dsh web --port {} --no-open {extra} >/tmp/dsh-desktop-manager-{}.log 2>&1 </dev/null &"#,
        t.port, t.port
    );
    spawner.spawn_wsl_detached(runner, &distro, &script)?;
    Ok((None, t.port, crate::proc::poll_port(prober, t.port, poll_max, sleep_ms)))
}

/// Arresto WSL via pkill nella distro.
pub fn stop_wsl_with(runner: &dyn CommandRunner, t: &EnvTarget) -> Result<String, String> {
    let distro = t.distro.clone().ok_or_else(|| "distro WSL mancante".to_string())?;
    let script = format!(
        r#"export PATH="$HOME/.bun/bin:$PATH";pkill -f 'dsh web --port {}' || true"#,
        t.port
    );
    let mut args = wsl_args_after(&distro);
    args.push(script);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, _out, err) = runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(30))?;
    if code != 0 && !err.trim().is_empty() {
        return Err(err.trim().to_string());
    }
    Ok(format!("Comando di arresto inviato nella distro {distro} (porta {}).", t.port))
}

/// Update Windows (bun add -g) oppure WSL (bun o npm nella distro).
pub fn run_update_with(runner: &dyn CommandRunner, fs: &dyn FsAccess, home: Option<PathBuf>, t: &EnvTarget, version: &str) -> (i32, String) {
    let pkg = format!("@deepseek-ai/dsh@{version}");
    if t.kind == "windows" {
        let installer = home
            .as_ref()
            .map(|h| h.join(".bun").join("bin").join("bun.exe"))
            .filter(|p| fs.path_exists(p))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "bun".to_string());
        match runner.run_capture(&installer, &["add", "-g", &pkg], UPDATE_TIMEOUT) {
            Ok((code, out, err)) => (code, format!("{out}\n{err}")),
            Err(e) => (-1, e),
        }
    } else {
        match t.distro.clone() {
            Some(d) => {
                let script = format!(
                r#"export PATH="$HOME/.bun/bin:$PATH";if command -v bun >/dev/null 2>&1;then bun add -g '{pkg}';else npm install -g '{pkg}';fi"#
                );
                let mut args = wsl_args_after(&d);
                args.push(script);
                let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                match runner.run_capture("wsl.exe", &arg_refs, UPDATE_TIMEOUT) {
                    Ok((code, out, err)) => (code, format!("{out}\n{err}")),
                    Err(e) => (-1, e),
                }
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

    struct FakeSpawner { pid: u32, fail: Option<String>, last_script: std::sync::Mutex<Option<String>> }
    impl FakeSpawner {
        fn ok(pid: u32) -> Self { Self { pid, fail: None, last_script: std::sync::Mutex::new(None) } }
    }
    impl ProcessSpawner for FakeSpawner {
        fn spawn_windows_detached(&self, _exe: &str, _args: &[&str], _workdir: &PathBuf, _log: &PathBuf) -> Result<u32, String> {
            match &self.fail { Some(e) => Err(e.clone()), None => Ok(self.pid) }
        }
        fn spawn_wsl_detached(&self, _runner: &dyn CommandRunner, _distro: &str, script: &str) -> Result<(), String> {
            *self.last_script.lock().unwrap() = Some(script.to_string());
            match &self.fail { Some(e) => Err(e.clone()), None => Ok(()) }
        }
    }
    fn target(kind: &str, distro: Option<&str>, port: u16) -> EnvTarget {
        EnvTarget { kind: kind.into(), name: kind.into(), distro: distro.map(|s| s.into()), port, extra_args: vec![], workspace: None }
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
    fn start_wsl_builds_quoted_script() {
        let runner = FakeRunner::default();
        let prober = FakeProber { open: vec![] };
        let sp = FakeSpawner::ok(0);
        let t = target("wsl", Some("Ubuntu"), 3100);
        let (pid, port, _) = start_wsl_with_timeout(&runner, &prober, &sp, &t, Duration::from_millis(1), |_| {}).unwrap();
        assert_eq!(pid, None);
        assert_eq!(port, 3100);
        let s = sp.last_script.lock().unwrap().clone().unwrap();
        assert!(s.contains("export PATH="));
        assert!(s.contains("nohup dsh web --port 3100"));
    }
    #[test]
    fn start_wsl_requires_distro() {
        let runner = FakeRunner::default();
        let prober = FakeProber { open: vec![] };
        let sp = FakeSpawner::ok(0);
        assert!(start_wsl_with_timeout(&runner, &prober, &sp, &target("wsl", None, 3100), Duration::from_millis(1), |_| {}).is_err());
    }
    #[test]
    fn stop_wsl_ok_message() {
        let s = crate::detect::probe_script();
        let _ = s;
        let t = target("wsl", Some("Debian"), 3101);
        let script_prefix = "export PATH=";
        let _ = script_prefix;
        let r = FakeRunner::default();
        // chiave esatta: usiamo stop_wsl_with con runner che accetta tutto
        let any = AcceptAll;
        let msg = stop_wsl_with(&any, &t).unwrap();
        assert!(msg.contains("Debian"));
        assert!(msg.contains("3101"));
        let _ = r;
    }
    #[test]
    fn stop_wsl_requires_distro() {
        let any = AcceptAll;
        assert!(stop_wsl_with(&any, &target("wsl", None, 3100)).is_err());
    }
    #[test]
    fn update_windows_prefers_bun_exe() {
        let home = PathBuf::from("/home/u");
        let fs = FakeFs::with("/home/u/.bun/bin/bun.exe", "");
        let bun = PathBuf::from("/home/u").join(".bun").join("bin").join("bun.exe").to_string_lossy().into_owned();
        let r = FakeRunner::with_output(&bun, &["add", "-g", "@deepseek-ai/dsh@2.0.0"], 0, "ok", "");
        let (code, out) = run_update_with(&r, &fs, Some(home), &target("windows", None, 3080), "2.0.0");
        assert_eq!(code, 0);
        assert!(out.contains("ok"));
    }
    #[test]
    fn update_windows_falls_back_to_bun_on_path() {
        let fs = FakeFs::default();
        let r = FakeRunner::with_output("bun", &["add", "-g", "@deepseek-ai/dsh@1.0.0"], 1, "", "err");
        let (code, out) = run_update_with(&r, &fs, Some(PathBuf::from("/h")), &target("windows", None, 3080), "1.0.0");
        assert_eq!(code, 1);
        assert!(out.contains("err"));
    }
    #[test]
    fn update_wsl_reports_missing_distro() {
        let r = FakeRunner::default();
        let (code, _) = run_update_with(&r, &FakeFs::default(), None, &target("wsl", None, 3100), "1.0.0");
        assert_eq!(code, -1);
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

