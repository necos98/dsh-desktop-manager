//! Porta processi/rete (DIP): tutto cio che tocca OS va dietro trait.
//! Produzione = SystemRunner (Command/TcpStream/fs); i test iniettano FakeRunner.
//! (ISP: trait piccoli e separati per esecuzione, rete, filesystem, clock.)

use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::util::decode_output;

/// Esecuzione processi con timeout. Risultato: (exit_code, stdout, stderr).
pub trait CommandRunner: Send + Sync {
    fn run_capture(&self, prog: &str, args: &[&str], timeout: Duration) -> Result<(i32, String, String), String>;
}

/// Sonda TCP (per is_port_open / poll_port / find_free_port).
pub trait PortProber: Send + Sync {
    fn is_open(&self, port: u16) -> bool;
}

/// Accesso filesystem minimo (detection Windows + log).
/// Solo lettura/esistenza: la scrittura log resta nel modulo log (SRP).
pub trait FsAccess: Send + Sync {
    fn read_to_string(&self, path: &Path) -> Option<String>;
    fn path_exists(&self, path: &Path) -> bool;
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run_capture(&self, prog: &str, args: &[&str], timeout: Duration) -> Result<(i32, String, String), String> {
        let prog = prog.to_string();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (tx, rx) = mpsc::channel();
        let prog_msg = prog.clone();
        std::thread::spawn(move || {
            let out = Command::new(&prog).args(&args).output();
            let _ = tx.send(out);
        });
        match rx.recv_timeout(timeout) {
            Ok(Ok(o)) => Ok((
                o.status.code().unwrap_or(-1),
                decode_output(o.stdout),
                decode_output(o.stderr),
            )),
            Ok(Err(e)) => Err(format!("errore esecuzione {prog_msg}: {e}")),
            Err(_) => Err(format!("timeout dopo {}s eseguendo {prog_msg}", timeout.as_secs())),
        }
    }
}

impl PortProber for SystemRunner {
    fn is_open(&self, port: u16) -> bool {
        is_port_open_inner(port)
    }
}

