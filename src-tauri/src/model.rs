//! Modello di dominio condiviso (SRP: solo tipi serializzati).
//! Speculare ai tipi frontend in `src/types.ts` (rename camelCase).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EnvProbe {
    pub kind: String,
    pub name: String,
    pub distro: Option<String>,
    pub installed: bool,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub dsh_home: Option<String>,
    pub error: Option<String>,
    /// npm rilevato nell'ambiente (None = non verificato). Serve per
    /// avvisare l'utente quando npm manca: in quel caso
    /// installazione/aggiornamento sono impossibili e tocca all'utente
    /// installarlo (il manager non installa mai toolchain da solo).
    pub has_npm: Option<bool>,
}

impl EnvProbe {
    /// Costruttore "non installato" (usato nel fallback errori + test).
    #[allow(dead_code)]
    pub fn not_installed(kind: &str, name: &str, distro: Option<String>) -> Self {
        Self {
            kind: kind.to_string(),
            name: name.to_string(),
            distro,
            installed: false,
            version: None,
            executable: None,
            dsh_home: None,
            error: None,
            has_npm: None,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WslDistro {
    pub name: String,
    pub state: String,
    pub wsl_version: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EnvTarget {
    pub kind: String, // "windows" | "wsl"
    pub name: String,
    pub distro: Option<String>,
    pub port: u16,
    #[serde(default)]
    pub extra_args: Vec<String>,
    pub workspace: Option<String>,
    /// Runtime Node scelto dall'utente (solo WSL): dir bin (es.
    /// `/home/u/.nvm/versions/node/v24.20.0/bin`) oppure versione nvm
    /// (es. `v24.20.0`). None = automatico (prima dir che risolve).
    /// Se punta a qualcosa di sparito -> errore esplicito, mai fallback
    /// silenzioso (niente lotteria).
    #[serde(default)]
    pub node_runtime: Option<String>,
}

/// Un runtime Node rilevato nella distro (comando `list_node_runtimes`).
/// `id` e cio che la UI salva in `node_runtime`: la dir bin.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeRuntime {
    /// Dir bin (es. `/home/u/.nvm/versions/node/v24.20.0/bin`).
    pub id: String,
    /// Etichetta UI (es. `nvm v24.20.0 (default)`).
    pub label: String,
    /// Versione node (`node --version`), se leggibile.
    pub node_version: Option<String>,
    /// True se e il default nvm (alias) o l'unico di sistema.
    pub is_default: bool,
    /// Origine: `nvm`, `system`.
    pub source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    pub ok: bool,
    pub message: String,
    pub pid: Option<u32>,
    pub port: u16,
    pub reached: bool,
    pub auth_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub ok: bool,
    pub exit_code: i32,
    pub output: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TabSpec {
    pub env_id: String,
    pub url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutInput {
    pub tabbar: f64,
    pub width: f64,
    pub height: f64,
    pub active_env: Option<String>,
    #[serde(default)]
    pub tabs: Vec<TabSpec>,
}

/// Esito della diagnostica WSL mostrato nel pannello "Diagnostica" della UI.
/// Mai un errore lanciato: l'eventuale fallimento fatale finisce in `error`,
/// cosi il frontend puo sempre disegnare la checklist passo-passo.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WslDiag {
    pub distro: String,
    /// Stato da `wsl -l -v` (Running/Stopped/...), None se non elencabile.
    pub state: Option<String>,
    pub dsh_installed: bool,
    pub dsh_version: Option<String>,
    pub has_npm: bool,
    /// Porta aperta vista da dentro la distro (/dev/tcp): distingue
    /// "server giu" da "server su ma irraggiungibile da Windows (rete WSL2)".
    pub port_open_in_distro: Option<bool>,
    /// Porta aperta vista da Windows (stessa sonda di is_port_open).
    pub port_open_from_windows: bool,
    /// Ultime righe del log di distro (None se illeggibile/assente).
    pub log_tail: Option<String>,
    pub error: Option<String>,
}

/// Chiave ambiente: "windows" oppure "wsl:<distro>" (usata per Procs e webview).
/// Pura e unit-testata: cambia solo se cambia lo schema delle chiavi.
pub fn env_key(t: &EnvTarget) -> String {
    if t.kind == "windows" {
        "windows".to_string()
    } else {
        format!("wsl:{}", t.distro.as_deref().unwrap_or("?"))
    }
}

/// Label della webview figlia che mostra la GUI di DSH per un ambiente.
/// Pura e unit-testata.
pub fn child_webview_label(env_key: &str) -> String {
    let slug: String = env_key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    format!("web-{slug}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(kind: &str, distro: Option<&str>) -> EnvTarget {
        EnvTarget { kind: kind.into(), name: kind.into(), distro: distro.map(|s| s.into()), port: 3080, extra_args: vec![], workspace: None, node_runtime: None }
    }

    #[test]
    fn env_key_windows_vs_wsl() {
        assert_eq!(env_key(&target("windows", None)), "windows");
        assert_eq!(env_key(&target("wsl", Some("Ubuntu"))), "wsl:Ubuntu");
        assert_eq!(env_key(&target("wsl", None)), "wsl:?");
    }

    #[test]
    fn child_label_slugs_env_keys() {
        assert_eq!(child_webview_label("windows"), "web-windows");
        assert_eq!(child_webview_label("wsl:Ubuntu"), "web-wsl-Ubuntu");
        assert_eq!(child_webview_label("wsl:My Distro!"), "web-wsl-My-Distro-");
    }

    #[test]
    fn not_installed_builds_empty_probe() {
        let p = EnvProbe::not_installed("windows", "Windows", None);
        assert!(!p.installed);
        assert_eq!(p.kind, "windows");
        assert!(p.version.is_none());
        assert!(p.has_npm.is_none());
    }
}
