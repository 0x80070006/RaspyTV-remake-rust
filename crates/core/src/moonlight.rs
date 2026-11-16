//! Moonlight : liste des jeux (`moonlight-qt list --csv`) et préparation du kiosque labwc.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

pub const BIN: &str = "/usr/bin/moonlight-qt";

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub name: String,
    pub id: String,
    pub hdr: bool,
    pub boxart: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Games {
    pub games: Vec<Game>,
    pub needs_pairing: bool,
    pub unavailable: bool,
}

pub fn parse_csv_line(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                value.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => values.push(std::mem::take(&mut value).trim().to_string()),
            _ => value.push(c),
        }
    }
    values.push(value.trim().to_string());
    values
}

/// Interprète la sortie de `moonlight-qt list --csv` (colonne 5 : jeu masqué).
pub fn parse_games(stdout: &str, error: Option<&str>) -> Games {
    let games: Vec<Game> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .skip(1)
        .map(parse_csv_line)
        .filter(|row| !row[0].is_empty() && row.get(4).map(String::as_str) != Some("true"))
        .map(|row| {
            let boxart = row.get(6).cloned().unwrap_or_default();
            let boxart = if ["file:", "http:", "https:"].iter().any(|p| boxart.starts_with(p)) { boxart } else { String::new() };
            Game { name: row[0].clone(), id: row.get(1).cloned().unwrap_or_default(), hdr: row.get(2).map(String::as_str) == Some("true"), boxart }
        })
        .collect();
    if !games.is_empty() {
        return Games { games, ..Games::default() };
    }
    match error {
        Some(message) => {
            let lower = message.to_lowercase();
            let pairing = lower.contains("not been paired") || lower.contains("pair before");
            Games { games, needs_pairing: pairing, unavailable: !pairing }
        }
        None => Games::default(),
    }
}

/// Dossier des jaquettes de Moonlight (`~/.cache/Moonlight Game Streaming Project/Moonlight/boxart`).
pub fn boxart_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::home().join(".cache"))
        .join("Moonlight Game Streaming Project")
        .join("Moonlight")
        .join("boxart")
}

/// `file://…/boxart/<hôte>/<id>.png` → `/boxart/<hôte>/<id>.png` ; les URL http(s) restent telles quelles,
/// et un fichier hors du dossier des jaquettes est ignoré.
pub fn boxart_route(url: &str, dir: &Path) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    let Some(file) = url.strip_prefix("file://") else { return String::new() };
    match Path::new(file).strip_prefix(dir) {
        Ok(relative) if relative.components().all(|c| matches!(c, std::path::Component::Normal(_))) => {
            format!("/boxart/{}", relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
        }
        _ => String::new(),
    }
}

pub fn list_games(host: &str) -> Games {
    if host.is_empty() {
        return Games { unavailable: true, ..Games::default() };
    }
    match crate::sys::run_env(BIN, &["list", "--csv", host], Duration::from_secs(5), &[("QT_QPA_PLATFORM", "offscreen")]) {
        Ok(output) => {
            let error = (!output.success()).then(|| output.error());
            let games = parse_games(&output.stdout, error.as_deref());
            if games.games.is_empty() && error.is_some() {
                crate::rlog!("moonlight list unavailable: {}", error.unwrap_or_default().lines().next().unwrap_or(""));
            }
            games
        }
        Err(error) => {
            crate::rlog!("moonlight list unavailable: {error}");
            Games { unavailable: true, ..Games::default() }
        }
    }
}

/// Arguments du flux : fenêtré (maximisé par labwc), souris absolue, 1080p.
pub fn stream_args(host: &str, game: &str) -> Vec<String> {
    if game.is_empty() {
        return Vec::new();
    }
    ["--display-mode", "windowed", "--absolute-mouse", "--resolution", "1920x1080", "stream", host, game].iter().map(|s| s.to_string()).collect()
}

// ------------------------------------------------------------- préparation du kiosque

/// Moonlight.conf : fenêtre sans bordure, accélération de la souris, interface plein écran.
pub fn update_moonlight_conf(original: &str) -> String {
    let mut lines: Vec<String> = original.lines().map(|l| format!("{l}\n")).collect();
    let general = match lines.iter().position(|l| l.trim() == "[General]") {
        Some(i) => i,
        None => {
            lines.insert(0, "[General]\n".into());
            0
        }
    };
    let end = (general + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('[')).unwrap_or(lines.len());
    let values = [("windowmode", "2"), ("mouseacceleration", "true"), ("uidisplaymode", "1")];
    let mut section: Vec<String> =
        lines[general + 1..end].iter().filter(|line| !values.iter().any(|(key, _)| line.split('=').next().unwrap_or("").trim() == *key)).cloned().collect();
    section.extend(values.iter().map(|(k, v)| format!("{k}={v}\n")));
    lines.splice(general + 1..end, section);
    lines.concat()
}

const RULE_MARKER: &str = "<!-- RaspyTV Moonlight mouse-safe kiosk v2 -->";
const LEGACY_RULE: &str =
    r#"<windowRules><windowRule identifier="com.moonlight_stream.Moonlight" serverDecoration="no"><action name="Maximize" /></windowRule></windowRules>"#;

