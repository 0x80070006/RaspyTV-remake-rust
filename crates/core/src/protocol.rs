//! Commandes JSON reçues sur `$XDG_RUNTIME_DIR/raspytv-nav.sock` (manette TelecomHand,
//! scripts) et produites par la barre latérale / le panneau. Même format que la
//! version Electron : `{"action": "…", …}`.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Home,
    Settings,
    Launch(String),
    Moonlight,
    MoonlightGame(String),
    /// Clavier système (squeekboard) pendant Moonlight.
    Keyboard,
    Volume(f64),
    VolumeSet(f64),
    AudioOutput(String),
    Media {
        op: String,
        value: f64,
    },
    SystemPower(String),
    NavPeek(bool),
    TvHome,
    TvSettings,
    TvBack,
    TvPlayPause,
    TvContext,
    TvNavPeek,
    TvPanel,
    TvPanelState(bool),
    TvState,
    NoutubeImportSession,
}

fn string(value: &Value, key: &str, max: usize) -> Option<String> {
    value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= max).map(String::from)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    let n = value.get(key)?;
    n.as_f64().or_else(|| n.as_str().and_then(|s| s.parse().ok())).filter(|v| v.is_finite())
}

/// Valide une commande ; tout ce qui n'est pas reconnu est ignoré.
pub fn parse(value: &Value) -> Option<Command> {
    let action = value.get("action")?.as_str()?;
    Some(match action {
        "home" => Command::Home,
        "settings" => Command::Settings,
        "launch" => Command::Launch(string(value, "id", 200)?),
        "moonlight" => Command::Moonlight,
        "moonlight-game" => Command::MoonlightGame(string(value, "name", 200)?),
        "keyboard" => Command::Keyboard,
        "volume" => Command::Volume(number(value, "delta")?),
        "volume-set" => Command::VolumeSet(number(value, "value")?),
        "audio-output" => Command::AudioOutput(string(value, "id", 300)?),
        "media" => {
            let op = string(value, "op", 16)?;
            if !["toggle", "next", "previous", "seek"].contains(&op.as_str()) {
                return None;
            }
            Command::Media { op, value: number(value, "value").unwrap_or(0.0) }
        }
        "system-power" => {
            let op = string(value, "op", 16)?;
            if op != "reboot" && op != "poweroff" {
                return None;
            }
            Command::SystemPower(op)
        }
        "nav-peek" => Command::NavPeek(value.get("open") == Some(&Value::Bool(true))),
        "tv-home" => Command::TvHome,
        "tv-settings" => Command::TvSettings,
        "tv-back" => Command::TvBack,
        "tv-play-pause" => Command::TvPlayPause,
        "tv-context" => Command::TvContext,
        "tv-nav-peek" => Command::TvNavPeek,
        "tv-panel" => Command::TvPanel,
        "tv-panel-state" => Command::TvPanelState(value.get("open") == Some(&Value::Bool(true))),
        "tv-state" => Command::TvState,
        "noutube-import-session" => Command::NoutubeImportSession,
        _ => return None,
    })
}

/// Application telle que la barre latérale l'affiche.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NavApp {
    pub id: String,
    pub name: String,
    pub native: bool,
    /// Chemin local de l'icône (vide : icône générique).
    pub icon: String,
    pub color: String,
}

/// État publié pour la barre latérale (et `nav-state.json`, lu par d'autres outils).
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LayerState {
    pub active_id: Option<String>,
    pub native_mode: bool,
    pub art_mode: bool,
    pub system_volume: Option<u8>,
    pub system_muted: bool,
    pub apps: Vec<NavApp>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub app: String,
    pub title: String,
    pub artist: String,
    /// Jaquette mise en cache localement.
    pub artwork: String,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PanelOutput {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub kind: String,
    pub headset: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PanelAudio {
    pub volume: u8,
    pub muted: bool,
    pub outputs: Vec<PanelOutput>,
    /// (id `bt:MAC`, nom)
    pub bluetooth: Vec<(String, String)>,
}

/// État du panneau PS : lecture en cours et son.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct PanelState {
    pub media: Option<MediaInfo>,
    pub audio: PanelAudio,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commands_are_validated() {
        assert_eq!(parse(&json!({"action": "launch", "id": "twitch"})), Some(Command::Launch("twitch".into())));
        assert_eq!(parse(&json!({"action": "volume", "delta": "5"})), Some(Command::Volume(5.0)));
        assert_eq!(parse(&json!({"action": "system-power", "op": "halt"})), None);
        assert_eq!(parse(&json!({"action": "media", "op": "seek", "value": 12.5})), Some(Command::Media { op: "seek".into(), value: 12.5 }));
        assert_eq!(parse(&json!({"action": "tv-panel-state", "open": "true"})), Some(Command::TvPanelState(false)));
        assert_eq!(parse(&json!({"action": "rm -rf"})), None);
        assert_eq!(parse(&json!({"action": "launch"})), None);
    }
}
