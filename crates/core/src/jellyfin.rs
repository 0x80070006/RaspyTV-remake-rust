//! Jellyfin : session conservée, bandeau, « Continuer », « Récemment ajoutés », lecture.
//!
//! Les images passent par le proxy `/jf/…` du shell : la page d'accueil ne voit
//! jamais le jeton.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http;

pub const CLIENT_VERSION: &str = "2.3.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub token: String,
    pub user_id: String,
    #[serde(default)]
    pub server_id: String,
    #[serde(default)]
    pub server_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeroItem {
    pub id: String,
    pub name: String,
    pub overview: String,
    pub year: String,
    pub rating: String,
    pub runtime: Option<u32>,
    pub backdrop: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaCard {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub progress: f64,
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct HomeData {
    pub items: Vec<HeroItem>,
    pub continue_items: Vec<MediaCard>,
    pub recent_items: Vec<MediaCard>,
    pub needs_login: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn authorization(token: &str) -> String {
    format!(r#"MediaBrowser Client="RaspyTV", Device="Raspberry Pi", DeviceId="raspytv-home", Version="{CLIENT_VERSION}", Token="{token}""#)
}

fn clean_token(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric() || "._~-".contains(*c)).collect()
}

fn clean_hex(value: &str) -> String {
    value.chars().filter(char::is_ascii_hexdigit).collect()
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn pick<'a>(object: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key).filter(|v| !v.is_null()))
}

fn parse_storage(raw: Option<&str>) -> Option<Value> {
    serde_json::from_str::<Value>(raw?).ok().and_then(|v| match v {
        // Le stockage peut contenir une chaîne JSON encodée deux fois.
        Value::String(inner) => serde_json::from_str(&inner).ok(),
        other => Some(other),
    })
}

/// Sessions candidates trouvées dans le stockage local de Jellyfin Web
/// (`jellyfin_credentials`) ou de l'ancienne page cinématique.
pub fn candidates_from_storage(cinematic: Option<&str>, jellyfin: Option<&str>) -> Vec<Session> {
    let mut list: Vec<Session> = Vec::new();
    let mut add = |token: Option<&Value>, user: Option<&Value>, server_id: Option<&Value>, server_name: Option<&Value>| {
        let token = clean_token(&text(token));
        let user_id = clean_hex(&text(user));
        if !token.is_empty() && !user_id.is_empty() && !list.iter().any(|s| s.token == token && s.user_id == user_id) {
            list.push(Session { token, user_id, server_id: text(server_id), server_name: text(server_name) });
        }
    };
    if let Some(own) = parse_storage(cinematic) {
        add(
            pick(&own, &["token", "accessToken", "AccessToken"]),
            pick(&own, &["userId", "UserId"]),
            pick(&own, &["serverId", "ServerId"]),
            pick(&own, &["serverName", "ServerName"]),
        );
    }
    if let Some(credentials) = parse_storage(jellyfin) {
        let servers = pick(&credentials, &["Servers", "servers"]).and_then(Value::as_array).cloned().unwrap_or_default();
        for server in &servers {
            add(
                pick(server, &["AccessToken", "accessToken"]),
                pick(server, &["UserId", "userId"]),
                pick(server, &["Id", "id"]),
                pick(server, &["Name", "name"]),
            );
        }
        add(pick(&credentials, &["AccessToken", "accessToken"]), pick(&credentials, &["UserId", "userId"]), None, None);
    }
    list
}

/// Script exécuté dans la vue Jellyfin pour relire son stockage local.
pub const CAPTURE_SCRIPT: &str =
    "JSON.stringify({cinematic:localStorage.getItem('cinematic_credentials'),jellyfin:localStorage.getItem('jellyfin_credentials')})";

/// Script qui réinstalle la session dans le stockage local, avant de charger Jellyfin Web.
pub fn restore_script(origin: &str, local_origin: &str, session: &Session) -> String {
    let now = jiff::Timestamp::now().as_millisecond();
    let server = json!({"DateLastAccessed": now, "LastConnectionMode": 2, "ManualAddress": origin, "manualAddressOnly": false,
        "LocalAddress": local_origin, "Address": origin, "Name": session.server_name, "Id": session.server_id,
        "UserId": session.user_id, "AccessToken": session.token});
    let credentials = json!({"Servers": [server]}).to_string();
    let cinematic = json!({"token": session.token, "userId": session.user_id, "serverId": session.server_id, "serverName": session.server_name}).to_string();
    format!(
        "(()=>{{localStorage.setItem('jellyfin_credentials',{});localStorage.setItem('cinematic_credentials',{});return true}})()",
        Value::String(credentials),
        Value::String(cinematic)
    )
}

/// Ouvre la fiche puis appuie sur Lecture (Jellyfin n'a pas de route de lecture directe ;
/// le bouton gère aussi la reprise).
pub fn play_script(item_id: &str, server_id: &str) -> Option<String> {
    if !(16..=64).contains(&item_id.len()) || !item_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let server = if server_id.is_empty() { String::new() } else { format!("&serverId={}", clean_hex(server_id)) };
    let route = Value::String(format!("#/details?id={item_id}{server}"));
    Some(format!("(()=>{{location.hash={route};const route=location.hash,started=Date.now();const attempt=()=>{{if(location.hash!==route)return;const button=[...document.querySelectorAll('.itemDetailPage:not(.hide) .btnPlay')].find(item=>item.offsetParent&&!item.classList.contains('hide'));if(button){{button.click();return}}if(Date.now()-started>15000)return;setTimeout(attempt,200)}};setTimeout(attempt,500)}})()"))
}

/// Échap dans Jellyfin : ferme d'abord le lecteur natif. Renvoie `true` si un lecteur a été fermé.
pub const ESCAPE_SCRIPT: &str =
    "(()=>{if(!location.hash.startsWith('#/video')&&!document.querySelector('video.htmlvideoplayer'))return false;history.back();return true})()";

pub struct Client {
    origin: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(origin: &str) -> Self {
        Self { origin: origin.trim_end_matches('/').to_string(), agent: http::agent(Duration::from_secs(8)) }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    fn headers(token: &str) -> [(&'static str, String); 2] {
        [("X-Emby-Token", token.to_string()), ("Authorization", authorization(token))]
    }

    fn get(&self, route: &str, token: Option<&str>) -> Option<Value> {
        let headers = token.map(Self::headers);
        let list: Vec<(&str, &str)> = headers.iter().flatten().map(|(k, v)| (*k, v.as_str())).collect();
        http::get_json(&self.agent, &format!("{}{route}", self.origin), &list)
    }

    /// Vérifie qu'une session est acceptée ; complète l'identifiant et le nom du serveur.
    pub fn validate(&self, candidate: &Session) -> Option<Session> {
        if candidate.token.is_empty() || candidate.user_id.is_empty() {
            return None;
        }
        self.get(&format!("/Users/{}", candidate.user_id), Some(&candidate.token))?;
        let info = self.get("/System/Info/Public", None).unwrap_or(Value::Null);
        let mut valid = candidate.clone();
        if valid.server_id.is_empty() {
            valid.server_id = text(info.get("Id"));
        }
        if valid.server_name.is_empty() {
            valid.server_name = info.get("ServerName").and_then(Value::as_str).unwrap_or("Jellyfin").to_string();
        }
        Some(valid)
    }

    fn first(&self, routes: &[String], token: &str) -> Option<Value> {
        routes.iter().find_map(|route| self.get(route, Some(token)))
    }

    /// Bandeau et lignes de l'accueil.
    pub fn home(&self, session: &Session) -> Result<HomeData, String> {
        let user = &session.user_id;
        let query = format!("UserId={user}");
        let fields = "Overview,ProductionYear,RunTimeTicks,CommunityRating,UserData,ParentIndexNumber,IndexNumber,SeriesName";
        let images = "&ImageTypeLimit=1&EnableImageTypes=Backdrop,Primary";
        let movies_query = format!("Recursive=true&IncludeItemTypes=Movie&SortBy=SortName&SortOrder=Ascending&Limit=200&Fields={fields}{images}");
        let movies = self
            .first(&[format!("/Items?{query}&{movies_query}"), format!("/Users/{user}/Items?{movies_query}")], &session.token)
            .ok_or("médiathèque inaccessible")?;
        let resume = self.first(
            &[
                format!("/UserItems/Resume?{query}&MediaTypes=Video&Limit=12&Fields={fields}{images},Thumb"),
                format!("/Users/{user}/Items/Resume?MediaTypes=Video&Limit=12&Fields={fields}{images},Thumb"),
            ],
            &session.token,
        );
        let added = self.get(
            &format!("/Items?{query}&Recursive=true&IncludeItemTypes=Movie&SortBy=DateCreated,SortName&SortOrder=Descending&Limit=20&Fields={fields}{images}"),
            Some(&session.token),
        );
        Ok(map_home(&movies, resume.as_ref(), added.as_ref()))
    }

    /// Image demandée par l'accueil via le proxy `/jf/…`.
    pub fn image(&self, path_and_query: &str, token: &str) -> Option<(String, Vec<u8>)> {
        if !is_image_route(path_and_query) {
            return None;
        }
        let headers = Self::headers(token);
        let list: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        http::get_bytes(&self.agent, &format!("{}{path_and_query}", self.origin), &list, 6 * 1024 * 1024)
    }
}

/// Seules les routes d'images produites par [`image_path`] passent par le proxy.
pub fn is_image_route(route: &str) -> bool {
    let Some(rest) = route.strip_prefix("/Items/") else { return false };
    let Some((id, rest)) = rest.split_once("/Images/") else { return false };
    let Some((kind, query)) = rest.split_once('?') else { return false };
    let Some(width) = query.strip_prefix("maxWidth=").and_then(|q| q.strip_suffix("&quality=76")) else { return false };
    (16..=64).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_hexdigit())
        && matches!(kind, "Backdrop/0" | "Primary")
        && !width.is_empty()
        && width.len() <= 4
        && width.chars().all(|c| c.is_ascii_digit())
}

/// Chemin servi par le shell (`/jf` + route Jellyfin).
pub fn image_path(id: &str, kind: &str, width: u32) -> String {
    let suffix = if kind == "Backdrop" { "/0" } else { "" };
    format!("/jf/Items/{id}/Images/{kind}{suffix}?maxWidth={width}&quality=76")
}

fn items(value: Option<&Value>) -> Vec<Value> {
    value.and_then(|v| v.get("Items")).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn has_backdrop(item: &Value) -> bool {
    item.get("BackdropImageTags").and_then(Value::as_array).is_some_and(|a| !a.is_empty())
}

fn progress(item: &Value) -> f64 {
    item.pointer("/UserData/PlayedPercentage").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 100.0)
}

fn card_image(item: &Value, id: &str) -> String {
    if has_backdrop(item) {
        image_path(id, "Backdrop", 640)
    } else if item.pointer("/ImageTags/Primary").is_some() {
        image_path(id, "Primary", 500)
    } else {
        String::new()
    }
}

fn year_or_film(item: &Value) -> String {
    let year = text(item.get("ProductionYear"));
    if year.is_empty() {
        "Film".into()
    } else {
        year
    }
}

pub fn map_home(movies: &Value, resume: Option<&Value>, added: Option<&Value>) -> HomeData {
    let hero = items(Some(movies))
        .iter()
        .filter(|item| has_backdrop(item))
        .map(|item| {
            let id = clean_hex(&text(item.get("Id")));
            HeroItem {
                name: text(item.get("Name")),
                overview: text(item.get("Overview")),
                year: text(item.get("ProductionYear")),
                rating: item.get("CommunityRating").and_then(Value::as_f64).filter(|r| *r > 0.0).map(|r| format!("{r:.1}")).unwrap_or_default(),
                runtime: item.get("RunTimeTicks").and_then(Value::as_f64).filter(|t| *t > 0.0).map(|t| (t / 600_000_000.0).round() as u32),
                backdrop: image_path(&id, "Backdrop", 1400),
                id,
            }
        })
        .filter(|item| !item.id.is_empty())
        .collect();
    let continue_items = items(resume)
        .iter()
        .take(8)
        .map(|item| {
            let id = clean_hex(&text(item.get("Id")));
            let subtitle = if item.get("Type").and_then(Value::as_str) == Some("Episode") {
                let episode = format!(
                    "S{} E{}",
                    item.get("ParentIndexNumber").and_then(Value::as_u64).unwrap_or(0),
                    item.get("IndexNumber").and_then(Value::as_u64).unwrap_or(0)
                );
                [text(item.get("SeriesName")), episode].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
            } else {
                year_or_film(item)
            };
            MediaCard { name: text(item.get("Name")), subtitle, progress: progress(item), image: card_image(item, &id), id }
        })
        .collect();
    let recent_items = items(added)
        .iter()
        .map(|item| {
            let id = clean_hex(&text(item.get("Id")));
            MediaCard { name: text(item.get("Name")), subtitle: year_or_film(item), progress: progress(item), image: card_image(item, &id), id }
        })
        .collect();
    HomeData { items: hero, continue_items, recent_items, needs_login: false, error: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn candidates_are_sanitized_and_deduplicated() {
        let jf = json!({"Servers": [{"AccessToken": "abc\"def", "UserId": "AB-12", "Id": "srv", "Name": "Maison"}], "AccessToken": "abcdef", "UserId": "ab12"})
            .to_string();
        let list = candidates_from_storage(None, Some(&jf));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].token, "abcdef");
        assert_eq!(list[0].user_id, "AB12");
        assert_eq!(list[0].server_name, "Maison");
    }

    #[test]
    fn double_encoded_storage_is_read() {
        let own = Value::String(json!({"token": "t1", "userId": "ff"}).to_string()).to_string();
        let list = candidates_from_storage(Some(&own), None);
        assert_eq!(list, vec![Session { token: "t1".into(), user_id: "ff".into(), ..Session::default() }]);
    }

    #[test]
    fn image_routes_are_strict() {
        let route = image_path(ID, "Backdrop", 1400);
        assert!(is_image_route(route.strip_prefix("/jf").unwrap()));
        assert!(is_image_route(&format!("/Items/{ID}/Images/Primary?maxWidth=500&quality=76")));
        assert!(!is_image_route(&format!("/Items/{ID}/Images/Primary?maxWidth=500&quality=76&api_key=x")));
        assert!(!is_image_route("/Users/abc"));
        assert!(!is_image_route(&format!("/Items/{ID}/../System/Images/Primary?maxWidth=5&quality=76")));
    }

    #[test]
    fn home_mapping() {
        let movies = json!({"Items": [
            {"Id": ID, "Name": "Orbite", "Overview": "x", "ProductionYear": 2024, "RunTimeTicks": 79_200_000_000u64, "CommunityRating": 8.14, "BackdropImageTags": ["a"]},
            {"Id": "ffffffffffffffff", "Name": "Sans fond", "BackdropImageTags": []}
        ]});
        let resume = json!({"Items": [{"Id": ID, "Name": "Ép", "Type": "Episode", "SeriesName": "Série", "ParentIndexNumber": 1, "IndexNumber": 4, "UserData": {"PlayedPercentage": 140.0}, "ImageTags": {"Primary": "p"}}]});
        let data = map_home(&movies, Some(&resume), None);
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].runtime, Some(132));
        assert_eq!(data.items[0].rating, "8.1");
        assert_eq!(data.continue_items[0].subtitle, "Série · S1 E4");
        assert_eq!(data.continue_items[0].progress, 100.0);
        assert!(data.continue_items[0].image.contains("/Images/Primary"));
    }

    #[test]
    fn play_script_rejects_bad_ids() {
        assert!(play_script("../../x", "").is_none());
        assert!(play_script(ID, "srv1").unwrap().contains("#/details?id="));
    }

    #[test]
    fn restore_script_quotes_values() {
        let session = Session { token: "t'\"".into(), user_id: "ab".into(), server_id: String::new(), server_name: "J".into() };
        let script = restore_script("http://a:8096", "http://a:8096", &session);
        assert!(script.starts_with("(()=>{localStorage.setItem('jellyfin_credentials',\""));
    }
}
