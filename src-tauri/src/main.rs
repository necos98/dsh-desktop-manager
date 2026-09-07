// DSH Desktop Manager - backend Rust (Tauri 2), struttura SOLID.
//
// main.rs e solo composition root: comandi Tauri sottili + bootstrap.
// La logica vive nei moduli (SRP):
//  - model.rs     tipi serializzati + env_key/child_webview_label (puri)
//  - util.rs      decode_output/parse_wsl_distros/first_semver/extract_auth_url (puri)
//  - proc.rs      porte CommandRunner/PortProber/FsAccess + SystemRunner (DIP)
//  - detect.rs    detection Windows/WSL dietro le porte (testabile con fake)
//  - lifecycle.rs start/stop/update/log/auth-url dietro le porte
//  - webviews.rs  geometria/label/url puri per layout_tabs
//
// Comandi esposti al frontend (contratto invariato):
//  detect_windows / list_wsl_distros / probe_wsl / is_port_open /
//  find_free_port / start_env / stop_env / layout_tabs / open_in_browser / run_update
//
// Auto-update: plugin tauri_plugin_updater (check/download/install da
// latest.json delle GitHub Releases) + tauri_plugin_process (relaunch).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod detect;
mod lifecycle;
mod model;
mod proc;
mod util;
mod webviews;

use std::sync::Mutex;
use std::time::Duration;

use tauri::{LogicalPosition, LogicalSize, Manager, RunEvent, State, WebviewUrl};
use tauri::webview::{NewWindowResponse, WebviewBuilder};

use detect::{detect_windows_sync, list_wsl_distros_with, probe_wsl_with};
use lifecycle::{
    find_auth_url_sync, find_free_port_with, run_update_with, start_windows_with, start_wsl_with,
    stop_wsl_with, SystemSpawner,
};
use crate::proc::CommandRunner;
use model::{
    child_webview_label, env_key, EnvProbe, EnvTarget, LayoutInput, StartResult, StopResult,
    UpdateResult, WslDistro,
};

/// Stato gestito: processi avviati dal manager (mai quelli esterni).
struct Procs {
    inner: Mutex<proc::ProcRegistry>,
}

/// Esegue un comando con timeout su un thread dedicato (versione async-safe).
async fn blocking<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Comandi Tauri (sottili: delegano ai moduli)
// ---------------------------------------------------------------------------

#[tauri::command]
async fn detect_windows() -> EnvProbe {
    blocking(|| Ok(detect_windows_sync())).await.unwrap_or_else(|e| EnvProbe {
        kind: "windows".into(),
        name: "Windows".into(),
        distro: None,
        installed: false,
        version: None,
        executable: None,
        dsh_home: None,
        error: Some(e),
    })
}

#[tauri::command]
async fn list_wsl_distros() -> Result<Vec<WslDistro>, String> {
    blocking(|| list_wsl_distros_with(&proc::SystemRunner)).await
}

#[tauri::command]
async fn probe_wsl(distro: String) -> Result<EnvProbe, String> {
    let d = distro.clone();
    blocking(move || probe_wsl_with(&proc::SystemRunner, &d)).await
}

#[tauri::command]
async fn is_port_open(port: u16) -> bool {
    blocking(move || Ok(proc::is_port_open_inner(port))).await.unwrap_or(false)
}

#[tauri::command]
async fn find_free_port(from: u16) -> u16 {
    blocking(move || Ok(find_free_port_with(&proc::SystemRunner, from))).await.unwrap_or(from)
}