impl FsAccess for SystemRunner {
    fn read_to_string(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

pub fn is_port_open_inner(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap_or_else(|_| {
        // fallback non raggiungibile
        SocketAddr::from(([127, 0, 0, 1], 0))
    });
    if addr.port() == 0 {
        return false;
    }
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// Prima porta libera da `from` (minimo 1024, finestra di 200).
/// Pura rispetto alla porta PortProber: testabile con FakeProber.
pub fn find_free_port_inner(prober: &dyn PortProber, from: u16) -> u16 {
    let start = from.max(1024);
    for p in start..start.saturating_add(200) {
        if !prober.is_open(p) {
            return p;
        }
    }
    start
}

/// Attende che la porta risponda, fino a max_wait (poll 400ms).
/// Dietro PortProber + sleep iniettabile nei test via `sleep_ms`.
pub fn poll_port(prober: &dyn PortProber, port: u16, max_wait: Duration, sleep_ms: impl Fn(u64)) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < max_wait {
        if prober.is_open(port) {
            return true;
        }
        sleep_ms(400);
    }
    prober.is_open(port)
}

/// Registry dei processi avviati dal manager (Windows pid + chiavi WSL).
/// Solo i pid qui dentro possono essere killati (mai processi esterni).
#[derive(Default)]
pub struct ProcRegistry {
    pub pids: HashMap<String, u32>,
    pub wsl_started: Vec<String>,
}

impl ProcRegistry {
    pub fn remember_pid(&mut self, key: String, pid: u32) {
        self.pids.insert(key, pid);
    }
    pub fn remember_wsl(&mut self, key: String) {
        self.wsl_started.push(key);
    }
    pub fn take_pid(&mut self, key: &str) -> Option<u32> {
        self.pids.remove(key)
    }
    pub fn forget_wsl(&mut self, key: &str) {
        self.wsl_started.retain(|k| k != key);
    }
}

pub fn kill_pid_tree(pid: u32) -> bool {
    let res = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    matches!(res, Ok(o) if o.status.success())
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// FakeRunner: risposte programmate per comando (DIP nei test).
    /// Chiave = "prog arg1 arg2 ...".
    #[derive(Default)]
    pub struct FakeRunner {
        pub outputs: Mutex<HashMap<String, (i32, String, String)>>,
        pub errors: Mutex<HashMap<String, String>>,
        pub calls: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        pub fn with_output(prog: &str, args: &[&str], code: i32, out: &str, err: &str) -> Self {
            let f = Self::default();
            f.outputs.lock().unwrap().insert(key_of(prog, args), (code, out.to_string(), err.to_string()));
            f
        }
        pub fn with_error(prog: &str, args: &[&str], msg: &str) -> Self {
            let f = Self::default();
            f.errors.lock().unwrap().insert(key_of(prog, args), msg.to_string());
            f
        }
    }

        fn key_of(prog: &str, args: &[&str]) -> String {
            let mut k = prog.to_string();
            for a in args {
                k.push(' ');
                k.push_str(a);
            }
            k
        }

    impl CommandRunner for FakeRunner {
        fn run_capture(&self, prog: &str, args: &[&str], _timeout: Duration) -> Result<(i32, String, String), String> {
            let k = key_of(prog, args);
            self.calls.lock().unwrap().push(k.clone());
            if let Some(msg) = self.errors.lock().unwrap().get(&k) {
                return Err(msg.clone());
            }
            self.outputs.lock().unwrap().get(&k).cloned().ok_or_else(|| format!("comando non stubbato: {k}"))
        }
    }

    /// FakeProber: porte aperte programmate.
    pub struct FakeProber {
        pub open: Vec<u16>,
    }

    impl PortProber for FakeProber {
        fn is_open(&self, port: u16) -> bool {
            self.open.contains(&port)
        }
    }

    /// FakeFs: file programmati in memoria.
    #[derive(Default)]
    pub struct FakeFs {
        pub files: HashMap<PathBuf, String>,
    }

    impl FakeFs {
        pub fn with(path: &str, content: &str) -> Self {
            let mut f = Self::default();
            f.files.insert(PathBuf::from(path), content.to_string());
            f
        }
    }

    impl FsAccess for FakeFs {
        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.files.get(path).cloned()
        }
        fn path_exists(&self, path: &Path) -> bool {
            self.files.contains_key(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::time::Duration;

    #[test]
    fn find_free_port_skips_open() {
        let p = FakeProber { open: vec![3080, 3081] };
        assert_eq!(find_free_port_inner(&p, 3080), 3082);
    }

    #[test]
    fn find_free_port_clamps_below_1024() {
        let p = FakeProber { open: vec![] };
        assert_eq!(find_free_port_inner(&p, 80), 1024);
    }

    #[test]
    fn find_free_port_falls_back_when_window_full() {
        let p = FakeProber { open: (1024..1224).collect() };
        assert_eq!(find_free_port_inner(&p, 1024), 1024);
    }

    #[test]
    fn poll_port_true_when_already_open() {
        let p = FakeProber { open: vec![3080] };
        assert!(poll_port(&p, 3080, Duration::from_secs(5), |_| {}));
    }

    #[test]
    fn poll_port_false_on_timeout_without_sleep() {
        let p = FakeProber { open: vec![] };
        assert!(!poll_port(&p, 3080, Duration::from_millis(1), |_| {}));
    }

    #[test]
    fn fake_runner_records_calls_and_errors() {
        let f = FakeRunner::with_output("wsl.exe", &["-l", "-v"], 0, "out", "err");
        let (code, out, err) = f.run_capture("wsl.exe", &["-l", "-v"], Duration::from_secs(1)).unwrap();
        assert_eq!((code, out.as_str(), err.as_str()), (0, "out", "err"));
        assert_eq!(f.calls.lock().unwrap().len(), 1);
        let e = FakeRunner::with_error("wsl.exe", &["-l"], "boom");
        assert!(e.run_capture("wsl.exe", &["-l"], Duration::from_secs(1)).is_err());
        assert!(f.run_capture("wsl.exe", &["altro"], Duration::from_secs(1)).is_err());
    }

    #[test]
    fn fake_fs_reads_programmed_files() {
        let fs = FakeFs::with("/x/package.json", "hello");
        assert_eq!(fs.read_to_string(std::path::Path::new("/x/package.json")).as_deref(), Some("hello"));
        assert!(!fs.path_exists(std::path::Path::new("/missing")));
    }

    #[test]
    fn proc_registry_tracks_only_managed_pids() {
        let mut r = ProcRegistry::default();
        r.remember_pid("windows".into(), 123);
        r.remember_wsl("wsl:U".into());
        assert_eq!(r.take_pid("windows"), Some(123));
        assert_eq!(r.take_pid("windows"), None);
        r.forget_wsl("wsl:U");
        assert!(r.wsl_started.is_empty());
    }
}
