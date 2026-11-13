//! Cœur de RaspyTV : tout ce qui ne dépend ni du moteur web ni de GTK.
//!
//! Chaque module est testable sous Windows comme sous Linux. Les commandes du
//! système (`pactl`, `bluetoothctl`, `moonlight-qt`, `systemctl`…) passent par
//! [`sys::run`], qui renvoie une erreur propre quand l'outil est absent.

pub mod apps;
pub mod audio;
pub mod config;
pub mod cookies;
pub mod fsutil;
pub mod hero;
pub mod http;
pub mod icon;
pub mod jellyfin;
pub mod log;
pub mod media;
pub mod moonlight;
pub mod network;
pub mod paths;
pub mod protocol;
pub mod site;
pub mod stats;
pub mod sys;
pub mod vault;
pub mod weather;

/// Version affichée dans les paramètres.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
