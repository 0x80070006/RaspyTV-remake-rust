//! `config.json` : réglages de l'interface, raccourcis et ordre des tuiles.
//!
//! La lecture est tolérante : un champ invalide reprend sa valeur par défaut au
//! lieu de faire échouer tout le fichier.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::apps::App;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Shortcut {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_description")]
    pub description: String,
}

fn default_color() -> String {
    "#4f8cff".into()
}
fn default_description() -> String {
    "Raccourci personnalisé".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub theme: String,
    pub background: String,
    pub clock24: bool,
    pub sounds: bool,
    pub sound_volume: u8,
    pub reduced_motion: bool,
    pub show_weather: bool,
    pub show_clock: bool,
    pub show_recent: bool,
    pub max_recent: u8,
    pub max_resident_apps: u8,
    pub nav_delay: u32,
    pub app_order: Vec<String>,
    pub shortcuts: Vec<Shortcut>,
    pub recent: Vec<String>,
    pub restore_apps: bool,
}

impl Config {
    pub fn defaults(apps: &[App]) -> Self {
        Self {
            theme: "dark".into(),
            background: "color:#000000".into(),
            clock24: true,
            sounds: true,
            sound_volume: 45,
            reduced_motion: false,
            show_weather: true,
            show_clock: true,
            show_recent: true,
            max_recent: 4,
            max_resident_apps: 4,
            nav_delay: 850,
            app_order: apps.iter().map(|a| a.id.clone()).collect(),
            shortcuts: Vec::new(),
            recent: Vec::new(),
            restore_apps: false,
        }
    }

    /// Équivalent de `safeConfig` : chaque champ est vérifié séparément.
    pub fn from_value(raw: &Value, apps: &[App]) -> Self {
        let d = Self::defaults(apps);
        let b = |key: &str, default: bool| raw.get(key).and_then(Value::as_bool).unwrap_or(default);
        let s = |key: &str, default: &str| raw.get(key).and_then(Value::as_str).unwrap_or(default).to_string();
        let n = |key: &str| raw.get(key).and_then(Value::as_f64);
        let theme = s("theme", &d.theme);
        let resident = n("maxResidentApps").filter(|v| v.fract() == 0.0).map(|v| v.clamp(2.0, 6.0) as u8).unwrap_or(d.max_resident_apps);
        let strings =
            |key: &str| -> Option<Vec<String>> { raw.get(key)?.as_array().map(|list| list.iter().filter_map(|x| x.as_str().map(String::from)).collect()) };
        let shortcuts = raw
            .get("shortcuts")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|item| serde_json::from_value::<Shortcut>(item.clone()).ok()).filter(valid_shortcut).collect())
            .unwrap_or_default();
        Self {
            theme: if ["dark", "light", "auto"].contains(&theme.as_str()) { theme } else { d.theme },
            background: s("background", &d.background),
            clock24: b("clock24", d.clock24),
            sounds: b("sounds", d.sounds),
            sound_volume: n("soundVolume").map(|v| v.clamp(0.0, 100.0) as u8).unwrap_or(d.sound_volume),
            reduced_motion: b("reducedMotion", d.reduced_motion),
            show_weather: b("showWeather", d.show_weather),
            show_clock: b("showClock", d.show_clock),
            show_recent: b("showRecent", d.show_recent),
            max_recent: n("maxRecent").map(|v| v.clamp(0.0, 12.0) as u8).unwrap_or(d.max_recent),
            max_resident_apps: resident,
            nav_delay: n("navDelay").map(|v| v.clamp(0.0, 5000.0) as u32).unwrap_or(d.nav_delay),
            app_order: with_new_default_apps(strings("appOrder").unwrap_or(d.app_order), apps),
            shortcuts,
            recent: strings("recent").unwrap_or_default().into_iter().take(12).collect(),
            restore_apps: b("restoreApps", d.restore_apps),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    /// Applique un correctif venu de l'interface (`settings:save`), puis revalide tout.
    pub fn patched(&self, patch: &Value, apps: &[App]) -> Self {
        let mut merged = self.to_value();
        if let (Some(target), Some(source)) = (merged.as_object_mut(), patch.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        Self::from_value(&merged, apps)
    }

    pub fn load(file: &Path, apps: &[App]) -> Self {
        match std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            Some(raw) => {
                crate::fsutil::restrict(file);
                Self::from_value(&raw, apps)
            }
            None => {
                crate::rlog!("config recovery: valeurs par défaut");
                let defaults = Self::defaults(apps);
                let _ = defaults.save(file);
                defaults
            }
        }
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        crate::fsutil::atomic_write(file, text.as_bytes())
    }

    pub fn touch_recent(&mut self, id: &str) {
        self.recent.retain(|x| x != id);
        self.recent.insert(0, id.to_string());
        self.recent.truncate(12);
    }
}