/// rc.xml de labwc : Moonlight sans décoration et maximisé. `None` si déjà à jour.
pub fn update_labwc_rc(xml: &str) -> Result<Option<String>, String> {
    roxmltree::Document::parse(xml).map_err(|e| format!("rc.xml invalide : {e}"))?;
    if xml.contains(RULE_MARKER) {
        return Ok(None);
    }
    let xml = xml.replace(LEGACY_RULE, "");
    let rule: String = std::iter::once(format!("\n{RULE_MARKER}\n"))
        .chain(
            ["com.moonlight_stream.Moonlight", "Moonlight"]
                .iter()
                .map(|id| format!(r#"<windowRule identifier="{id}" serverDecoration="no"><action name="Maximize" /></windowRule>"#) + "\n"),
        )
        .collect();
    let updated = if let Some(position) = xml.find("</windowRules>") {
        format!("{}{rule}{}", &xml[..position], &xml[position..])
    } else {
        let trimmed = xml.trim_end();
        let close = trimmed.rfind("</").ok_or("rc.xml sans élément racine")?;
        format!("{}<windowRules>{rule}</windowRules>\n{}\n", &trimmed[..close], &trimmed[close..])
    };
    roxmltree::Document::parse(&updated).map_err(|e| format!("rc.xml produit invalide : {e}"))?;
    Ok(Some(updated))
}

fn config_root() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".config"))
}

fn backup_once(source: &Path, target: &Path) {
    let backup = target.with_extension("raspytv-backup");
    if source.exists() && !backup.exists() {
        let _ = std::fs::copy(source, backup);
    }
}

/// Équivalent de `moonlight-kiosk-setup.py` : prépare les fichiers puis `labwc --reconfigure`.
pub fn prepare_kiosk() -> Result<(), String> {
    let root = config_root();
    let settings = root.join("Moonlight Game Streaming Project").join("Moonlight.conf");
    let original = std::fs::read_to_string(&settings).unwrap_or_else(|_| "[General]\n".into());
    let updated = update_moonlight_conf(&original);
    if updated != original {
        backup_once(&settings, &settings);
        crate::fsutil::atomic_write(&settings, updated.as_bytes()).map_err(|e| e.to_string())?;
    }
    let rc = root.join("labwc").join("rc.xml");
    let source = if rc.exists() { rc.clone() } else { PathBuf::from("/etc/xdg/labwc/rc.xml") };
    let xml = std::fs::read_to_string(&source).map_err(|e| format!("{}: {e}", source.display()))?;
    if let Some(updated) = update_labwc_rc(&xml)? {
        backup_once(&source, &rc);
        crate::fsutil::atomic_write(&rc, updated.as_bytes()).map_err(|e| e.to_string())?;
    }
    // Reconfiguration à chaque appel : une tentative précédente a pu échouer après l'écriture.
    let pid = std::env::var("LABWC_PID")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| crate::sys::quiet("pgrep", &["-x", "labwc"]).lines().next().unwrap_or_default().to_string());
    let output = crate::sys::run_env("labwc", &["--reconfigure"], Duration::from_secs(5), &[("LABWC_PID", &pid)])?;
    if output.success() {
        Ok(())
    } else {
        Err(output.error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxart_routes() {
        let dir = Path::new("/home/tv/.cache/Moonlight Game Streaming Project/Moonlight/boxart");
        assert_eq!(boxart_route("file:///home/tv/.cache/Moonlight Game Streaming Project/Moonlight/boxart/AB/12.png", dir), "/boxart/AB/12.png");
        assert_eq!(boxart_route("file:///etc/passwd", dir), "");
        assert_eq!(boxart_route("file:///home/tv/.cache/Moonlight Game Streaming Project/Moonlight/boxart/../x", dir), "");
        assert_eq!(boxart_route("https://img.example/a.png", dir), "https://img.example/a.png");
    }

    #[test]
    fn csv_with_quotes() {
        assert_eq!(parse_csv_line(r#""Jeu, le retour",12,"dit ""bonjour""""#), ["Jeu, le retour", "12", r#"dit "bonjour""#]);
    }

    #[test]
    fn games_and_errors() {
        let out =
            "Name,ID,HDR,Something,Hidden,X,Boxart\nDesktop,1,false,x,false,y,file:///a.png\nCaché,2,false,x,true,y,\nSteam,3,true,x,false,y,javascript:x\n";
        let games = parse_games(out, None);
        assert_eq!(games.games.len(), 2);
        assert_eq!(games.games[0].boxart, "file:///a.png");
        assert!(games.games[1].hdr && games.games[1].boxart.is_empty());
        assert!(parse_games("", Some("Computer has not been paired")).needs_pairing);
        assert!(parse_games("", Some("Unable to connect")).unavailable);
    }

    #[test]
    fn moonlight_conf_is_merged() {
        let conf = "[General]\nwindowmode=0\nfoo=bar\n[Other]\nwindowmode=9\n";
        let updated = update_moonlight_conf(conf);
        assert_eq!(updated, "[General]\nfoo=bar\nwindowmode=2\nmouseacceleration=true\nuidisplaymode=1\n[Other]\nwindowmode=9\n");
        assert_eq!(update_moonlight_conf(&updated), updated);
        assert!(update_moonlight_conf("").starts_with("[General]\n"));
    }

    #[test]
    fn labwc_rule_is_added_once() {
        let xml = "<labwc_config><core/></labwc_config>";
        let updated = update_labwc_rc(xml).unwrap().unwrap();
        assert!(updated.contains("<windowRules>") && updated.contains(RULE_MARKER));
        assert_eq!(update_labwc_rc(&updated).unwrap(), None);
        let with_rules = "<labwc_config><windowRules><windowRule identifier=\"x\"/></windowRules></labwc_config>";
        assert!(update_labwc_rc(with_rules).unwrap().unwrap().contains("identifier=\"x\""));
        assert!(update_labwc_rc("<pas fermé>").is_err());
    }
}
