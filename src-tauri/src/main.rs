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
// Comandi esposti al frontend:
//  detect_windows / list_wsl_distros / probe_wsl(distro, node_runtime?) /
//  list_node_runtimes(distro) / is_port_open / find_free_port /
//  start_env / stop_env / layout_tabs / open_in_browser / run_update /
//  read_env_log / diagnose_wsl(distro, port, node_runtime?)
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
use std::time::{Duration, Instant};

use tauri::{LogicalPosition, LogicalSize, Manager, RunEvent, State, WebviewUrl};
use tauri::webview::{NewWindowResponse, WebviewBuilder};

use detect::{list_node_runtimes_with, list_wsl_distros_with, probe_wsl_fast_with, probe_wsl_with_runtime, CachedDistro, PathSnapshot, WSL_BOOT_TIMEOUT, WSL_WARM_TIMEOUT};
use lifecycle::{
    diagnose_wsl_with_runtime, find_auth_url_sync, find_free_port_with, read_env_log_with,
    run_update_with, start_windows_with, start_wsl_with, stop_wsl_with, SystemSpawner,
};
use model::{
    child_webview_label, env_key, EnvProbe, EnvTarget, LayoutInput, NodeRuntime, StartResult,
    StopResult, UpdateResult, WslDiag, WslDistro,
};

/// Stato gestito: processi avviati dal manager (mai quelli esterni).
struct Procs {
    inner: Mutex<proc::ProcRegistry>,
}

/// Stato rilevamento: snapshot PATH con TTL (una passata per finestra).
struct DetectCache {
    at: Mutex<Option<Instant>>,
}

/// TTL cache rilevamento Windows (fs locale: la scansione costa poco, ma
/// ogni comando la rifaceva — con N comandi all'avvio si somma).
const DETECT_CACHE_TTL: Duration = Duration::from_secs(30);

/// Cache ultimo `scan_boot` per distro (stato Running + esito sonde): la
/// prima sonda dopo un boot freddo usa WSL_BOOT_TIMEOUT, le altre
/// WSL_WARM_TIMEOUT (15s bastano a distro calda, vedi baseline 0.1s/spawn).
struct WslBootCache {
    inner: Mutex<std::collections::HashMap<String, bool>>,
}

impl WslBootCache {
    fn timeout_for(&self, distro: &str, state: Option<&str>) -> Duration {
        let running = state.map(|s| s.eq_ignore_ascii_case("running")).unwrap_or(false);
        let known = self.inner.lock().unwrap().get(distro).copied().unwrap_or(false);
        if running || known {
            WSL_WARM_TIMEOUT
        } else {
            WSL_BOOT_TIMEOUT
        }
    }

    fn mark_contacted(&self, distro: &str) {
        self.inner.lock().unwrap().insert(distro.to_string(), true);
    }
}