/// Une application native ajoutée par une mise à jour se range juste après sa
/// voisine d'origine, pas en fin de liste.
pub fn with_new_default_apps(order: Vec<String>, defaults: &[App]) -> Vec<String> {
    let mut result = order;
    for (index, app) in defaults.iter().enumerate() {
        if result.contains(&app.id) {
            continue;
        }
        let previous = defaults[..index].iter().rev().find(|item| result.contains(&item.id));
        let position = previous.and_then(|p| result.iter().position(|x| *x == p.id)).map(|i| i + 1).unwrap_or(0);
        result.insert(position, app.id.clone());
    }
    result
}

/// URL http(s) avec un hôte et sans identifiants.
pub fn valid_url(value: &str) -> bool {
    match url::Url::parse(value) {
        Ok(u) => matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.is_empty()) && u.username().is_empty() && u.password().is_none(),
        Err(_) => false,
    }
}

pub fn valid_shortcut(item: &Shortcut) -> bool {
    !item.id.is_empty() && !item.name.is_empty() && item.name.chars().count() < 61 && valid_url(&item.url)
}

/// Icône acceptée : fichier du dossier d'icônes, ressource intégrée ou URL.
pub fn valid_icon(value: &str, icon_dir: &Path) -> bool {
    if value.len() > 2048 {
        return false;
    }
    if let Some(file) = value.strip_prefix("file://") {
        let file = Path::new(file);
        return !file.components().any(|c| matches!(c, std::path::Component::ParentDir)) && file.starts_with(icon_dir);
    }
    if let Some(name) = value.strip_prefix("assets/icons/") {
        return !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) && !name.contains("..");
    }
    valid_url(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::default_apps;
    use crate::site::Site;

    fn apps() -> Vec<App> {
        default_apps(&Site { immich_url: "https://photos.example".into(), ..Site::default() })
    }

    #[test]
    fn invalid_fields_fall_back() {
        let raw = json!({"theme": "neon", "maxResidentApps": 12, "sounds": "yes", "shortcuts": [
            {"id": "custom-1", "name": "Arte", "url": "https://www.arte.tv"},
            {"id": "custom-2", "name": "Bad", "url": "javascript:alert(1)"}
        ]});
        let config = Config::from_value(&raw, &apps());
        assert_eq!(config.theme, "dark");
        assert_eq!(config.max_resident_apps, 6);
        assert!(config.sounds);
        assert_eq!(config.shortcuts.len(), 1);
        assert_eq!(config.shortcuts[0].color, "#4f8cff");
    }

    #[test]
    fn new_native_app_goes_after_its_neighbour() {
        let order = vec!["twitch".to_string(), "jellyfin".into(), "youtube".into()];
        let merged = with_new_default_apps(order, &apps());
        // Immich suit Twitch dans le catalogue : il se range donc juste après lui.
        assert_eq!(merged, ["twitch", "immich", "jellyfin", "youtube", "youtube-music"]);
    }

    #[test]
    fn urls_and_icons_are_checked() {
        assert!(valid_url("https://example.org/a"));
        assert!(!valid_url("https://user:pw@example.org"));
        assert!(!valid_url("file:///etc/passwd"));
        let dir = Path::new("/home/tv/.config/raspytv-rs/icons");
        assert!(valid_icon("assets/icons/home.png", dir));
        assert!(!valid_icon("assets/icons/../../secret", dir));
        assert!(valid_icon("file:///home/tv/.config/raspytv-rs/icons/a.png", dir));
        assert!(!valid_icon("file:///home/tv/.config/raspytv-rs/icons/../config.json", dir));
    }

    #[test]
    fn patch_is_revalidated() {
        let config = Config::defaults(&apps());
        let next = config.patched(&json!({"maxResidentApps": 1, "clock24": false}), &apps());
        assert_eq!(next.max_resident_apps, 2);
        assert!(!next.clock24);
    }
}
