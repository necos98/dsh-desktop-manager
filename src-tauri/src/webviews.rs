//! Layout webview figlie (SRP): solo geometria + label/url.
//! Il codice Tauri resta nel comando layout_tabs; qui vive la logica pura.

use crate::model::{child_webview_label, LayoutInput};

/// Geometria contenuto sotto la tabbar (px interi). Pura.
pub fn content_geometry(tabbar: f64, width: f64, height: f64) -> (i32, i32, i32) {
    let tabbar_px = tabbar.round() as i32;
    let width_px = width.round() as i32;
    let content_h = ((height - tabbar).max(100.0)).round() as i32;
    (tabbar_px, width_px, content_h)
}

/// Label webview da creare (una per tab in esecuzione). Pura.
pub fn labels_to_ensure(input: &LayoutInput) -> Vec<String> {
    input.tabs.iter().map(|t| child_webview_label(&t.env_id)).collect()
}

/// Label della tab attiva (None = schermata impostazioni). Pura.
pub fn active_label(input: &LayoutInput) -> Option<String> {
    input.active_env.as_ref().map(|id| child_webview_label(id))
}

/// URL desiderato per una label, se ancora tra le tab. Pura.
pub fn wanted_url(input: &LayoutInput, label: &str) -> Option<String> {
    input
        .tabs
        .iter()
        .find(|t| child_webview_label(&t.env_id) == label)
        .map(|t| t.url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LayoutInput;
    #[test]
    fn geometry_rounds_and_clamps_height() {
        let (tab, w, h) = content_geometry(44.4, 1200.6, 800.2);
        assert_eq!((tab, w, h), (44, 1201, 756));
        let (_, _, h2) = content_geometry(44.0, 800.0, 50.0);
        assert_eq!(h2, 100);
    }
    #[test]
    fn labels_and_active() {
        let input = LayoutInput { tabbar: 44.0, width: 1.0, height: 1.0, active_env: Some("wsl:Ubuntu".into()), tabs: vec![crate::model::TabSpec { env_id: "wsl:Ubuntu".into(), url: "http://127.0.0.1:3100".into() }] };
        assert_eq!(labels_to_ensure(&input), vec!["web-wsl-Ubuntu"]);
        assert_eq!(active_label(&input).as_deref(), Some("web-wsl-Ubuntu"));
        assert_eq!(wanted_url(&input, "web-wsl-Ubuntu").as_deref(), Some("http://127.0.0.1:3100"));
        assert!(wanted_url(&input, "web-altro").is_none());
    }
    #[test]
    fn no_active_when_settings() {
        let input = LayoutInput { tabbar: 44.0, width: 1.0, height: 1.0, active_env: None, tabs: vec![] };
        assert!(active_label(&input).is_none());
        assert!(labels_to_ensure(&input).is_empty());
    }
}

