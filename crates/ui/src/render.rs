//! Génération du HTML de l'accueil et des paramètres.
//!
//! Fonctions pures (aucune dépendance au navigateur) : elles sont testées
//! nativement et reprennent à l'identique le balisage de la version Electron,
//! pour que `style.css` et `layout-override.css` s'appliquent sans changement.

use crate::model::*;

pub fn esc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Couleur CSS sûre (`#rrggbb` ou `#rgb`), sinon la couleur par défaut.
pub fn safe_color(value: &str) -> &str {
    let hex = value.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        value
    } else {
        "#438dff"
    }
}

pub fn weather_text(code: i64) -> &'static str {
    match code {
        0 => "Ciel dégagé",
        1 => "Peu nuageux",
        2 => "Nuageux",
        3 => "Couvert",
        45 | 48 => "Brouillard",
        51..=57 => "Bruine",
        61..=67 => "Pluie",
        71..=77 => "Neige",
        80..=82 => "Averses",
        95..=99 => "Orage",
        _ => "Météo",
    }
}

fn icon(app: &UiApp) -> String {
    let src = &app.icon;
    if ["file:", "http:", "https:", "assets/", "/icons/"].iter().any(|p| src.starts_with(p)) {
        format!(r#"<img src="{}" alt="" draggable="false">"#, esc(&icon_url(src)))
    } else {
        let letter: String = if src.is_empty() { app.name.chars().take(1).collect() } else { src.clone() };
        format!("<span>{}</span>", esc(&letter))
    }
}

/// Les icônes enregistrées localement (`file://…/icons/x.png`) sont servies par le shell sous `/icons/`.
pub fn icon_url(src: &str) -> String {
    if let Some(path) = src.strip_prefix("file://") {
        let name = path.rsplit(['/', '\\']).next().unwrap_or("");
        return format!("/icons/{name}");
    }
    src.to_string()
}

struct Brand {
    logo: Option<&'static str>,
    mark: Option<&'static str>,
    label: Option<&'static str>,
    bg: &'static str,
    fg: Option<&'static str>,
}

/// Logos complets sur les couleurs de marque ; sans logo, la marque et le nom.
fn brand(id: &str) -> Option<Brand> {
    Some(match id {
        "jellyfin" => Brand { logo: Some("assets/logos/jellyfin.svg"), mark: None, label: None, bg: "linear-gradient(135deg,#101b45,#000b25)", fg: None },
        "youtube" => Brand {
            logo: None,
            mark: Some("assets/icons/noutube.png"),
            label: Some("NouTube"),
            bg: "linear-gradient(135deg,#0d4a8f,#03142e)",
            fg: Some("#ffffff"),
        },
        "youtube-music" => Brand {
            logo: None,
            mark: Some("assets/icons/noutube.png"),
            label: Some("NouTube Music"),
            bg: "linear-gradient(135deg,#7a0f33,#1c040d)",
            fg: Some("#ffd7e2"),
        },
        "twitch" => Brand { logo: Some("assets/logos/twitch.svg"), mark: None, label: None, bg: "#9146ff", fg: None },
        "homelab" => Brand {
            logo: None,
            mark: Some("assets/icons/dashy.png"),
            label: Some("Dashboard"),
            bg: "linear-gradient(135deg,#06433f,#021614)",
            fg: Some("#7ff7e8"),
        },
        "immich" => Brand { logo: Some("assets/logos/immich.svg"), mark: None, label: None, bg: "#ffffff", fg: None },
        _ => return None,
    })
}

fn tile_button(attrs: &str, label: &str, body: &str, bg: &str, fg: &str, extra: &str) -> String {
    format!(r#"<button class="app-card tile {extra}" {attrs} style="--tile-bg:{bg};--tile-fg:{fg}" aria-label="{}">{body}</button>"#, esc(label))
}

fn tile_mark(src: &str, label: &str) -> String {
    format!(r#"<span class="tile-mark">{src}</span><span class="tile-label">{}</span>"#, esc(label))
}

pub fn app_card(app: &UiApp) -> String {
    let brand = if app.native { brand(&app.id) } else { None };
    let accent = safe_color(&app.color);
    let body = match &brand {
        Some(Brand { logo: Some(logo), .. }) => format!(r#"<img class="tile-logo" src="{logo}" alt="" draggable="false">"#),
        Some(Brand { mark: Some(mark), label, .. }) => tile_mark(&format!(r#"<img src="{mark}" alt="" draggable="false">"#), label.unwrap_or(&app.name)),
        _ => tile_mark(&icon(app), &app.name),
    };
    let bg = brand
        .as_ref()
        .map(|b| b.bg.to_string())
        .unwrap_or_else(|| format!("linear-gradient(135deg,color-mix(in srgb,{accent} 62%,#0b0e16),color-mix(in srgb,{accent} 22%,#06080e))"));
    let fg = brand.as_ref().and_then(|b| b.fg).unwrap_or("#fff");
    let attrs = format!(r#"data-open="{}"{}"#, esc(&app.id), if app.native { "" } else { " data-custom" });
    let label = if app.native { app.name.clone() } else { format!("{} · clic droit pour supprimer", app.name) };
    let open = if app.open { r#"<i class="tile-open" aria-hidden="true"></i>"# } else { "" };
    tile_button(&attrs, &label, &format!("{body}{open}"), &bg, fg, if app.open { "is-open" } else { "" })
}

fn utility_tile(attrs: &str, icon_html: &str, label: &str) -> String {
    tile_button(attrs, label, &tile_mark(icon_html, label), "linear-gradient(135deg,#1a2030,#0b0e16)", "#e9eef9", "utility-card")
}

pub fn weather(state: &State, data: Option<&Weather>) -> String {
    let Some(data) = data else {
        let label = if state.site.weather_label.is_empty() { String::new() } else { format!("{} · ", esc(&state.site.weather_label)) };
        return format!(r#"<div class="weather ghost">{label}météo…</div>"#);
    };
    if data.unavailable {
        return r#"<div class="weather ghost">Météo indisponible</div>"#.into();
    }
    format!(
        r#"<div class="weather"><span class="sun">☀</span><div><b>{}°</b><span>{}</span></div><small>{}° / {}°{}</small></div>"#,
        data.temperature,
        weather_text(data.code),
        data.min,
        data.max,
        if data.offline { " · hors ligne" } else { "" }
    )
}

/// (classe, libellé, détail)
pub fn network_parts(network: Option<Network>) -> (&'static str, &'static str, &'static str) {
    match network {
        None => ("checking", "Vérification…", "Aucune connexion réseau"),
        Some(n) if n.online => ("online", "En ligne", "Connexion Internet disponible"),
        Some(n) if n.lan => ("offline", "Hors ligne", "Réseau local disponible, Internet indisponible"),
        Some(_) => ("offline", "Hors ligne", "Aucune connexion réseau"),
    }
}

pub fn network_badge(network: Option<Network>) -> String {
    let (class, label, detail) = network_parts(network);
    format!(r#"<span id="network-status" class="network-status {class}" title="{detail}"><i></i>{label}</span>"#)
}

pub struct HomeContext<'a> {
    pub state: &'a State,
    pub weather: Option<&'a Weather>,
    pub network: Option<Network>,
    pub date: &'a str,
    pub time: &'a str,
}

pub fn home(ctx: &HomeContext) -> String {
    let state = ctx.state;
    let apps: String = state.ordered_apps().into_iter().map(app_card).collect();
    let art = utility_tile("data-art", r#"<img src="assets/icons/gallery.png" alt="" draggable="false">"#, "Mode Tableau");
    let add = utility_tile("data-add", r#"<span class="tile-plus">+</span>"#, "Ajouter un raccourci");
    let settings = utility_tile(r#"data-page="settings""#, r#"<img src="assets/icons/settings.png" alt="" draggable="false">"#, "Paramètres");
    let clock = if state.config.show_clock { format!(r#"<time id="clock">{}</time>"#, esc(ctx.time)) } else { String::new() };
    let weather = if state.config.show_weather { weather(state, ctx.weather) } else { String::new() };
    format!(
        concat!(
            r#"<header class="tv-header"><div class="identity"><div><strong>RaspyTV</strong><p class="date" id="date">{date}</p></div></div><div class="weather-stack">{network}{clock}{weather}</div></header>"#,
            r#"<section id="cinema-hero" class="cinema-hero is-loading" aria-live="polite"><div class="hero-shade"></div><div class="hero-copy"><span class="hero-kicker">LUMO · À DÉCOUVRIR</span><h1>Votre cinéma</h1><p>Chargement de votre sélection…</p></div></section>"#,
            r#"<section class="home-stage"><section id="continue-section" class="content-strip" hidden><div class="home-section-head"><h2>Continuer</h2></div><div id="continue-row" class="continue-row"></div></section>"#,
            r#"<section id="recent-section" class="content-strip" hidden><div class="home-section-head"><h2>Récemment ajoutés</h2></div><div id="recent-row" class="continue-row"></div></section>"#,
            r#"<section class="apps-section"><div class="home-section-head"><h2>Applications</h2></div><div class="grid tv-grid">{apps}{art}{add}{settings}</div></section>"#,
            r#"<section class="games-section"><div class="home-section-head"><h2>Jeux</h2><span id="moonlight-status">Moonlight · synchronisation…</span></div><div id="moonlight-row" class="game-row"><button class="game-card moonlight-setup" data-moonlight-open><img src="assets/icons/moonlight.svg" alt=""><span>Chargement des jeux…</span></button></div></section></section>"#
        ),
        date = esc(ctx.date),
        network = network_badge(ctx.network),
        clock = clock,
        weather = weather,
        apps = apps,
        art = art,
        add = add,
        settings = settings,
    )
}

pub fn hero_meta(item: &HeroItem) -> String {
    let runtime = item.runtime.map(|r| format!("{r} min")).unwrap_or_default();
    let rating = if item.rating.is_empty() { String::new() } else { format!("★ {}", item.rating) };
    [item.year.clone(), runtime, rating].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// Supprime les balises HTML d'un résumé Jellyfin.
pub fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

pub fn hero_slide(items: &[HeroItem], index: usize) -> String {
    let item = &items[index];
    let overview = strip_tags(&item.overview);
    let overview = if overview.is_empty() { "Disponible dans votre médiathèque.".to_string() } else { overview };
    let dots: String = (0..items.len())
        .map(|i| format!(r#"<button data-hero-index="{i}" class="{}" aria-label="Sélection {}"></button>"#, if i == index { "active" } else { "" }, i + 1))
        .collect();
    format!(
        r#"<div class="hero-shade"></div><div class="hero-copy"><span class="hero-kicker">LUMO · À DÉCOUVRIR</span><h1>{}</h1><div class="hero-meta">{}</div><p>{}</p><button class="hero-play" data-jellyfin-play="{}">▶ Lecture</button></div><div class="hero-dots">{dots}</div>"#,
        esc(&item.name),
        esc(&hero_meta(item)),
        esc(&overview),
        esc(&item.id)
    )
}

pub fn hero_unavailable(needs_login: bool) -> String {
    let (title, text) = if needs_login {
        ("Connexion requise", "Ouvre Jellyfin et connecte-toi une seule fois. RaspyTV conservera ensuite la session.")
    } else {
        ("Médiathèque indisponible", "Le serveur répond, mais la médiathèque ne peut pas être chargée pour le moment.")
    };
    format!(
        r#"<div class="hero-shade"></div><div class="hero-copy"><span class="hero-kicker">JELLYFIN</span><h1>{title}</h1><p>{text}</p><button class="hero-play" data-open="jellyfin">Ouvrir Jellyfin</button><button class="hero-retry" data-hero-retry>Réessayer</button></div>"#
    )
}

pub fn media_card(item: &MediaCard, action: &str) -> String {
    let image = if item.image.is_empty() {
        String::new()
    } else {
        format!(r#"<img src="{}" alt="" loading="lazy" decoding="async" onerror="this.remove()">"#, esc(&item.image))
    };
    let subtitle = if item.subtitle.is_empty() { "Jellyfin" } else { &item.subtitle };
    let progress = if item.progress > 0.0 {
        format!(r#"<span class="continue-progress"><i style="width:{}%"></i></span>"#, item.progress.clamp(0.0, 100.0))
    } else {
        String::new()
    };
    format!(
        r#"<button class="continue-card" data-jellyfin-play="{id}" aria-label="{action} {name}">{image}<span class="continue-shade"></span><span class="continue-copy"><strong>{name}</strong><small>{subtitle}</small></span>{progress}</button>"#,
        id = esc(&item.id),
        name = esc(&item.name),
        subtitle = esc(subtitle),
    )
}

/// (statut, contenu de la rangée Jeux)
pub fn games_row(data: &Games) -> (String, String) {
    if !data.games.is_empty() {
        let n = data.games.len();
        let status = format!("{n} jeu{} · PC", if n > 1 { "x" } else { "" });
        let row = data
            .games
            .iter()
            .map(|g| {
                let art = if g.boxart.is_empty() {
                    r#"<img class="game-fallback" src="assets/icons/moonlight.svg" alt="">"#.to_string()
                } else {
                    format!(r#"<img src="{}" alt="" loading="lazy" decoding="async" onerror="this.remove()">"#, esc(&g.boxart))
                };
                format!(r#"<button class="game-card" data-moonlight-game="{n}" aria-label="Jouer à {n}">{art}<span>{n}</span></button>"#, n = esc(&g.name))
            })
            .collect();
        return (status, row);
    }
    let status = if data.needs_pairing {
        "PC détecté · association requise"
    } else if data.unavailable {
        "PC indisponible"
    } else {
        "Aucun jeu publié"
    };
    let label = if data.needs_pairing {
        "Associer Moonlight"
    } else if data.unavailable {
        "Ouvrir Moonlight"
    } else {
        "Configurer les jeux"
    };
    (
        status.into(),
        format!(r#"<button class="game-card moonlight-setup" data-moonlight-open><img src="assets/icons/moonlight.svg" alt=""><span>{label}</span></button>"#),
    )
}

// ------------------------------------------------------------------ paramètres

pub struct Tab {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub icon: &'static str,
}

pub const TABS: [Tab; 5] = [
    Tab {
        id: "display",
        label: "Affichage",
        hint: "Thème, arrière-plan et animations",
        icon: r#"<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>"#,
    },
    Tab { id: "home", label: "Accueil", hint: "Ce qui s’affiche sur l’écran d’accueil", icon: r#"<path d="M4 11l8-7 8 7"/><path d="M6 10v10h12V10"/>"# },
    Tab { id: "sound", label: "Son", hint: "Sortie audio, volume et bruitages", icon: r#"<path d="M4 10v4h4l5 4V6L8 10H4z"/><path d="M17 9a4 4 0 0 1 0 6"/>"# },
    Tab {
        id: "apps",
        label: "Applications",
        hint: "Applications ouvertes et mémoire",
        icon: r#"<rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/>"#,
    },
    Tab {
        id: "system",
        label: "Système",
        hint: "État du Raspberry Pi et alimentation",
        icon: r#"<rect x="7" y="7" width="10" height="10" rx="2"/><path d="M10 3v4M14 3v4M10 17v4M14 17v4M3 10h4M3 14h4M17 10h4M17 14h4"/>"#,
    },
];

pub fn setting_row(title: &str, hint: &str, control: &str) -> String {
    let hint = if hint.is_empty() { String::new() } else { format!("<small>{hint}</small>") };
    format!(r#"<div class="set-row"><div class="set-text"><strong>{title}</strong>{hint}</div><div class="set-control">{control}</div></div>"#)
}

fn group(title: &str, body: &str) -> String {
    let title = if title.is_empty() { String::new() } else { format!("<h2>{title}</h2>") };
    format!(r#"<section class="set-group">{title}{body}</section>"#)
}

fn switch(field: &str, checked: bool, name: &str) -> String {
    format!(
        r#"<label class="switch"><input type="checkbox" data-setting="{field}" aria-label="{}" {}><span class="switch-track"></span></label>"#,
        esc(name),
        if checked { "checked" } else { "" }
    )
}

fn select(field: &str, options: &[(String, String)], value: &str) -> String {
    let options: String =
        options.iter().map(|(v, t)| format!(r#"<option value="{}" {}>{}</option>"#, esc(v), if v == value { "selected" } else { "" }, esc(t))).collect();
    format!(r#"<select data-setting="{field}">{options}</select>"#)
}

pub fn range(attributes: &str, value: u8) -> String {
    format!(r#"<div class="range"><input type="range" min="0" max="100" value="{value}" {attributes}><output>{value}%</output></div>"#)
}

pub const BACKGROUND_COLORS: [(&str, &str); 13] = [
    ("#000000", "Noir"),
    ("#050507", "Noir doux"),
    ("#f4f5f7", "Blanc"),
    ("#17191f", "Anthracite"),
    ("#263345", "Ardoise"),
    ("#071c35", "Bleu nuit"),
    ("#0a3d91", "Bleu profond"),
    ("#064f5e", "Cyan profond"),
    ("#0d3d2e", "Vert forêt"),
    ("#39452a", "Olive"),
    ("#551b2c", "Bordeaux"),
    ("#3b1d4a", "Aubergine"),
    ("#7a3828", "Terre cuite"),
];

fn background_select(value: &str, backgrounds: &[Background]) -> String {
    let option = |id: &str, name: &str| format!(r#"<option value="{}" {}>{}</option>"#, esc(id), if value == id { "selected" } else { "" }, esc(name));
    let group = |label: &str, entries: String| if entries.is_empty() { String::new() } else { format!(r#"<optgroup label="{label}">{entries}</optgroup>"#) };
    let statics: String = backgrounds.iter().filter(|b| b.kind == "image").map(|b| option(&b.id, &b.name)).collect();
    let videos: String = backgrounds.iter().filter(|b| b.kind == "video").map(|b| option(&b.id, &b.name)).collect();
    let colors: String = BACKGROUND_COLORS.iter().map(|(c, n)| option(&format!("color:{c}"), n)).collect();
    format!(
        r#"<select data-setting="background">{}{}{}{}</select>"#,
        group("Fonds statiques", statics),
        group("Fonds animés", videos),
        group("Couleurs fixes", colors),
        option("none", "Aucun")
    )
}

pub fn settings_pane(tab: &str, state: &State, backgrounds: &[Background]) -> String {
    let c = &state.config;
    match tab {
        "home" => {
            let label = if state.site.weather_label.is_empty() { String::new() } else { format!("{}, ", esc(&state.site.weather_label)) };
            group(
                "",
                &[
                    setting_row("Météo", &format!("{label}en haut à droite"), &switch("showWeather", c.show_weather, "Météo")),
                    setting_row("Horloge", "", &switch("showClock", c.show_clock, "Horloge")),
                    setting_row("Applications récentes", "", &switch("showRecent", c.show_recent, "Applications récentes")),
                ]
                .concat(),
            )
        }
        "sound" => {
            group(
                "Sortie du système",
                r#"<div id="audio-settings"><div class="audio-system"><p class="set-loading">Détection des sorties audio…</p></div></div>"#,
            ) + &group(
                "Bruitages de l’interface",
                &(setting_row("Sons d’interface", "Petits sons à la navigation", &switch("sounds", c.sounds, "Sons d’interface"))
                    + &setting_row("Volume des bruitages", "", &range(r#"data-setting="soundVolume""#, c.sound_volume))),
            )
        }
        "apps" => {
            let open: Vec<&UiApp> = state.apps.iter().filter(|a| a.open).collect();
            let mut rows: String = open
                .iter()
                .map(|a| setting_row(&esc(&a.name), "Ouverte en arrière-plan", &format!(r#"<button class="pill" data-close="{}">Fermer</button>"#, esc(&a.id))))
                .collect();
            if rows.is_empty() {
                rows = setting_row("Aucune application ouverte", "Les applications restent ouvertes quand on revient à l’accueil.", "");
            } else {
                rows.push_str(r#"<div class="set-actions"><button class="pill danger" data-close-all>Fermer toutes les applications</button></div>"#);
            }
            let limits: Vec<(String, String)> = (2..=6).map(|v: u8| (v.to_string(), v.to_string())).collect();
            let limit = if c.max_resident_apps == 0 { 4 } else { c.max_resident_apps };
            group("Applications ouvertes", &rows)
                + &group(
                    "Mémoire",
                    &setting_row(
                        "Applications gardées en mémoire",
                        "Au-delà, les moins récentes sont fermées. Les sessions restent enregistrées.",
                        &select("maxResidentApps", &limits, &limit.to_string()),
                    ),
                )
        }
        "system" => {
            group("Raspberry Pi", r#"<div id="stats" class="stats"><p class="set-loading">Chargement des informations système…</p></div>"#)
                + &group(
                    "Alimentation",
                    &(setting_row("Redémarrer", "Une confirmation est demandée", r#"<button class="pill danger" data-action="reboot">Redémarrer</button>"#)
                        + &setting_row(
                            "Éteindre",
                            "Il faudra rallumer le Raspberry Pi à la main",
                            r#"<button class="pill danger" data-action="poweroff">Éteindre</button>"#,
                        )),
                )
        }
        _ => {
            let themes = [("dark", "Sombre"), ("light", "Clair"), ("auto", "Automatique")].map(|(a, b)| (a.to_string(), b.to_string()));
            group(
                "",
                &[
                    setting_row("Thème", "", &select("theme", &themes, &c.theme)),
                    setting_row("Arrière-plan", "Fond de l’écran d’accueil", &background_select(&c.background, backgrounds)),
                    setting_row(
                        "Réduire les animations",
                        "Plus sobre, et plus léger pour le Raspberry Pi",
                        &switch("reducedMotion", c.reduced_motion, "Réduire les animations"),
                    ),
                    setting_row("Heure au format 24 h", "", &switch("clock24", c.clock24, "Heure au format 24 h")),
                ]
                .concat(),
            )
        }
    }
}

pub fn settings(tab: &str, state: &State, backgrounds: &[Background]) -> String {
    let current = TABS.iter().find(|t| t.id == tab).unwrap_or(&TABS[0]);
    let tabs: String = TABS
        .iter()
        .map(|t| {
            let active = t.id == current.id;
            format!(
                r#"<button class="settings-tab {}" data-settings-tab="{}" aria-current="{}"><svg viewBox="0 0 24 24" aria-hidden="true">{}</svg><span>{}</span></button>"#,
                if active { "active" } else { "" },
                t.id,
                if active { "page" } else { "false" },
                t.icon,
                t.label
            )
        })
        .collect();
    format!(
        r#"<header class="settings-head"><button class="back" data-home>← <span>Accueil</span></button><div><strong>Paramètres</strong><small>{}</small></div></header><div class="settings-shell"><nav class="settings-tabs" aria-label="Sections des paramètres">{tabs}</nav><div class="settings-pane">{}</div></div>"#,
        esc(current.hint),
        settings_pane(current.id, state, backgrounds)
    )
}

pub fn audio_settings(audio: &Audio) -> String {
    let refresh = r#"<div class="set-actions"><button class="pill" data-audio-refresh>Actualiser les appareils</button></div>"#;
    if audio.unavailable || (audio.sinks.is_empty() && audio.bluetooth.is_empty()) {
        return setting_row("Aucune sortie audio détectée", "Branche ou allume un appareil, puis actualise.", "") + refresh;
    }
    let mut options: String = audio
        .sinks
        .iter()
        .map(|s| {
            let detail = if s.detail.is_empty() { String::new() } else { format!(" · {}", esc(&s.detail)) };
            format!(r#"<option value="{}" {}>{}{detail}</option>"#, esc(&s.id), if s.active { "selected" } else { "" }, esc(&s.name))
        })
        .collect();
    options.extend(audio.bluetooth.iter().map(|d| format!(r#"<option value="{}">{} · Bluetooth, se connecter</option>"#, esc(&d.id), esc(&d.name))));
    setting_row(
        "Sortie audio",
        "Les casques Bluetooth jumelés apparaissent ici dès qu’ils sont allumés.",
        &format!("<select data-audio-output>{options}</select>"),
    ) + &setting_row("Volume du système", "", &range("data-system-volume", audio.volume))
        + refresh
}

pub fn audio_unavailable() -> String {
    setting_row("Contrôle audio indisponible", "", r#"<button class="pill" data-audio-refresh>Réessayer</button>"#)
}

fn gigabytes(bytes: u64) -> f64 {
    (bytes as f64 / 1_073_741_824.0 * 10.0).round() / 10.0
}

pub fn stats(s: &Stats) -> String {
    let temperature = s.temperature.map(|t| format!("{t:.1} °C")).unwrap_or_else(|| "Indisponible".into());
    format!(
        "<dl><dt>Machine</dt><dd>{}</dd><dt>CPU</dt><dd>{}</dd><dt>RAM</dt><dd>{} / {} Go</dd><dt>Température</dt><dd>{temperature}</dd><dt>Disque libre</dt><dd>{}</dd><dt>Apps en mémoire</dt><dd>{} / {}</dd><dt>RaspyTV</dt><dd>{} · Rust · {}</dd></dl>",
        esc(&s.hostname),
        esc(&s.cpu),
        gigabytes(s.ram_used),
        gigabytes(s.ram_total),
        esc(s.disk_free.as_deref().unwrap_or("—")),
        s.resident_apps,
        s.resident_limit,
        esc(&s.version),
        esc(&s.engine)
    )
}

// ------------------------------------------------------------------ fenêtres

pub fn power_confirm(action: &str) -> String {
    let reboot = action == "reboot";
    format!(
        r#"<div class="modal"><div class="confirm"><h2>{} le Raspberry Pi ?</h2><p>{}</p><button class="back" data-dismiss>Annuler</button><button class="danger" data-power-confirm="{}">{}</button></div></div>"#,
        if reboot { "Redémarrer" } else { "Éteindre" },
        if reboot { "RaspyTV revient dans une minute environ." } else { "Il faudra le rallumer à la main." },
        if reboot { "reboot" } else { "poweroff" },
        if reboot { "Redémarrer" } else { "Éteindre" }
    )
}

pub fn shortcut_dialog() -> String {
    r#"<div class="modal"><form id="shortcut-form"><button type="button" class="close" data-dismiss>×</button><h2>Ajouter un raccourci</h2><label>Nom<input name="name" maxlength="60" required placeholder="Ex. Arte"></label><label>URL<input name="url" type="url" required placeholder="https://…"></label><p class="form-error" hidden></p><p>L’icône est récupérée automatiquement. Tu peux la modifier ultérieurement.</p><button class="primary">Ajouter au Home</button></form></div>"#.into()
}

pub fn delete_shortcut_dialog(app: &UiApp) -> String {
    format!(
        r#"<div class="modal"><div class="confirm"><button class="close" data-dismiss>×</button><h2>Supprimer {} ?</h2><p>Cette application personnalisée sera retirée de RaspyTV. Son site et ses données externes ne seront pas supprimés.</p><button class="back" data-dismiss>Annuler</button><button class="danger confirm-delete" data-remove-shortcut="{}">Supprimer</button></div></div>"#,
        esc(&app.name),
        esc(&app.id)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State {
            config: UiConfig {
                theme: "dark".into(),
                show_clock: true,
                show_weather: true,
                app_order: vec!["twitch".into(), "jellyfin".into()],
                ..UiConfig::default()
            },
            apps: vec![
                UiApp { id: "jellyfin".into(), name: "Jellyfin".into(), native: true, ..UiApp::default() },
                UiApp { id: "twitch".into(), name: "Twitch".into(), native: true, open: true, ..UiApp::default() },
                UiApp { id: "custom-1".into(), name: "<Radio>".into(), color: "red;}".into(), icon: "file:///home/x/icons/a.png".into(), ..UiApp::default() },
            ],
            ..State::default()
        }
    }

    #[test]
    fn escapes_and_orders() {
        let s = state();
        let html = home(&HomeContext { state: &s, weather: None, network: None, date: "jeudi 13 novembre", time: "18:30" });
        let twitch = html.find(r#"data-open="twitch""#).unwrap();
        let jellyfin = html.find(r#"data-open="jellyfin""#).unwrap();
        assert!(twitch < jellyfin);
        assert!(html.contains("&lt;Radio&gt;") && !html.contains("<Radio>"));
        assert!(html.contains("#438dff") && !html.contains("red;}"));
        assert!(html.contains(r#"src="/icons/a.png""#));
        assert!(html.contains("is-open"));
    }

    #[test]
    fn hero_and_rows() {
        let items = vec![HeroItem {
            id: "abc".into(),
            name: "Film".into(),
            overview: "<p>Bien</p>".into(),
            year: "2024".into(),
            rating: "7.5".into(),
            runtime: Some(120),
            ..HeroItem::default()
        }];
        let slide = hero_slide(&items, 0);
        assert!(slide.contains("2024 · 120 min · ★ 7.5") && !slide.contains("&lt;p&gt;"));
        let (status, _) =
            games_row(&Games { games: vec![Game { name: "A".into(), ..Game::default() }, Game { name: "B".into(), ..Game::default() }], ..Games::default() });
        assert_eq!(status, "2 jeux · PC");
        assert_eq!(games_row(&Games { needs_pairing: true, ..Games::default() }).0, "PC détecté · association requise");
    }

    #[test]
    fn settings_tabs() {
        let s = state();
        assert!(settings("apps", &s, &[]).contains(r#"data-close="twitch""#));
        assert!(settings("inconnu", &s, &[]).contains("Thème"));
        assert!(power_confirm("reboot").contains(r#"data-power-confirm="reboot""#));
        assert_eq!(weather_text(63), "Pluie");
    }
}
