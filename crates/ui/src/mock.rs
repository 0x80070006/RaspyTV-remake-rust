//! Données fictives de la démonstration (`web/index.html` ouvert hors de RaspyTV).
//! Rien ici ne contacte de serveur : les images sont des dégradés SVG générés.

use serde_json::{json, Value};

/// Dégradé SVG en `data:` URI, avec un titre facultatif.
pub fn art(a: &str, b: &str, c: &str, label: &str) -> String {
    let text = if label.is_empty() {
        String::new()
    } else {
        format!(
            r##"<text x="800" y="500" font-family="sans-serif" font-size="120" font-weight="700" fill="#fff" fill-opacity=".9" text-anchor="middle">{label}</text>"##
        )
    };
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1600 900"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{a}"/><stop offset=".55" stop-color="{b}"/><stop offset="1" stop-color="{c}"/></linearGradient><radialGradient id="r" cx=".72" cy=".38" r=".5"><stop offset="0" stop-color="#fff" stop-opacity=".35"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient></defs><rect width="1600" height="900" fill="url(#g)"/><circle cx="1150" cy="340" r="420" fill="url(#r)"/><path d="M0 720 Q400 600 800 700 T1600 640 V900 H0Z" fill="#000" opacity=".28"/><path d="M0 800 Q500 700 1000 790 T1600 760 V900 H0Z" fill="#000" opacity=".35"/>{text}</svg>"##
    );
    let mut encoded = String::from("data:image/svg+xml;charset=utf-8,");
    for byte in svg.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'() ".contains(&byte) {
            encoded.push(if byte == b' ' { ' ' } else { byte as char });
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded.replace(' ', "%20")
}

pub fn state(page_settings_open_apps: bool) -> Value {
    let apps = [
        ("jellyfin", "Jellyfin", "assets/icons/jellyfin.svg", "#a855f7", true),
        ("youtube", "NouTube", "assets/icons/noutube.png", "#ff244a", true),
        ("youtube-music", "NouTube Music", "assets/icons/noutube.png", "#ff2d55", true),
        ("twitch", "Twitch", "assets/icons/twitch.svg", "#a970ff", true),
        ("homelab", "Dashboard", "assets/icons/dashy.png", "#00e5d0", true),
        ("immich", "Immich", "assets/icons/immich.png", "#fa2921", true),
        ("custom-demo", "Ma radio", "", "#ff8a3d", false),
    ];
    let list: Vec<Value> = apps
        .iter()
        .map(|(id, name, icon, color, native)| {
            json!({"id": id, "name": name, "icon": icon, "color": color, "url": "https://example.com/", "native": native,
                   "open": page_settings_open_apps && (*id == "youtube-music" || *id == "jellyfin")})
        })
        .collect();
    json!({
        "config": {"theme": "dark", "background": "color:#000000", "clock24": true, "sounds": false, "soundVolume": 45,
                   "reducedMotion": false, "showWeather": true, "showClock": true, "showRecent": true, "maxResidentApps": 4,
                   "appOrder": apps.iter().map(|a| a.0).collect::<Vec<_>>()},
        "apps": list, "activeId": null, "nativeMode": false, "artMode": false,
        "site": {"weatherLabel": "Paris", "moonlight": true}
    })
}

