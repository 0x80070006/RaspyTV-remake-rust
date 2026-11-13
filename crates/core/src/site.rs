//! `site.json` : adresses du réseau et position météo, hors du dépôt.

use serde::{Deserialize, Serialize};

use crate::paths::Paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Site {
    pub jellyfin_url: String,
    /// Autres noms sous lesquels le même serveur Jellyfin répond (nom Tailscale…).
    pub jellyfin_extra_hosts: Vec<String>,
    pub moonlight_host: String,
    pub dashboard_url: String,
    pub immich_url: String,
    pub weather_latitude: f64,
    pub weather_longitude: f64,
    pub weather_label: String,
    pub timezone: String,
}

impl Default for Site {
    fn default() -> Self {
        Self {
            jellyfin_url: "http://jellyfin.local:8096".into(),
            jellyfin_extra_hosts: Vec::new(),
            moonlight_host: String::new(),
            dashboard_url: String::new(),
            immich_url: String::new(),
            weather_latitude: 48.8566,
            weather_longitude: 2.3522,
            weather_label: String::new(),
            timezone: "Europe/Paris".into(),
        }
    }
}

impl Site {
    /// Lit `site.json` de la version Rust, sinon celui de la version Electron.
    pub fn load(paths: &Paths) -> Self {
        for file in [paths.site_file(), paths.legacy_site_file()] {
            if let Ok(text) = std::fs::read_to_string(&file) {
                match serde_json::from_str::<Site>(&text) {
                    Ok(site) => return site,
                    Err(error) => crate::rlog!("site.json illisible ({}) : {error}", file.display()),
                }
            }
        }
        Self::default()
    }

    /// Origine du serveur Jellyfin (`http://hôte:port`), sans chemin.
    pub fn jellyfin_origin(&self) -> String {
        url::Url::parse(&self.jellyfin_url).map(|u| u.origin().ascii_serialization()).unwrap_or_else(|_| Site::default().jellyfin_url)
    }

    /// Noms d'hôte acceptés pour les cookies Jellyfin.
    pub fn jellyfin_hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = url::Url::parse(&self.jellyfin_url).ok().and_then(|u| u.host_str().map(str::to_lowercase)).into_iter().collect();
        hosts.extend(self.jellyfin_extra_hosts.iter().map(|h| h.to_lowercase()));
        hosts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_site_keeps_defaults() {
        let site: Site = serde_json::from_str(r#"{"jellyfinUrl":"http://media.example:8096/web/"}"#).unwrap();
        assert_eq!(site.jellyfin_origin(), "http://media.example:8096");
        assert_eq!(site.timezone, "Europe/Paris");
        assert_eq!(site.jellyfin_hosts(), vec!["media.example".to_string()]);
    }
}
