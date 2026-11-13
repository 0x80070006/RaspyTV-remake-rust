//! Emplacements des fichiers de RaspyTV.
//!
//! La version Rust range ses données dans `~/.config/raspytv-rs/` pour cohabiter
//! avec la version Electron (`~/.config/raspy-tv/`), dont elle relit seulement
//! `site.json` si elle n'a pas encore le sien.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    /// Données de la version Rust.
    pub data: PathBuf,
    /// Données de la version Electron (lecture seule, pour `site.json`).
    pub legacy: PathBuf,
    /// Dossier d'exécution (`$XDG_RUNTIME_DIR`) : sockets et drapeaux.
    pub runtime: PathBuf,
    /// Ressources web (accueil WASM, CSS, icônes, scripts injectés).
    pub web: PathBuf,
}

fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        if let Some(dir) = std::env::var_os("APPDATA") {
            return PathBuf::from(dir);
        }
    }
    home().join(".config")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    #[cfg(unix)]
    {
        // SAFETY: getuid n'a pas de précondition.
        let uid = unsafe { libc_getuid() };
        let dir = PathBuf::from(format!("/run/user/{uid}"));
        if dir.is_dir() {
            return dir;
        }
    }
    std::env::temp_dir()
}

#[cfg(unix)]
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// Ressources web : `RASPYTV_WEB`, sinon `web/` à côté de l'exécutable,
/// sinon `/usr/share/raspytv/web`, sinon `web/` du dossier courant (développement).
fn web_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RASPYTV_WEB") {
        return PathBuf::from(dir);
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("web"));
            // target/debug/raspytv → racine du dépôt
            if let Some(root) = parent.parent().and_then(Path::parent) {
                candidates.push(root.join("web"));
            }
        }
    }
    candidates.push(PathBuf::from("/usr/share/raspytv/web"));
    candidates.push(PathBuf::from("web"));
    candidates.into_iter().find(|dir| dir.join("index.html").is_file()).unwrap_or_else(|| PathBuf::from("web"))
}

impl Paths {
    pub fn from_env() -> Self {
        let config = config_home();
        let data = std::env::var_os("RASPYTV_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| config.join("raspytv-rs"));
        Self { data, legacy: config.join("raspy-tv"), runtime: runtime_dir(), web: web_dir() }
    }

    pub fn config_file(&self) -> PathBuf {
        self.data.join("config.json")
    }
    pub fn site_file(&self) -> PathBuf {
        self.data.join("site.json")
    }
    pub fn legacy_site_file(&self) -> PathBuf {
        self.legacy.join("site.json")
    }
    pub fn icons(&self) -> PathBuf {
        self.data.join("icons")
    }
    pub fn logs(&self) -> PathBuf {
        self.data.join("logs")
    }
    pub fn nav_state(&self) -> PathBuf {
        self.data.join("nav-state.json")
    }
    pub fn panel_state(&self) -> PathBuf {
        self.data.join("panel-state.json")
    }
    pub fn daily_hero(&self) -> PathBuf {
        self.data.join("daily-hero.json")
    }
    pub fn vault(&self) -> PathBuf {
        self.data.join("jellyfin-session.bin")
    }
    pub fn vault_key(&self) -> PathBuf {
        self.data.join("vault.key")
    }
    pub fn cache_stamps(&self) -> PathBuf {
        self.data.join("cache-maintenance.json")
    }
    pub fn weather_cache(&self) -> PathBuf {
        self.data.join("weather.json")
    }
    pub fn youtube_import(&self) -> PathBuf {
        self.data.join("import").join("youtube-cookies.txt")
    }
    /// Dossier de données web d'un service (cookies, stockage local, cache).
    pub fn web_data(&self, partition: &str) -> PathBuf {
        self.data.join("web").join(partition)
    }
    /// Tableaux du mode Tableau : d'abord `~/.config/raspytv-rs/art`, puis `web/assets/art`.
    pub fn art_dirs(&self) -> [PathBuf; 2] {
        [self.data.join("art"), self.web.join("assets").join("art")]
    }
    /// Fonds d'écran animés ou statiques personnels.
    pub fn background_dirs(&self) -> [PathBuf; 2] {
        [self.data.join("backgrounds"), self.web.join("assets").join("backgrounds")]
    }
    pub fn nav_socket(&self, prefix: &str) -> PathBuf {
        self.runtime.join(format!("{prefix}-nav.sock"))
    }
    pub fn panel_flag(&self, prefix: &str) -> PathBuf {
        self.runtime.join(format!("{prefix}-panel.open"))
    }

    pub fn ensure(&self) {
        for dir in [self.data.clone(), self.icons(), self.logs()] {
            crate::fsutil::ensure_private_dir(&dir);
        }
    }
}
