//! Protocole `raspy://` de l'accueil.
//!
//! - fichiers de `web/` (HTML, CSS, WASM, icônes) ;
//! - `/icons/…` : icônes téléchargées des raccourcis ;
//! - `/art/…`, `/bg/…` : tableaux et fonds personnels ;
//! - `/jf/…` : images Jellyfin, demandées par le shell avec le jeton (jamais exposé à la page).

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use raspytv_core::jellyfin::{Client, Session};
use wry::http::{header, Request, Response, StatusCode};

pub const SCHEME: &str = "raspy";

/// Adresse de l'accueil selon la plate-forme (WebView2 traduit les protocoles en `http://<nom>.localhost`).
pub fn home_url(page: &str) -> String {
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/{page}")
    } else {
        format!("{SCHEME}://localhost/{page}")
    }
}

pub fn is_home_url(url: &str) -> bool {
    url.starts_with(&home_url(""))
}

#[derive(Clone)]
pub struct Shared {
    pub web: PathBuf,
    pub icons: PathBuf,
    pub art: Vec<PathBuf>,
    pub backgrounds: Vec<PathBuf>,
    pub jellyfin_origin: String,
    pub session: Arc<RwLock<Option<Session>>>,
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "ttf" => "font/ttf",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Chemin relatif sûr : pas de `..`, pas de chemin absolu.
pub fn safe_join(base: &Path, relative: &str) -> Option<PathBuf> {
    let decoded = percent_decode(relative);
    let relative = Path::new(decoded.trim_start_matches('/'));
    if relative.components().all(|c| matches!(c, Component::Normal(_))) {
        Some(base.join(relative))
    } else {
        None
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn respond(status: StatusCode, mime: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache")
        .header("Content-Security-Policy", "default-src 'self' data:; script-src 'self' 'wasm-unsafe-eval' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: file:; media-src 'self'")
        .body(Cow::<'static, [u8]>::Owned(body))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"" as &[u8])))
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    respond(StatusCode::NOT_FOUND, "text/plain", b"introuvable".to_vec())
}

fn file(path: Option<PathBuf>) -> Response<Cow<'static, [u8]>> {
    match path.and_then(|p| std::fs::read(&p).ok().map(|bytes| (mime(&p), bytes))) {
        Some((mime, bytes)) => respond(StatusCode::OK, mime, bytes),
        None => not_found(),
    }
}

fn first_existing(bases: &[PathBuf], name: &str) -> Option<PathBuf> {
    bases.iter().filter_map(|b| safe_join(b, name)).find(|p| p.is_file())
}

pub fn handle(shared: &Shared, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    let path = uri.path().to_string();
    if let Some(route) = path.strip_prefix("/jf") {
        let route = match uri.query() {
            Some(query) => format!("{route}?{query}"),
            None => route.to_string(),
        };
        let token = shared.session.read().ok().and_then(|s| s.as_ref().map(|s| s.token.clone()));
        let Some(token) = token else { return not_found() };
        return match Client::new(&shared.jellyfin_origin).image(&route, &token) {
            Some((mime, bytes)) if mime.starts_with("image/") => {
                let mime: &'static str = if mime.contains("png") {
                    "image/png"
                } else if mime.contains("webp") {
                    "image/webp"
                } else {
                    "image/jpeg"
                };
                respond(StatusCode::OK, mime, bytes)
            }
            _ => not_found(),
        };
    }
    if let Some(name) = path.strip_prefix("/icons/") {
        return file(safe_join(&shared.icons, name));
    }
    if let Some(name) = path.strip_prefix("/art/") {
        return file(first_existing(&shared.art, name));
    }
    if let Some(name) = path.strip_prefix("/bg/") {
        return file(first_existing(&shared.backgrounds, name));
    }
    let name = if path == "/" || path.is_empty() { "index.html" } else { path.as_str() };
    file(safe_join(&shared.web, name))
}

/// Noms lisibles des fonds de la version Electron.
fn background_name(stem: &str) -> String {
    let known = [
        ("appa-avatar", "Appa Avatar"),
        ("black-hole", "Trou noir"),
        ("cozy-bedroom-at-night", "Chambre cosy"),
        ("cyberpunk-bedroom", "Chambre cyberpunk"),
        ("cyberpunk-night-city", "Ville cyberpunk"),
        ("dipper-fireflies", "Lucioles"),
        ("frutiger-aero-office", "Frutiger Aero"),
        ("hacker-typer", "Hacker"),
        ("pandora", "Pandora"),
        ("sleepy-rainy-evening", "Soirée pluvieuse"),
        ("twilight-at-mount-fuji", "Mont Fuji"),
        ("cinema-wall", "Mur cinéma"),
    ];
    let base = stem.split('.').next().unwrap_or(stem);
    known.iter().find(|(k, _)| *k == base).map(|(_, v)| v.to_string()).unwrap_or_else(|| base.replace(['-', '_'], " "))
}

fn list_files(dirs: &[PathBuf], extensions: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .filter(|n| extensions.iter().any(|ext| n.to_lowercase().ends_with(ext)))
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn backgrounds(shared: &Shared) -> serde_json::Value {
    let list: Vec<serde_json::Value> = list_files(&shared.backgrounds, &[".mp4", ".webm", ".png", ".jpg", ".jpeg", ".webp"])
        .into_iter()
        .map(|name| {
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name).to_string();
            let video = name.ends_with(".mp4") || name.ends_with(".webm");
            serde_json::json!({"id": stem, "name": background_name(&stem), "kind": if video { "video" } else { "image" }, "url": format!("/bg/{name}")})
        })
        .collect();
    serde_json::Value::Array(list)
}

pub fn art(shared: &Shared) -> serde_json::Value {
    serde_json::Value::Array(list_files(&shared.art, &[".png", ".jpg", ".jpeg", ".webp"]).into_iter().map(|n| format!("/art/{n}").into()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_cannot_escape() {
        let base = Path::new("/srv/web");
        assert_eq!(safe_join(base, "/assets/a.png"), Some(base.join("assets/a.png")));
        assert_eq!(safe_join(base, "/../secret"), None);
        assert_eq!(safe_join(base, "/%2e%2e/secret"), None);
        assert_eq!(safe_join(base, "/a%20b.png"), Some(base.join("a b.png")));
    }

    #[test]
    fn background_names() {
        assert_eq!(background_name("black-hole.3840x2160"), "Trou noir");
        assert_eq!(background_name("my-beach"), "my beach");
    }
}
