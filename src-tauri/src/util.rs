//! Utilita pure (SRP): parsing/decodifica senza side-effect.
//! Cambiano solo se cambia il formato degli input (wsl.exe, dsh --version).

use crate::model::WslDistro;

/// Decodifica output di processi Windows/WSL: gestisce UTF-16LE (wsl.exe) e UTF-8.
pub fn decode_output(bytes: Vec<u8>) -> String {
    fn utf16(bytes: &[u8]) -> String {
        let start = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE { 2 } else { 0 };
        let units: Vec<u16> = bytes[start..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    }
    if bytes.len() >= 2 {
        if bytes[0] == 0xFF && bytes[1] == 0xFE {
            return utf16(&bytes);
        }
        // euristica UTF-16LE senza BOM: molti zero nelle posizioni dispari
        let sample = bytes.len().min(256);
        let zeros = (1..sample).step_by(2).filter(|&i| bytes[i] == 0).count();
        if zeros as f32 > sample as f32 * 0.4 {
            return utf16(&bytes);
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// Parsa l'output testuale di `wsl.exe -l -v` in lista di distro.
/// Funzione pura (nessun processo spawnato) per poterla unit-testare:
/// salta header NAME e righe di separatori, individua la keyword di stato
/// (Running, Stopped, ...) e ricongiunge i token precedenti come nome,
/// rimuovendo il marcatore "* " della distro predefinita.
pub fn parse_wsl_distros(out: &str) -> Vec<WslDistro> {
    let mut result = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty()
            || t.starts_with("NAME")
            || t.chars().all(|c| c == '-' || c == ' ')
        {
            continue;
        }
        let tokens: Vec<&str> = t.split_whitespace().collect();
        if let Some(pos) = tokens.iter().position(|w| {
            matches!(*w, "Running" | "Stopped" | "Installing" | "Uninstalling" | "Converting" | "Stopping")
        }) {
            // wsl.exe -l -v marca la distro predefinita con "* " davanti al nome:
            // va rimosso, altrimenti probe/start/update falliscono con
            // WSL_E_DISTRO_NOT_FOUND (wsl exit -1).
            let raw_name = tokens[..pos].join(" ");
            let name = raw_name.trim_start_matches(['*', ' ']).trim().to_string();
            let state = tokens[pos].to_string();
            let wsl_version = tokens.get(pos + 1).map(|s| s.to_string());
            if !name.is_empty() {
                result.push(WslDistro { name, state, wsl_version });
            }
        }
    }
    result
}

/// Primo token del testo che assomiglia a semver (es. "0.1.0" anche in
/// "dsh version 0.1.0"). Pura e unit-testata.
pub fn first_semver(text: &str) -> Option<String> {
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        for tok in t.split_whitespace() {
            let clean = tok.trim_matches(|c| matches!(c, 'v' | 'V' | ',' | ';' | '"' | '\'' | '(' | ')'));
            if clean.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && clean.contains('.') {
                return Some(clean.to_string());
            }
        }
    }
    None
}

/// Estrae l'URL autenticato stampato da `dsh web` su stdout
/// (riga "dsh web: http://127.0.0.1:<porta>/?token=...").
/// Restituisce il primo URL con `token=`; l'eventuale URL LAN tra parentesi viene ignorato.
/// Pura e unit-testata.
pub fn extract_auth_url(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(pos) = line.find("dsh web:") {
            let rest = line[pos + "dsh web:".len()..].trim_start();
            if let Some(url) = rest.split_whitespace().next() {
                let url = url.trim_end_matches(|c| matches!(c, ')' | '.' | ',' | ';' | '"'));
                if (url.starts_with("http://") || url.starts_with("https://")) && url.contains("token=") {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Costruisce un buffer UTF-16LE (con o senza BOM) da una stringa.
    fn utf16le_bytes(s: &str, bom: bool) -> Vec<u8> {
        let mut v = Vec::new();
        if bom {
            v.extend_from_slice(&[0xFF, 0xFE]);
        }
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v
    }

    #[test]
    fn decodes_utf8_plain() {
        assert_eq!(decode_output(b"hello".to_vec()), "hello");
    }

    #[test]
    fn decodes_utf16le_with_bom() {
        let bytes = utf16le_bytes("Ubuntu  Running  2", true);
        assert!(decode_output(bytes).contains("Ubuntu"));
    }

    #[test]
    fn decodes_utf16le_without_bom_by_heuristic() {
        let bytes = utf16le_bytes("Debian  Stopped  2", false);
        assert!(decode_output(bytes).contains("Debian"));
    }

    #[test]
    fn parses_wsl_list_basic() {
        let out = "  NAME                   STATE           VERSION\n* Ubuntu                 Running         2\n  Debian                 Stopped         2\n";
        assert_eq!(
            parse_wsl_distros(out),
            vec![
                WslDistro { name: "Ubuntu".into(), state: "Running".into(), wsl_version: Some("2".into()) },
                WslDistro { name: "Debian".into(), state: "Stopped".into(), wsl_version: Some("2".into()) },
            ]
        );
    }

    #[test]
    fn parses_wsl_list_multiword_and_separators() {
        let out = "NAME  STATE  VERSION\n--- --- ---\n* My Custom Distro  Running  2\n";
        assert_eq!(
            parse_wsl_distros(out),
            vec![WslDistro { name: "My Custom Distro".into(), state: "Running".into(), wsl_version: Some("2".into()) }]
        );
    }

    #[test]
    fn parses_wsl_list_skips_garbage() {
        assert!(parse_wsl_distros("").is_empty());
        assert!(parse_wsl_distros("NAME STATE VERSION\n").is_empty());
    }

    #[test]
    fn parses_wsl_list_all_states() {
        let out = "NAME S V\nA Installing 2\nB Uninstalling 2\nC Converting 2\nD Stopping 2\n";
        let names: Vec<String> = parse_wsl_distros(out).into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn first_semver_picks_first_dotted_token() {
        assert_eq!(first_semver("dsh version 0.1.0\nok"), Some("0.1.0".to_string()));
        assert_eq!(first_semver("no version here"), None);
        assert_eq!(first_semver(""), None);
    }

    #[test]
    fn first_semver_strips_v_prefix_and_quotes() {
        assert_eq!(first_semver("v1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(first_semver("(2.0.0),"), Some("2.0.0".to_string()));
    }

    #[test]
    fn extract_auth_url_finds_token_url() {
        let text = "qualcosa\ndsh web: http://127.0.0.1:3080/?token=abc123\naltro";
        assert_eq!(
            extract_auth_url(text),
            Some("http://127.0.0.1:3080/?token=abc123".to_string())
        );
    }

    #[test]
    fn extract_auth_url_ignores_plain_url() {
        assert_eq!(extract_auth_url("dsh web: http://127.0.0.1:3080/\n"), None);
        assert_eq!(extract_auth_url("niente qui"), None);
    }

    #[test]
    fn extract_auth_url_trims_trailing_punct() {
        let text = "dsh web: http://127.0.0.1:3080/?token=abc).";
        assert_eq!(extract_auth_url(text), Some("http://127.0.0.1:3080/?token=abc".to_string()));
    }
}
