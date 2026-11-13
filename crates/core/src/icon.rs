//! Icône d'un raccourci : `<link rel=icon>` de la page, sinon le service de favicons de Google.

use std::path::Path;
use std::time::Duration;

use rand::RngCore;

use crate::config::valid_url;

/// Premier `href` d'un `<link rel="…icon…">` trouvé dans le HTML.
pub fn find_icon_href(html: &str) -> Option<String> {
    let link = regex::Regex::new(r#"(?is)<link[^>]+rel=["'][^"']*(?:icon|apple-touch-icon)[^"']*["'][^>]*>"#).ok()?;
    let href = regex::Regex::new(r#"(?is)href=["']([^"']+)["']"#).ok()?;
    let found = link.find_iter(html).find_map(|m| href.captures(m.as_str()).map(|c| c[1].to_string()));
    found
}

pub fn extension_for(mime: &str) -> &'static str {
    if mime.contains("svg") {
        "svg"
    } else if mime.contains("png") {
        "png"
    } else if mime.contains("webp") {
        "webp"
    } else {
        "ico"
    }
}

/// Télécharge l'icône dans `icon_dir` et renvoie son URL `file://`.
pub fn fetch(target: &str, icon_dir: &Path) -> Option<String> {
    if !valid_url(target) {
        return None;
    }
    let page = url::Url::parse(target).ok()?;
    let fallback = format!("https://www.google.com/s2/favicons?domain={}&sz=128", page.host_str()?);
    let agent = crate::http::agent(Duration::from_secs(6));
    let html =
        agent.get(page.as_str()).call().ok().and_then(|mut r| r.body_mut().with_config().limit(2 * 1024 * 1024).read_to_string().ok()).unwrap_or_default();
    let source = find_icon_href(&html).and_then(|href| page.join(&href).ok()).map(|u| u.to_string()).filter(|u| valid_url(u)).unwrap_or(fallback);
    let (mime, bytes) = crate::http::get_bytes(&agent, &source, &[], 1024 * 1024)?;
    let mut id = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut id);
    let name: String = id.iter().map(|b| format!("{b:02x}")).collect();
    let file = icon_dir.join(format!("{name}.{}", extension_for(&mime)));
    crate::fsutil::atomic_write(&file, &bytes).ok()?;
    Some(format!("file://{}", file.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_icon_links() {
        let html = r#"<head><link rel="stylesheet" href="a.css"><LINK REL="shortcut icon" HREF="/favicon.png"></head>"#;
        assert_eq!(super::find_icon_href(html).as_deref(), Some("/favicon.png"));
        assert_eq!(super::find_icon_href("<p>rien</p>"), None);
        assert_eq!(super::extension_for("image/svg+xml"), "svg");
    }
}
