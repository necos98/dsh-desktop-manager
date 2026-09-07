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
    probe
}

/// Produzione: home reale.
pub fn detect_windows_sync() -> EnvProbe {
    detect_windows_with(&crate::proc::SystemRunner, &crate::proc::SystemRunner, home_dir())
}

pub fn wsl_args_after(distro: &str) -> Vec<String> {
    vec![
        "-d".into(),
        distro.to_string(),
        "--".into(),
        "bash".into(),
        "-lc".into(),
    ]
}

/// Script sentinel di probe. Il PATH va quotato: con Windows PATH ereditato
/// contenente parentesi (es. Program Files (x86)) l'export non quotato esce
/// con exit 2. Pubblico cosi i test usano la stessa stringa della produzione.
pub fn probe_script() -> String {
    r#"export PATH="$HOME/.bun/bin:$PATH";export BUN_INSTALL="$HOME/.bun";if command -v dsh >/dev/null 2>&1;then dsh --version;echo __DSH_OK__;else echo __DSH_NOT_FOUND__;fi"#.to_string()
}

pub fn probe_wsl_with(runner: &dyn CommandRunner, distro: &str) -> Result<EnvProbe, String> {
    let mut probe = EnvProbe {
        kind: "wsl".to_string(),
        name: distro.to_string(),
        distro: Some(distro.to_string()),
        installed: false,
        version: None,
        executable: None,
        dsh_home: None,
        error: None,
    };
    let script = probe_script();
    let mut args = wsl_args_after(distro);
    args.push(script.to_string());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, out, err) = runner.run_capture("wsl.exe", &arg_refs, WSL_BOOT_TIMEOUT)?;
    let combined = format!("{out}\n{err}");
    if combined.contains("__DSH_NOT_FOUND__") {
        probe.installed = false;
        return Ok(probe);
    }
    if combined.contains("__DSH_OK__") {
        probe.installed = true;
        probe.version = first_semver(&out);
        probe.executable = Some("dsh (PATH distro, tipicamente ~/.bun/bin)".to_string());
        return Ok(probe);
    }
    // Probabile errore di livello WSL (distro sconosciuta, wsl non attivo, ...).
    // Nota: wsl.exe scrive questi errori su stdout (UTF-16LE), quindi
    // err e' spesso vuoto: preferire combined per non perdere il messaggio.
    let msg = if code == 0 {
        combined.trim()
    } else if !err.trim().is_empty() {
        err.trim()
    } else {
        combined.trim()
    };
    probe.error = Some(if msg.is_empty() {
        format!("wsl.exe exit {code}")
    } else {
        msg.to_string()
    });
    probe.installed = false;
    Ok(probe)
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
        EnvTarget { kind: "windows".into(), name: "Windows".into(), distro: None, port: 3080, extra_args: vec![], workspace: None }
    }

    #[test]
    fn wsl_args_shape() {
        assert_eq!(wsl_args_after("Ubuntu"), vec!["-d", "Ubuntu", "--", "bash", "-lc"]);
    }
    #[test]
    fn probe_script_quotes_path() {
        let s = probe_script();
        assert!(s.contains("export PATH="));
        assert!(s.contains("__DSH_OK__"));
        assert!(s.contains("__DSH_NOT_FOUND__"));
    }
    #[test]
    fn probe_not_found_marks_uninstalled() {
        let s = probe_script();
        let r = FakeRunner::with_output("wsl.exe", &["-d", "U", "--", "bash", "-lc", &s], 0, "__DSH_NOT_FOUND__\n", "");
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(!probe.installed);
        assert_eq!(probe.distro.as_deref(), Some("U"));
    }
    #[test]
    fn probe_ok_sets_version() {
        let s = probe_script();
        let r = FakeRunner::with_output("wsl.exe", &["-d", "U", "--", "bash", "-lc", &s], 0, "dsh version 1.2.3\n__DSH_OK__\n", "");
        let probe = probe_wsl_with(&r, "U").unwrap();
        assert!(probe.installed);
        assert_eq!(probe.version.as_deref(), Some("1.2.3"));
    }
    #[test]
    fn probe_wsl_error_reports_combined_output() {
        let s = probe_script();
        let r = FakeRunner::with_output("wsl.exe", &["-d", "Nope", "--", "bash", "-lc", &s], 0, "strano", "");
        let probe = probe_wsl_with(&r, "Nope").unwrap();
        assert!(!probe.installed);
        assert!(probe.error.unwrap().contains("strano"));
    }
    #[test]
    fn probe_runner_error_propagates() {
        let r = FakeRunner::with_error("wsl.exe", &["-d", "X", "--", "bash", "-lc", "Y"], "timeout");
        assert!(probe_wsl_with(&r, "X").is_err());
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