/// Misura un comando e lo registra nel log (strumentazione cold-boot: le
/// fasi di avvio risultano nei log con `BOOT cmd=<nome> ms=<durata>`).
fn log_boot_phase(cmd: &str, started: Instant) {
    eprintln!("BOOT cmd={cmd} ms={}", started.elapsed().as_millis());
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
async fn detect_windows(cache: State<'_, DetectCache>) -> Result<EnvProbe, String> {
    let t0 = Instant::now();
    let path = refresh_detect_cache(&cache);
    let probe = blocking(move || Ok(detect::detect_windows_with_path(&proc::SystemRunner, &proc::SystemRunner, proc::home_dir(), &path)))
    .await
    .unwrap_or_else(|e| EnvProbe {
        kind: "windows".into(),
        name: "Windows".into(),
        distro: None,
        installed: false,
        version: None,
        executable: None,
        dsh_home: None,
        error: Some(e),
        has_bun: None,
        has_npm: None,
    });
    log_boot_phase("detect_windows", t0);
    Ok(probe)
}

/// Rilegge lo snapshot PATH se la cache e scaduta (una sola scansione per
/// finestra TTL invece di una per comando).
fn refresh_detect_cache(cache: &DetectCache) -> PathSnapshot {
    // PathSnapshot non e Clone: ricattura solo quando serve. Il lock resta
    // corto (solo lettura timestamp); la cattura avviene fuori dal lock.
    let stale = {
        let guard = cache.at.lock().unwrap();
        guard.map(|at| at.elapsed() > DETECT_CACHE_TTL).unwrap_or(true)
    };
    if stale {
        let fresh = PathSnapshot::capture();
        // Ricontrolla sotto lock (un altro thread puo aver gia aggiornato).
        let mut guard = cache.at.lock().unwrap();
        if guard.map(|at| at.elapsed() > DETECT_CACHE_TTL).unwrap_or(true) {
            *guard = Some(Instant::now());
            return fresh;
        }
    }
    PathSnapshot::capture()
}

#[tauri::command]
async fn list_wsl_distros() -> Result<Vec<WslDistro>, String> {
    let t0 = Instant::now();
    let out = blocking(|| list_wsl_distros_with(&proc::SystemRunner)).await;
    log_boot_phase("list_wsl_distros", t0);
    out
}

#[tauri::command]
async fn probe_wsl(distro: String, node_runtime: Option<String>) -> Result<EnvProbe, String> {
    let t0 = Instant::now();
    let out = blocking(move || probe_wsl_with_runtime(&proc::SystemRunner, &distro, node_runtime.as_deref())).await;
    log_boot_phase("probe_wsl", t0);
    out
}

/// Sonda WSL veloce a singolo spawn (avvio + refresh): per N distro costa
/// ~N spawn invece di ~5-9N. Timeout caldo di default; la prima sonda dopo
/// un boot freddo usa WSL_BOOT_TIMEOUT (vedi `scan_boot`).
/// `wsl_state`: stato distro da `wsl -l -v` (Running -> timeout caldo).
#[tauri::command]
async fn probe_wsl_fast(
    distro: String,
    wsl_state: Option<String>,
    boot_cache: State<'_, WslBootCache>,
) -> Result<EnvProbe, String> {
    let t0 = Instant::now();
    let timeout = boot_cache.timeout_for(&distro, wsl_state.as_deref());
    let distro_for_cache = distro.clone();
    let out = blocking(move || probe_wsl_fast_with(&proc::SystemRunner, &distro, timeout)).await;
    if out.is_ok() {
        boot_cache.mark_contacted(&distro_for_cache);
    }
    log_boot_phase("probe_wsl_fast", t0);
    out
}

/// Avvio rapido in UN giro: elenco distro + cache HOME/toolchain/stato per
/// la prima pittura, senza sonde per-distro (zero spawn oltre `wsl -l -v`).
/// Il frontend dipinge subito le righe e arricchisce in background.
#[tauri::command]
async fn scan_boot(boot_cache: State<'_, WslBootCache>) -> Result<BootScan, String> {
    let t0 = Instant::now();
    // Snapshot dei nomi gia contattati (fuori dal lock e fuori dal closure
    // 'static: State non puo entrare nei comandi async che ritornano Result
    // con riferimenti — si passa solo una copia owned).
    let known: std::collections::HashSet<String> = boot_cache
        .inner
        .lock()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let out = blocking(move || {
        let distros = list_wsl_distros_with(&proc::SystemRunner)?;
        let mut cached: Vec<CachedDistro> = Vec::with_capacity(distros.len());
        let mut contacted: Vec<String> = vec![];
        for d in &distros {
            // Stopped + mai contattata: NESSUNO spawn (il boot freddo da
            // 4.5s resta in background). Running o nota: UN solo spawn
            // veloce per HOME/toolchain (0.1s, vedi baseline).
            let running = d.state.eq_ignore_ascii_case("running");
            let was_known = known.contains(&d.name);
            if running || was_known {
                match probe_wsl_fast_with(&proc::SystemRunner, &d.name, WSL_WARM_TIMEOUT) {
                    Ok(p) => {
                        contacted.push(d.name.clone());
                        cached.push(CachedDistro {
                            name: d.name.clone(),
                            state: d.state.clone(),
                            home: String::new(),
                            has_bun: p.has_bun.unwrap_or(false),
                            has_npm: p.has_npm.unwrap_or(false),
                            dsh_native_path: p.executable.as_ref().and_then(|e| {
                                e.strip_prefix("dsh nativo (")
                                    .and_then(|s| s.strip_suffix(')'))
                                    .map(|s| s.to_string())
                            }),
                            dsh_version: p.version.clone(),
                        });
                    }
                    Err(_) => {
                        cached.push(CachedDistro {
                            name: d.name.clone(),
                            state: d.state.clone(),
                            ..Default::default()
                        });
                    }
                }
            } else {
                cached.push(CachedDistro {
                    name: d.name.clone(),
                    state: d.state.clone(),
                    ..Default::default()
                });
            }
        }
        Ok(BootScan { distros, cached, contacted })
    })
    .await;
    if let Ok(scan) = &out {
        let mut guard = boot_cache.inner.lock().unwrap();
        for name in &scan.contacted {
            guard.insert(name.clone(), true);
        }
    }
    log_boot_phase("scan_boot", t0);
    out
}

/// Elenco distro + cache per la prima pittura (vedi `scan_boot`).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BootScan {
    distros: Vec<WslDistro>,
    cached: Vec<CachedDistro>,
    /// Distro contattate con successo (il backend le segna nella cache boot).
    #[serde(skip_serializing)]
    contacted: Vec<String>,
}