pub fn hero() -> Value {
    let films = [
        (
            "Les Marées d’Ambre",
            "#3b1d4a",
            "#b4543a",
            "#f2b36b",
            "2023",
            118,
            "7.8",
            "Sur une île où la mer se retire chaque été, une cartographe découvre une ville engloutie que personne ne se souvient avoir quittée.",
        ),
        (
            "Orbite Basse",
            "#071c35",
            "#0a3d91",
            "#59c3ff",
            "2024",
            132,
            "8.1",
            "Coincés dans une station en perdition, cinq techniciens ont quatre-vingt-dix minutes pour choisir qui redescendra sur Terre.",
        ),
        (
            "Le Dernier Phare",
            "#0d3d2e",
            "#2a6e5a",
            "#d8f0c8",
            "2022",
            104,
            "7.2",
            "Un gardien de phare reçoit chaque nuit des signaux lumineux d’un navire qui a sombré il y a cent ans.",
        ),
        (
            "Néon Sud",
            "#551b2c",
            "#a3245c",
            "#ff8bd1",
            "2021",
            97,
            "6.9",
            "Une chauffeuse de nuit traverse une métropole écrasée de chaleur avec, sur la banquette arrière, un passager qui ne vieillit pas.",
        ),
        (
            "Plein Hiver",
            "#17191f",
            "#4a5a78",
            "#e6eefc",
            "2020",
            121,
            "7.5",
            "Dans un village isolé par la neige, les habitants découvrent que le printemps a été annulé par décret.",
        ),
    ];
    let items: Vec<Value> = films
        .iter()
        .enumerate()
        .map(|(i, (name, a, b, c, year, runtime, rating, overview))| {
            json!({"id": format!("{i:x}0000000000000000"), "name": name, "overview": overview, "year": year, "runtime": runtime, "rating": rating, "backdrop": art(a, b, c, "")})
        })
        .collect();
    let card = |prefix: &str, i: usize, name: &str, subtitle: &str, progress: u8, colors: (&str, &str, &str)| json!({"id": format!("{prefix}{i:x}000000000000000"), "name": name, "subtitle": subtitle, "progress": progress, "image": art(colors.0, colors.1, colors.2, "")});
    let continues = [
        ("Orbite Basse", "2024", 62, ("#071c35", "#0a3d91", "#59c3ff")),
        ("Les Cartographes", "Les Cartographes · S1 E4", 35, ("#39452a", "#7a8a3a", "#e8e0a0")),
        ("Néon Sud", "2021", 81, ("#551b2c", "#a3245c", "#ff8bd1")),
        ("Station Polaire", "Station Polaire · S2 E1", 12, ("#263345", "#5a7a9a", "#cfe6ff")),
        ("Le Dernier Phare", "2022", 48, ("#0d3d2e", "#2a6e5a", "#d8f0c8")),
        ("Plein Hiver", "2020", 90, ("#17191f", "#4a5a78", "#e6eefc")),
    ];
    let recents = [
        ("Plein Hiver", "2020", ("#17191f", "#4a5a78", "#e6eefc")),
        ("La Dernière Marée", "2024", ("#071c35", "#1aa39a", "#a8ffe8")),
        ("Vent Solaire", "2023", ("#7a3828", "#e07a3a", "#ffd889")),
        ("Les Cartographes", "2022", ("#39452a", "#7a8a3a", "#e8e0a0")),
        ("Nuit Blanche", "2021", ("#3b1d4a", "#a970ff", "#e6d8ff")),
        ("Rive Gauche", "2020", ("#551b2c", "#e50914", "#ffb3b3")),
    ];
    json!({
        "items": items,
        "continueItems": continues.iter().enumerate().map(|(i, (n, s, p, c))| card("c", i, n, s, *p, *c)).collect::<Vec<_>>(),
        "recentItems": recents.iter().enumerate().map(|(i, (n, s, c))| card("e", i, n, s, 0, *c)).collect::<Vec<_>>(),
        "needsLogin": false
    })
}

pub fn games() -> Value {
    let list = [
        ("Rally Horizon", "#7a3828", "#e07a3a", "#ffd889"),
        ("Star Drift", "#071c35", "#3b1d4a", "#a970ff"),
        ("Pixel Forge", "#064f5e", "#1aa39a", "#a8ffe8"),
        ("Kart Mania", "#551b2c", "#e50914", "#ffb3b3"),
        ("Deep Ruins", "#17191f", "#39452a", "#c8d890"),
        ("Sky Harbor", "#0a3d91", "#4f8cff", "#e6eefc"),
    ];
    json!({"games": list.iter().map(|(n, a, b, c)| json!({"name": n, "boxart": art(a, b, c, n.split(' ').next().unwrap_or(""))})).collect::<Vec<_>>(), "needsPairing": false, "unavailable": false})
}

/// Réponse fictive à un appel du shell.
pub fn respond(method: &str, settings_apps: bool) -> Value {
    match method {
        "state" => state(settings_apps),
        "weather" => json!({"temperature": 14, "code": 2, "min": 9, "max": 17, "offline": false}),
        "network" => json!({"online": true, "lan": true}),
        "jellyfin.hero" => hero(),
        "moonlight.games" => games(),
        "stats" => json!({"hostname": "raspytv", "cpu": "Cortex-A72", "ramUsed": 1_503_238_553u64, "ramTotal": 4_294_967_296u64, "temperature": 48.3,
                          "diskFree": "41.2 Go", "residentApps": 2, "residentLimit": 4, "version": env!("CARGO_PKG_VERSION"), "engine": "WebKitGTK"}),
        "audio.get" => {
            json!({"sinks": [{"id": "hdmi", "name": "Téléviseur", "detail": "HDMI", "active": true}, {"id": "usb", "name": "Casque USB", "detail": "USB", "active": false}],
                              "volume": 45, "bluetooth": [{"id": "bt:00:11:22:33:44:55", "name": "Casque sans fil"}]})
        }
        "backgrounds.list" => json!([]),
        "art.list" => json!([art("#071c35", "#0a3d91", "#59c3ff", "Tableau"), art("#3b1d4a", "#b4543a", "#f2b36b", "Tableau")]),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mock_shapes_parse() {
        let state: crate::model::State = serde_json::from_value(super::state(true)).unwrap();
        assert_eq!(state.apps.len(), 7);
        let hero: crate::model::HomeData = serde_json::from_value(super::hero()).unwrap();
        assert_eq!(hero.items.len(), 5);
        assert!(super::art("#000", "#111", "#222", "A b").starts_with("data:image/svg+xml"));
        assert!(!super::art("#000", "#111", "#222", "").contains(' '));
    }
}