#[tauri::command]
async fn start_env(state: State<'_, Procs>, target: EnvTarget) -> Result<StartResult, String> {
    let key = env_key(&target);
    let kind = target.kind.clone();
    let name = target.name.clone();
    let closure_target = target.clone();
    let result = blocking(move || {
        let runner = proc::SystemRunner;
        let spawner = SystemSpawner;
        let (pid, used_port, reached) = if closure_target.kind == "windows" {
            let exe = detect_windows_sync().executable;
            start_windows_with(&runner, &runner, &spawner, move || exe.clone(), &closure_target, |ms| std::thread::sleep(std::time::Duration::from_millis(ms)))
        } else {
            start_wsl_with(&runner, &runner, &spawner, &closure_target, |ms| std::thread::sleep(std::time::Duration::from_millis(ms)))
        }?;
        // Server raggiungibile: acquisisce l'URL autenticato (?token=) dal log.
        // Se non compare in tempo, l'avvio resta valido (modalita' degradata).
        let auth_url = if reached {
            find_auth_url_sync(
                &runner,
                &runner,
                &closure_target.kind,
                closure_target.distro.as_deref(),
                used_port,
            )
        } else {
            None
        };
        Ok((pid, used_port, reached, auth_url))
    })
    .await;

    match result {
        Ok((pid, used_port, reached, auth_url)) => {
            {
                let mut reg = state.inner.lock().unwrap();
                if let Some(p) = pid {
                    reg.remember_pid(key.clone(), p);
                } else if kind != "windows" {
                    reg.remember_wsl(key.clone());
                }
            }
            let msg = if reached {
                format!("dsh avviato su {name} — GUI su http://127.0.0.1:{used_port}")
            } else {
                format!(
                    "Comando di avvio lanciato su {name} (porta {used_port}) ma la GUI non risponde ancora. Controlla i log dell'ambiente."
                )
            };
            Ok(StartResult { ok: true, message: msg, pid, port: used_port, reached, auth_url })
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
async fn stop_env(state: State<'_, Procs>, target: EnvTarget) -> Result<StopResult, String> {
    let key = env_key(&target);
    let kind = target.kind.clone();
    let port = target.port;

    if kind == "windows" {
        let pid = state.inner.lock().unwrap().take_pid(&key);
        return Ok(match pid {
            Some(p) => {
                let ok = proc::kill_pid_tree(p);
                StopResult {
                    ok,
                    message: if ok {
                        format!("Processo dsh (PID {p}) arrestato.")
                    } else {
                        format!("Impossibile arrestare il PID {p}.")
                    },
                }
            }
            None => StopResult {
                ok: false,
                message: format!(
                    "Nessuna istanza di dsh su Windows avviata da questo manager (porta {port}). Per sicurezza non fermo processi esterni."
                ),
            },
        });
    }

    let distro = target.distro.clone();
    let result = blocking(move || match distro {
        Some(_d) => stop_wsl_with(&proc::SystemRunner, &target),
        None => Err("distro WSL mancante".to_string()),
    })
    .await;
    state.inner.lock().unwrap().forget_wsl(&key);
    match result {
        Ok(msg) => Ok(StopResult { ok: true, message: msg }),
        Err(e) => Ok(StopResult { ok: false, message: e }),
    }
}

/// Crea/posiziona le webview figlie (GUI DSH) dentro la finestra principale,
/// sotto la barra delle tab. La tab attiva è visibile; le altre sono fuori
/// schermo (così lo z-order non conta e lo switch è istantaneo).
/// Comando async: la creazione di webview non deve avvenire sul main thread.
#[tauri::command]
async fn layout_tabs(app: tauri::AppHandle, input: LayoutInput) -> Result<(), String> {
    let window = app
        .get_window("main")
        .ok_or_else(|| "finestra 'main' non trovata".to_string())?;

    let (tabbar, width, content_h) =
        webviews::content_geometry(input.tabbar, input.width, input.height);
    let visible_pos = LogicalPosition::new(0, tabbar);
    let content_size = LogicalSize::new(width, content_h);

    // 1) crea le webview figlie mancanti (una per ambiente in esecuzione).
    // Le label vengono dal modulo webviews (regola centralizzata e testata).
    for label in webviews::labels_to_ensure(&input) {
        if window.webviews().iter().any(|w| w.label() == label) {
            continue;
        }
        let tab = input.tabs.iter().find(|t| child_webview_label(&t.env_id) == label);
        let url_str = tab.map(|t| t.url.as_str()).unwrap_or("about:blank");
        let url = url::Url::parse(url_str).map_err(|e| format!("URL non valido: {e}"))?;
        // Link esterni (non GUI locale) -> browser predefinito dell'utente:
        // la navigazione dentro la webview viene cancellata (false) e
        // `window.open` / target=_blank viene negato dopo aver aperto il browser.
        let builder = WebviewBuilder::new(label, WebviewUrl::External(url))
            .on_navigation(|nav_url| {
                if webviews::is_external_url(nav_url) {
                    let _ = webviews::open_in_system_browser(nav_url.as_str());
                    return false;
                }
                true
            })
            .on_new_window(|nav_url, _features| {
                if webviews::is_external_url(&nav_url) {
                    let _ = webviews::open_in_system_browser(nav_url.as_str());
                }
                NewWindowResponse::Deny
            });
        window
            .add_child(builder, visible_pos.clone(), content_size.clone())
            .map_err(|e| format!("creazione webview figlia: {e}"))?;
    }

    // 2) aggiorna gli URL (es. porta cambiata dopo un riavvio) e posiziona
    let active_label = webviews::active_label(&input);
    let off_pos = LogicalPosition::new(0, -40000);
    for w in window.webviews() {
        if !w.label().starts_with("web-") {
            continue;
        }
        // naviga verso l'URL desiderato se diverso da quello corrente
        if let Some(want) = webviews::wanted_url(&input, w.label()) {
            if let Ok(want) = url::Url::parse(&want) {
                if w.url().ok().as_ref() != Some(&want) {
                    let _ = w.navigate(want);
                }
            }
        }
        if Some(w.label()) == active_label.as_deref() {
            let _ = w.set_position(visible_pos.clone());
            let _ = w.set_size(content_size.clone());
            let _ = w.set_focus();
        } else {
            let _ = w.set_position(off_pos.clone());
            let _ = w.set_size(content_size.clone());
        }
    }
    Ok(())
}

#[tauri::command]
fn open_in_browser(url: String) -> Result<(), String> {
    webviews::open_in_system_browser(&url)
}

#[tauri::command]
async fn run_update(target: EnvTarget, version: String) -> UpdateResult {
    let kind = target.kind.clone();
    let _ = kind; // il branch vive in lifecycle::run_update_with
    let result = blocking(move || -> Result<(i32, String), String> {
        let runner = proc::SystemRunner;
        let (code, output) = run_update_with(
            &runner,
            &runner,
            proc::home_dir(),
            &target,
            &version,
        );
        Ok((code, output))
    })
    .await;

    match result {
        Ok((exit_code, output)) => UpdateResult {
            ok: exit_code == 0,
            exit_code,
            output,
        },
        Err(e) => UpdateResult { ok: false, exit_code: -1, output: e },
    }
}

// ---------------------------------------------------------------------------
// Main / setup
// ---------------------------------------------------------------------------

fn cleanup_started(state: &Procs) {
    let pids: Vec<u32> = state.inner.lock().unwrap().pids.values().copied().collect();
    for p in pids {
        let _ = proc::kill_pid_tree(p);
    }
    let wsl_keys: Vec<String> = state.inner.lock().unwrap().wsl_started.clone();
    for key in wsl_keys {
        if let Some(distro) = key.strip_prefix("wsl:") {
            let script = r#"export PATH="$HOME/.bun/bin:$PATH";pkill -f 'dsh web' || true"#;
            let mut args = detect::wsl_args_after(distro);
            args.push(script.to_string());
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let _ = proc::SystemRunner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(15));
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(Procs {
            inner: Mutex::new(proc::ProcRegistry::default()),
        })
        .invoke_handler(tauri::generate_handler![
            detect_windows,
            list_wsl_distros,
            probe_wsl,
            is_port_open,
            find_free_port,
            start_env,
            stop_env,
            layout_tabs,
            open_in_browser,
            run_update
        ])
        .build(tauri::generate_context!())
        .expect("errore durante l'avvio di DSH Desktop Manager")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = app.try_state::<Procs>() {
                    cleanup_started(&state);
                }
            }
        });
}