/// Runtime Node disponibili nella distro (nvm decrescenti + sistema).
/// Mai un Err lanciato per "nessun runtime": lista vuota (la UI mostra
/// l'avviso toolchain). Solo errori di distro irraggiungibile.
#[tauri::command]
async fn list_node_runtimes(distro: String) -> Result<Vec<NodeRuntime>, String> {
    blocking(move || Ok(list_node_runtimes_with(&proc::SystemRunner, &distro))).await
}

#[tauri::command]
async fn is_port_open(port: u16) -> bool {
    blocking(move || Ok(proc::is_port_open_inner(port))).await.unwrap_or(false)
}

/// Coda del log di un ambiente (Windows file / WSL /tmp nella distro).
/// Ritorna (percorso, coda): la UI li mostra nel pannello log dell'ambiente.
#[tauri::command]
async fn read_env_log(target: EnvTarget, lines: Option<usize>) -> Result<(String, String), String> {
    let n = lines.unwrap_or(120);
    blocking(move || {
        read_env_log_with(
            &proc::SystemRunner,
            &proc::SystemRunner,
            &target.kind,
            target.distro.as_deref(),
            target.port,
            n,
        )
    })
    .await
}

/// Diagnostica WSL passo-passo (elenco distro, probe, porte, coda log).
/// Non lancia mai: i fallimenti finiscono in `WslDiag.error`.
/// Stesso runtime scelto della probe (coerenza probe/start/update/diag).
#[tauri::command]
async fn diagnose_wsl(distro: String, port: u16, node_runtime: Option<String>) -> WslDiag {
    let d = distro.clone();
    blocking(move || Ok(diagnose_wsl_with_runtime(&proc::SystemRunner, &proc::SystemRunner, &proc::SystemRunner, &d, port, 40, node_runtime.as_deref())))
        .await
        .unwrap_or_else(|e: String| WslDiag {
            distro,
            state: None,
            dsh_installed: false,
            dsh_version: None,
            has_bun: false,
            has_npm: false,
            port_open_in_distro: None,
            port_open_from_windows: false,
            log_tail: None,
            error: Some(e),
        })
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
            let exe = detect::detect_windows_sync().executable;
            start_windows_with(&runner, &runner, &spawner, move || exe.clone(), &closure_target, |ms| std::thread::sleep(std::time::Duration::from_millis(ms)))
        } else {
            start_wsl_with(&runner, &runner, &runner, &spawner, &closure_target, |ms| std::thread::sleep(std::time::Duration::from_millis(ms)))
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
            // Cleanup atomico best-effort (niente shell): HOME, poi pkill
            // con PATH nativo via `env`. Errori ignorati (si sta uscendo).
            let runner = proc::SystemRunner;
            if let Ok(home) = detect::wsl_home_dir(&runner, distro) {
                let path = detect::wsl_native_path(&home);
                let mut args = detect::wsl_args_after(distro);
                args.extend(["env".into(), format!("PATH={path}"), "pkill".into(), "-f".into(), "dsh.web".into()]);
                let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                use crate::proc::CommandRunner;
                let _ = runner.run_capture("wsl.exe", &arg_refs, Duration::from_secs(15));
            }
        }
    }
}

fn main() {
    tauri::Builder::default()
        // Updater in background: il check all'avvio e differito dal frontend
        // (nessun costo sincrono qui); il plugin serve solo su azione utente.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(Procs {
            inner: Mutex::new(proc::ProcRegistry::default()),
        })
        .manage(DetectCache { at: Mutex::new(None) })
        .manage(WslBootCache { inner: Mutex::new(std::collections::HashMap::new()) })
        .invoke_handler(tauri::generate_handler![
            detect_windows,
            list_wsl_distros,
            scan_boot,
            probe_wsl,
            probe_wsl_fast,
            list_node_runtimes,
            is_port_open,
            find_free_port,
            start_env,
            stop_env,
            layout_tabs,
            open_in_browser,
            run_update,
            read_env_log,
            diagnose_wsl
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
