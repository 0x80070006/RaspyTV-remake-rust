//! Catalogue des applications : services natifs et raccourcis personnels.

use serde::{Deserialize, Serialize};

use crate::config::{Config, Shortcut};
use crate::site::Site;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct App {
    pub id: String,
    pub name: String,
    pub url: String,
    pub icon: String,
    pub color: String,
    pub description: String,
    /// Dossier de données partagé (NouTube et NouTube Music partagent la session Google).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition: Option<String>,
    /// Reçoit le script NouTube (publicités, Shorts, SponsorBlock).
    #[serde(default)]
    pub noutube: bool,
    /// Continue de jouer quand on la quitte (NouTube Music).
    #[serde(default)]
    pub background: bool,
    /// Produit du son : met en pause la musique d'arrière-plan à l'ouverture.
    #[serde(default)]
    pub media: bool,
    /// Application fournie par RaspyTV (et non raccourci personnel).
    #[serde(default)]
    pub native: bool,
}

impl App {
    pub fn partition(&self) -> &str {
        self.partition.as_deref().unwrap_or(&self.id)
    }
}

#[allow(clippy::too_many_arguments)]
fn native(id: &str, name: &str, url: String, icon: &str, color: &str, description: &str, media: bool) -> App {
    App {
        id: id.into(),
        name: name.into(),
        url,
        icon: icon.into(),
        color: color.into(),
        description: description.into(),
        partition: None,
        noutube: false,
        background: false,
        media,
        native: true,
    }
}

/// Services fournis par RaspyTV. Une URL vide dans `site.json` masque la tuile.
pub fn default_apps(site: &Site) -> Vec<App> {
    let jellyfin = site.jellyfin_origin();
    let mut youtube = native("youtube", "NouTube", "https://www.youtube.com/".into(), "assets/icons/noutube.png", "#ff244a", "YouTube sans publicité", true);
    youtube.partition = Some("youtube".into());
    youtube.noutube = true;
    let mut music = native(
        "youtube-music",
        "NouTube Music",
        "https://music.youtube.com/".into(),
        "assets/icons/noutube.png",
        "#ff2d55",
        "YouTube Music sans publicité",
        true,
    );
    music.partition = Some("youtube".into());
    music.noutube = true;
    music.background = true;
    vec![
        native("jellyfin", "Jellyfin", format!("{jellyfin}/web/#/home"), "assets/icons/jellyfin.svg", "#a855f7", "Ta médiathèque", true),
        youtube,
        music,
        native("twitch", "Twitch", "https://www.twitch.tv/".into(), "assets/icons/twitch.svg", "#a970ff", "Live et gaming", true),
        native("homelab", "Dashboard", site.dashboard_url.clone(), "assets/icons/dashy.png", "#00e5d0", "Dashboard du lab", false),
        native("immich", "Immich", site.immich_url.clone(), "assets/icons/immich.png", "#fa2921", "Photos et vidéos", true),
    ]
    .into_iter()
    .filter(|app| !app.url.is_empty())
    .collect()
}

/// Un raccourci ne reçoit que ses champs : il ne peut pas se donner les droits
/// d'une application native (NouTube, arrière-plan).
pub fn shortcut_app(shortcut: &Shortcut) -> App {
    App {
        id: shortcut.id.clone(),
        name: shortcut.name.clone(),
        url: shortcut.url.clone(),
        icon: shortcut.icon.clone().unwrap_or_default(),
        color: shortcut.color.clone(),
        description: shortcut.description.clone(),
        partition: None,
        noutube: false,
        background: false,
        media: false,
        native: false,
    }
}

pub fn all_apps(defaults: &[App], config: &Config) -> Vec<App> {
    defaults.iter().cloned().chain(config.shortcuts.iter().map(shortcut_app)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_urls_hide_tiles() {
        let site = Site { dashboard_url: String::new(), immich_url: "https://photos.example".into(), ..Site::default() };
        let ids: Vec<_> = default_apps(&site).into_iter().map(|a| a.id).collect();
        assert_eq!(ids, ["jellyfin", "youtube", "youtube-music", "twitch", "immich"]);
    }

    #[test]
    fn noutube_apps_share_one_partition() {
        let apps = default_apps(&Site::default());
        let yt = apps.iter().find(|a| a.id == "youtube").unwrap();
        let music = apps.iter().find(|a| a.id == "youtube-music").unwrap();
        assert_eq!(yt.partition(), music.partition());
        assert!(music.background && !yt.background);
    }
}
