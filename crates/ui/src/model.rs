//! Données reçues du shell (mêmes formes JSON que `raspytv-core`).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct UiConfig {
    pub theme: String,
    pub background: String,
    pub clock24: bool,
    pub sounds: bool,
    pub sound_volume: u8,
    pub reduced_motion: bool,
    pub show_weather: bool,
    pub show_clock: bool,
    pub show_recent: bool,
    pub max_resident_apps: u8,
    pub app_order: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct UiApp {
    pub id: String,
    pub name: String,
    pub url: String,
    pub icon: String,
    pub color: String,
    pub native: bool,
    pub open: bool,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SiteInfo {
    pub weather_label: String,
    pub moonlight: bool,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub config: UiConfig,
    pub apps: Vec<UiApp>,
    pub active_id: Option<String>,
    pub native_mode: bool,
    pub art_mode: bool,
    pub site: SiteInfo,
}

impl State {
    /// Applications dans l'ordre choisi, puis celles qui n'y figurent pas encore.
    pub fn ordered_apps(&self) -> Vec<&UiApp> {
        let mut list: Vec<&UiApp> = self.config.app_order.iter().filter_map(|id| self.apps.iter().find(|a| &a.id == id)).collect();
        list.extend(self.apps.iter().filter(|a| !self.config.app_order.contains(&a.id)));
        list
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct HeroItem {
    pub id: String,
    pub name: String,
    pub overview: String,
    pub year: String,
    pub rating: String,
    pub runtime: Option<u32>,
    pub backdrop: String,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaCard {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub progress: f64,
    pub image: String,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct HomeData {
    pub items: Vec<HeroItem>,
    pub continue_items: Vec<MediaCard>,
    pub recent_items: Vec<MediaCard>,
    pub needs_login: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Weather {
    pub temperature: i64,
    pub code: i64,
    pub min: i64,
    pub max: i64,
    pub offline: bool,
    pub unavailable: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Network {
    pub online: bool,
    pub lan: bool,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Game {
    pub name: String,
    pub boxart: String,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Games {
    pub games: Vec<Game>,
    pub needs_pairing: bool,
    pub unavailable: bool,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Sink {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Bluetooth {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Audio {
    pub sinks: Vec<Sink>,
    pub volume: u8,
    pub unavailable: bool,
    pub bluetooth: Vec<Bluetooth>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Stats {
    pub hostname: String,
    pub cpu: String,
    pub ram_used: u64,
    pub ram_total: u64,
    pub temperature: Option<f64>,
    pub disk_free: Option<String>,
    pub resident_apps: usize,
    pub resident_limit: u8,
    pub version: String,
    pub engine: String,
}

/// Fond d'écran proposé dans les paramètres.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Background {
    pub id: String,
    pub name: String,
    /// `video` ou `image`
    pub kind: String,
    pub url: String,
}
