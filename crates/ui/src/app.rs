//! Branchement du rendu sur la page (WebAssembly).
//!
//! Règle d'or : l'état vit dans un `RefCell` et n'est jamais emprunté pendant un
//! appel au DOM (un `focus()` peut déclencher aussitôt un autre gestionnaire).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    AddEventListenerOptions, Document, Element, Event, FocusOptions, HtmlAudioElement, HtmlElement, HtmlImageElement, HtmlInputElement, HtmlMediaElement,
    HtmlSelectElement, KeyboardEvent, MouseEvent, ScrollBehavior, ScrollIntoViewOptions, ScrollLogicalPosition, WheelEvent, Window,
};

use crate::mock;
use crate::model::*;
use crate::render;

// ------------------------------------------------------------------ état

#[derive(Clone, PartialEq)]
struct FocusToken {
    attr: Option<(String, String)>,
    index: Option<usize>,
    page: String,
}

#[derive(Default)]
struct Gallery {
    open: bool,
    closing: bool,
    index: usize,
    generation: u32,
    timer: Option<i32>,
    files: Vec<String>,
}

struct App {
    state: Option<State>,
    page: String,
    tab: String,
    weather: Option<Weather>,
    network: Option<Network>,
    backgrounds: Vec<Background>,
    hero_items: Vec<HeroItem>,
    hero_index: usize,
    hero_timer: Option<i32>,
    hero_transition: Option<i32>,
    hero_retry: Option<i32>,
    hero_retry_delay: i32,
    hero_generation: u32,
    preloaded: Vec<String>,
    moonlight_generation: u32,
    moonlight_timer: Option<i32>,
    network_timer: Option<i32>,
    toast_timer: Option<i32>,
    launching: bool,
    focus: Option<FocusToken>,
    modal_return: Option<Element>,
    previous_focus: Option<Element>,
    gallery: Gallery,
    last_hover: f64,
    audio_pending: bool,
    render_key: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            state: None,
            page: "home".into(),
            tab: "display".into(),
            weather: None,
            network: None,
            backgrounds: Vec::new(),
            hero_items: Vec::new(),
            hero_index: 0,
            hero_timer: None,
            hero_transition: None,
            hero_retry: None,
            hero_retry_delay: 15_000,
            hero_generation: 0,
            preloaded: Vec::new(),
            moonlight_generation: 0,
            moonlight_timer: None,
            network_timer: None,
            toast_timer: None,
            launching: false,
            focus: None,
            modal_return: None,
            previous_focus: None,
            gallery: Gallery::default(),
            last_hover: -1000.0,
            audio_pending: false,
            render_key: String::new(),
        }
    }
}

/// Suite d'un appel au shell.
type Reply = Box<dyn FnOnce(Result<Value, String>)>;

thread_local! {
    static APP: RefCell<App> = RefCell::new(App::default());
    static PENDING: RefCell<HashMap<u32, Reply>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    static SOUNDS: RefCell<Option<(HtmlAudioElement, HtmlAudioElement)>> = const { RefCell::new(None) };
}

fn app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

fn state() -> Option<State> {
    app(|a| a.state.clone())
}

fn page() -> String {
    app(|a| a.page.clone())
}

fn active_id() -> Option<String> {
    app(|a| a.state.as_ref().and_then(|s| s.active_id.clone()))
}

// ------------------------------------------------------------------ DOM

fn window() -> Window {
    web_sys::window().expect("window")
}

fn document() -> Document {
    window().document().expect("document")
}

fn q(selector: &str) -> Option<Element> {
    document().query_selector(selector).ok().flatten()
}

fn qa(scope: &Element, selector: &str) -> Vec<Element> {
    let Ok(list) = scope.query_selector_all(selector) else { return Vec::new() };
    (0..list.length()).filter_map(|i| list.get(i)?.dyn_into::<Element>().ok()).collect()
}

fn root() -> Element {
    q("#app").expect("#app")
}

fn body() -> HtmlElement {
    document().body().expect("body")
}

fn html_root() -> Element {
    document().document_element().expect("html")
}

fn set_timeout(f: impl FnOnce() + 'static, ms: i32) -> i32 {
    let callback = Closure::once_into_js(f);
    window().set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), ms).unwrap_or(0)
}

fn clear(timer: Option<i32>) {
    if let Some(id) = timer {
        window().clear_timeout_with_handle(id);
    }
}

fn now() -> f64 {
    window().performance().map(|p| p.now()).unwrap_or(0.0)
}

fn focus_el(el: &Element) {
    if let Some(el) = el.dyn_ref::<HtmlElement>() {
        let options = FocusOptions::new();
        options.set_prevent_scroll(true);
        let _ = el.focus_with_options(&options);
    }
}

fn click_el(el: &Element) {
    if let Some(el) = el.dyn_ref::<HtmlElement>() {
        el.click();
    }
}

fn scroll_nearest(el: &Element, smooth: bool) {
    let options = ScrollIntoViewOptions::new();
    options.set_block(ScrollLogicalPosition::Nearest);
    options.set_inline(ScrollLogicalPosition::Nearest);
    options.set_behavior(if smooth { ScrollBehavior::Smooth } else { ScrollBehavior::Auto });
    el.scroll_into_view_with_scroll_into_view_options(&options);
}

fn has_class(el: &Element, class: &str) -> bool {
    el.class_list().contains(class)
}

fn toggle_class(el: &Element, class: &str, on: bool) {
    let _ = el.class_list().toggle_with_force(class, on);
}

fn attr(el: &Element, name: &str) -> Option<String> {
    el.get_attribute(name)
}

fn active_element() -> Option<Element> {
    document().active_element()
}

fn closest(target: &Option<web_sys::EventTarget>, selector: &str) -> Option<Element> {
    target.as_ref()?.dyn_ref::<Element>()?.closest(selector).ok().flatten()
}

fn matches(el: &Element, selector: &str) -> bool {
    el.matches(selector).unwrap_or(false)
}

fn modal_open() -> bool {
    q(".modal").is_some()
}

thread_local! {
    static IGNORE: Closure<dyn FnMut(JsValue)> = Closure::new(|_: JsValue| {});
}

/// Ignore le rejet d'une promesse (lecture audio refusée, par exemple).
fn ignore_rejection(promise: &js_sys::Promise) {
    IGNORE.with(|handler| {
        let _ = promise.catch(handler);
    });
}

// ------------------------------------------------------------------ dialogue avec le shell

fn has_ipc() -> bool {
    js_sys::Reflect::get(&window(), &"ipc".into()).map(|v| !v.is_undefined() && !v.is_null()).unwrap_or(false)
}

fn post(message: &Value) {
    let Ok(ipc) = js_sys::Reflect::get(&window(), &"ipc".into()) else { return };
    if let Ok(function) = js_sys::Reflect::get(&ipc, &"postMessage".into()) {
        if let Some(function) = function.dyn_ref::<js_sys::Function>() {
            let _ = function.call1(&ipc, &JsValue::from_str(&message.to_string()));
        }
    }
}

fn demo_param(name: &str) -> Option<String> {
    let search = window().location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(name)
}

/// Appel au shell, réponse par `receive({t:'reply'})`. En démonstration, réponse fictive.
fn call(method: &str, params: Value, done: impl FnOnce(Result<Value, String>) + 'static) {
    if !has_ipc() {
        let value = mock::respond(method, demo_param("tab").as_deref() == Some("apps"));
        set_timeout(move || done(Ok(value)), 40);
        return;
    }
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1));
        id
    });
    PENDING.with(|p| p.borrow_mut().insert(id, Box::new(done)));
    post(&json!({"id": id, "method": method, "params": params}));
}

/// Message sans réponse attendue.
fn notify(method: &str, params: Value) {
    if has_ipc() {
        post(&json!({"method": method, "params": params}));
    }
}

/// Point d'entrée des messages du shell (`window.__raspyHome.receive(json)`).
#[wasm_bindgen]
pub fn receive(message: &str) {
    let Ok(message) = serde_json::from_str::<Value>(message) else { return };
    match message.get("t").and_then(Value::as_str).unwrap_or("") {
        "reply" => {
            let id = message.get("id").and_then(Value::as_u64).unwrap_or(0) as u32;
            if let Some(done) = PENDING.with(|p| p.borrow_mut().remove(&id)) {
                if message.get("ok") == Some(&Value::Bool(true)) {
                    done(Ok(message.get("value").cloned().unwrap_or(Value::Null)));
                } else {
                    done(Err(message.get("error").and_then(Value::as_str).unwrap_or("Erreur").to_string()));
                }
            }
        }
        "state" => {
            if let Ok(next) = serde_json::from_value::<State>(message["value"].clone()) {
                accept_state(next);
            }
        }
        "toast" => show_toast(message["text"].as_str().unwrap_or("")),
        "navigate" => {
            let target = message["page"].as_str().unwrap_or("");
            if target == "home" || target == "settings" {
                if let Some(modal) = q(".modal") {
                    modal.remove();
                }
                app(|a| a.page = target.to_string());
                render();
            }
        }
        "tvBack" => {
            if !close_modal() && page() == "settings" {
                notify("tv.settings", Value::Null);
            }
        }
        "audioChanged" => refresh_audio_when_idle(),
        _ => {}
    }
}

/// Touche venue de la manette (gilrs, dans le shell).
#[wasm_bindgen]
pub fn tv_key(key: &str) {
    if app(|a| a.gallery.open) {
        if matches!(key, "Escape" | "Enter" | "Home") {
            close_gallery();
        }
        return;
    }
    match key {
        "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" => {
            let direction = &key[5..].to_lowercase();
            let editing = active_element().is_some_and(|el| matches(&el, "input[type=range]"));
            if editing && (direction == "left" || direction == "right") {
                if let Some(input) = active_element().and_then(|e| e.dyn_into::<HtmlInputElement>().ok()) {
                    let value: f64 = input.value().parse().unwrap_or(0.0);
                    let next = (value + if direction == "left" { -5.0 } else { 5.0 }).clamp(0.0, 100.0);
                    input.set_value(&next.to_string());
                    let _ = input.dispatch_event(&Event::new("change").expect("event"));
                }
                return;
            }
            spatial_move(direction);
        }
        "Enter" => {
            if let Some(el) = active_element() {
                click_el(&el);
            }
        }
        "Escape" => escape(),
        "Home" => {
            app(|a| a.page = "home".into());
            render();
        }
        _ => {}
    }
}

// ------------------------------------------------------------------ rendu

fn time_text(clock24: bool) -> String {
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"hour".into(), &"2-digit".into());
    let _ = js_sys::Reflect::set(&options, &"minute".into(), &"2-digit".into());
    let _ = js_sys::Reflect::set(&options, &"hour12".into(), &JsValue::from_bool(!clock24));
    js_sys::Date::new_0().to_locale_time_string_with_options("fr-FR", &options).into()
}

fn date_text() -> String {
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"weekday".into(), &"long".into());
    let _ = js_sys::Reflect::set(&options, &"day".into(), &"numeric".into());
    let _ = js_sys::Reflect::set(&options, &"month".into(), &"long".into());
    js_sys::Date::new_0().to_locale_date_string("fr-FR", &options).into()
}

fn prefers_light() -> bool {
    window().match_media("(prefers-color-scheme: light)").ok().flatten().is_some_and(|m| m.matches())
}

fn render() {
    let Some(state) = state() else { return };
    let (page, tab, weather, network, backgrounds) = app(|a| (a.page.clone(), a.tab.clone(), a.weather.clone(), a.network, a.backgrounds.clone()));
    notify("shell.page", json!(page));
    remember_focus();
    let theme = match state.config.theme.as_str() {
        "auto" => {
            if prefers_light() {
                "light"
            } else {
                "dark"
            }
        }
        "light" => "light",
        _ => "dark",
    };
    let _ = html_root().set_attribute("data-theme", theme);
    toggle_class(&html_root(), "reduce", state.config.reduced_motion);
    let timers = app(|a| {
        a.hero_generation += 1;
        a.moonlight_generation += 1;
        a.render_key = render_key(&a.page, &state, a.weather.as_ref());
        [a.hero_transition.take(), a.hero_timer.take(), a.hero_retry.take(), a.moonlight_timer.take()]
    });
    timers.into_iter().for_each(clear);
    sync_background();
    let html = if page == "settings" {
        render::settings(&tab, &state, &backgrounds)
    } else {
        render::home(&render::HomeContext { state: &state, weather: weather.as_ref(), network, date: &date_text(), time: &time_text(state.config.clock24) })
    };
    root().set_inner_html(&html);
    if page == "settings" {
        match tab.as_str() {
            "system" => load_stats(),
            "sound" => load_audio_settings(),
            "display" if backgrounds.is_empty() => load_backgrounds(),
            _ => {}
        }
    } else if state.active_id.is_none() {
        root().set_scroll_top(0);
        load_hero(false);
        load_moonlight();
        paint_network();
    }
    if state.active_id.is_none() {
        set_timeout(restore_focus, 16);
    }
}

/// Ce qui, dans l'état, change l'affichage (le reste ne provoque pas de nouveau rendu).
fn render_key(page: &str, state: &State, weather: Option<&Weather>) -> String {
    let c = &state.config;
    let apps: Vec<_> = state.apps.iter().map(|a| (&a.id, &a.name, &a.icon, &a.color, a.native, if page == "settings" { a.open } else { false })).collect();
    format!(
        "{page}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{}",
        state.active_id,
        c.theme,
        c.background,
        c.clock24,
        c.sounds,
        c.sound_volume,
        c.reduced_motion,
        c.show_weather,
        c.show_clock,
        c.app_order,
        apps,
        weather.map(|w| w.temperature),
        c.max_resident_apps
    )
}

fn accept_state(next: State) {
    let (changed, first) = app(|a| {
        let key = render_key(&a.page, &next, a.weather.as_ref());
        let first = a.state.is_none();
        let changed = key != a.render_key;
        a.state = Some(next.clone());
        (changed, first)
    });
    if first || changed || root().children().length() == 0 {
        render();
        return;
    }
    sync_background();
    if next.active_id.is_none() {
        restore_focus();
    }
    hero_schedule();
}

fn sync_background() {
    let Some(state) = state() else { return };
    let backgrounds = app(|a| a.backgrounds.clone());
    let Some(video) = q("#background-video").and_then(|v| v.dyn_into::<web_sys::HtmlVideoElement>().ok()) else { return };
    let key = state.config.background.clone();
    let solid = key.strip_prefix("color:").filter(|c| render::safe_color(c) == *c).map(String::from);
    let entry = backgrounds.iter().find(|b| b.id == key).cloned();
    let static_source = entry.as_ref().filter(|b| b.kind == "image").map(|b| b.url.clone());
    let style = html_root().dyn_into::<HtmlElement>().expect("html").style();
    let _ = style.set_property("--raspy-bg", solid.as_deref().unwrap_or("var(--bg)"));
    let _ = style.set_property("--raspy-static-bg", &static_source.as_ref().map(|s| format!("url(\"{s}\")")).unwrap_or_else(|| "none".into()));
    toggle_class(&html_root(), "solid-background", solid.is_some());
    toggle_class(&html_root(), "static-background", static_source.is_some());
    let media: &HtmlMediaElement = video.as_ref();
    let source = entry.filter(|b| b.kind == "video").map(|b| b.url);
    let Some(source) = source else {
        let _ = media.pause();
        let _ = video.remove_attribute("src");
        let _ = video.set_attribute("data-source", "");
        media.load();
        return;
    };
    if attr(&video, "data-source").as_deref() != Some(source.as_str()) {
        let _ = video.set_attribute("data-source", &source);
        media.set_src(&source);
        media.load();
    }
    if has_class(&body(), "art-mode") || state.active_id.is_some() || document().hidden() || state.config.reduced_motion {
        let _ = media.pause();
    } else if let Ok(promise) = media.play() {
        ignore_rejection(&promise);
    }
}

// ------------------------------------------------------------------ bandeau Jellyfin

fn load_hero(force: bool) {
    // Chaque rendu redemande le bandeau ; seule la réponse la plus récente s'affiche
    // (le shell garde le résultat en cache, le serveur n'est pas sollicité deux fois).
    let (generation, timers) = app(|a| {
        a.hero_generation += 1;
        (a.hero_generation, [a.hero_timer.take(), a.hero_retry.take()])
    });
    timers.into_iter().for_each(clear);
    call("jellyfin.hero", json!({"force": force}), move |result| {
        let current = app(|a| a.hero_generation == generation && a.page == "home" && a.state.as_ref().is_some_and(|s| s.active_id.is_none()));
        if !current {
            return;
        }
        let Ok(value) = result else {
            schedule_hero_retry();
            return;
        };
        let data: HomeData = serde_json::from_value(value).unwrap_or_default();
        render_media_row("#continue-section", "#continue-row", &data.continue_items, "Continuer");
        render_media_row("#recent-section", "#recent-row", &data.recent_items, "Lire");
        let index = app(|a| {
            a.hero_items = data.items.clone();
            a.hero_index
        });
        if !data.items.is_empty() {
            app(|a| a.hero_retry_delay = 15_000);
            set_hero(index);
            hero_schedule();
        } else if let Some(host) = q("#cinema-hero") {
            host.set_inner_html(&render::hero_unavailable(data.needs_login));
            toggle_class(&host, "is-loading", false);
            if !data.needs_login {
                schedule_hero_retry();
            }
        }
    });
}

fn schedule_hero_retry() {
    if page() != "home" || active_id().is_some() || document().hidden() {
        return;
    }
    let delay = app(|a| {
        let delay = a.hero_retry_delay;
        a.hero_retry_delay = (delay * 2).min(120_000);
        delay
    });
    let timer = set_timeout(|| load_hero(true), delay);
    let old = app(|a| a.hero_retry.replace(timer));
    clear(old);
}

fn render_media_row(section: &str, row: &str, items: &[MediaCard], action: &str) {
    let (Some(section), Some(row)) = (q(section), q(row)) else { return };
    if items.is_empty() {
        let _ = section.set_attribute("hidden", "");
    } else {
        let _ = section.remove_attribute("hidden");
    }
    row.set_inner_html(&items.iter().map(|i| render::media_card(i, action)).collect::<String>());
}

fn preload_neighbours() {
    let sources = app(|a| {
        let n = a.hero_items.len();
        if n < 2 {
            return Vec::new();
        }
        let list: Vec<String> =
            [1, n - 1].iter().map(|offset| a.hero_items[(a.hero_index + offset) % n].backdrop.clone()).filter(|s| !a.preloaded.contains(s)).collect();
        if a.preloaded.len() > 14 {
            a.preloaded.clear();
        }
        a.preloaded.extend(list.iter().cloned());
        list
    });
    for source in sources {
        if let Ok(image) = HtmlImageElement::new() {
            image.set_decoding("async");
            image.set_src(&source);
        }
    }
}

fn set_hero(index: usize) {
    let Some(host) = q("#cinema-hero") else { return };
    let Some((items, index)) = app(|a| {
        if a.hero_items.is_empty() {
            return None;
        }
        a.hero_index = index % a.hero_items.len();
        Some((a.hero_items.clone(), a.hero_index))
    }) else {
        return;
    };
    if let Some(video) = q("#background-video").and_then(|v| v.dyn_into::<HtmlMediaElement>().ok()) {
        let _ = video.pause();
    }
    preload_neighbours();
    toggle_class(&host, "is-changing", true);
    let timer = set_timeout(
        move || {
            let Some(host) = q("#cinema-hero") else { return };
            let _ = host.dyn_ref::<HtmlElement>().expect("hero").style().set_property("background-image", &format!("url(\"{}\")", items[index].backdrop));
            host.set_inner_html(&render::hero_slide(&items, index));
            set_timeout(
                || {
                    if let Some(host) = q("#cinema-hero") {
                        toggle_class(&host, "is-loading", false);
                        toggle_class(&host, "is-changing", false);
                    }
                },
                16,
            );
        },
        160,
    );
    let old = app(|a| a.hero_transition.replace(timer));
    clear(old);
}

fn hero_schedule() {
    let old = app(|a| a.hero_timer.take());
    clear(old);
    let blocked = app(|a| a.page != "home" || a.hero_items.len() < 2 || a.state.as_ref().is_none_or(|s| s.active_id.is_some()));
    if blocked || document().hidden() || has_class(&body(), "art-mode") {
        return;
    }
    let timer = set_timeout(
        || {
            let ok = app(|a| a.page == "home" && a.state.as_ref().is_some_and(|s| s.active_id.is_none()));
            if ok && !document().hidden() && !has_class(&body(), "art-mode") && !modal_open() {
                let next = app(|a| a.hero_index + 1);
                set_hero(next);
            }
            hero_schedule();
        },
        8000,
    );
    app(|a| a.hero_timer = Some(timer));
}

// ------------------------------------------------------------------ jeux, réseau, météo

fn load_moonlight() {
    let old = app(|a| {
        a.moonlight_generation += 1;
        a.moonlight_timer.take()
    });
    clear(old);
    let generation = app(|a| a.moonlight_generation);
    call("moonlight.games", Value::Null, move |result| {
        let current = app(|a| a.page == "home" && a.moonlight_generation == generation && a.state.as_ref().is_some_and(|s| s.active_id.is_none()));
        if current {
            let games: Games = result.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or(Games { unavailable: true, ..Games::default() });
            if let (Some(row), Some(status)) = (q("#moonlight-row"), q("#moonlight-status")) {
                let (text, html) = render::games_row(&games);
                status.set_text_content(Some(&text));
                row.set_inner_html(&html);
            }
            let timer = set_timeout(load_moonlight, 600_000);
            app(|a| a.moonlight_timer = Some(timer));
        }
    });
}

fn paint_network() {
    let Some(host) = q("#network-status") else { return };
    let (class, label, detail) = render::network_parts(app(|a| a.network));
    host.set_class_name(&format!("network-status {class}"));
    host.set_inner_html(&format!("<i></i>{label}"));
    let _ = host.set_attribute("title", detail);
}

fn refresh_network() {
    let old = app(|a| a.network_timer.take());
    clear(old);
    call("network", Value::Null, |result| {
        let next: Network = result.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let was_offline = app(|a| {
            let was = a.network.map(|n| !n.online).unwrap_or(false);
            a.network = Some(next);
            was
        });
        paint_network();
        if was_offline && next.online && page() == "home" && active_id().is_none() {
            load_hero(true);
        }
        let timer = set_timeout(refresh_network, 15_000);
        app(|a| a.network_timer = Some(timer));
    });
}

fn load_weather() {
    call("weather", Value::Null, |result| {
        if let Some(weather) = result.ok().and_then(|v| serde_json::from_value::<Weather>(v).ok()) {
            app(|a| a.weather = Some(weather));
            render();
        }
    });
}

fn load_backgrounds() {
    call("backgrounds.list", Value::Null, |result| {
        let list: Vec<Background> = result.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        if list.is_empty() {
            return;
        }
        app(|a| a.backgrounds = list);
        sync_background();
        if page() == "settings" && app(|a| a.tab == "display") {
            render();
        }
    });
}

fn load_stats() {
    call("stats", Value::Null, |result| {
        if let (Some(host), Some(stats)) = (q("#stats"), result.ok().and_then(|v| serde_json::from_value::<Stats>(v).ok())) {
            host.set_inner_html(&render::stats(&stats));
        }
    });
}

fn load_audio_settings() {
    if q("#audio-settings .audio-system").is_none() {
        return;
    }
    call("audio.get", Value::Null, |result| {
        let Some(host) = q("#audio-settings .audio-system") else { return };
        match result.ok().and_then(|v| serde_json::from_value::<Audio>(v).ok()) {
            Some(audio) => host.set_inner_html(&render::audio_settings(&audio)),
            None => host.set_inner_html(&render::audio_unavailable()),
        }
    });
}

/// Un casque qui se connecte apparaît aussitôt, sauf si l'utilisateur règle déjà le son.
fn refresh_audio_when_idle() {
    if page() != "settings" || q("#audio-settings").is_none() {
        app(|a| a.audio_pending = false);
        return;
    }
    if active_element().and_then(|e| e.closest("#audio-settings").ok().flatten()).is_some() {
        app(|a| a.audio_pending = true);
        return;
    }
    app(|a| a.audio_pending = false);
    load_audio_settings();
}

// ------------------------------------------------------------------ sons, toast, fenêtres

fn play(kind: &str) {
    let Some(state) = state() else { return };
    if !state.config.sounds {
        return;
    }
    if kind == "hover" {
        let t = now();
        let throttled = app(|a| {
            if t - a.last_hover < 200.0 {
                return true;
            }
            a.last_hover = t;
            false
        });
        if throttled {
            return;
        }
    }
    SOUNDS.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            let make = |src: &str| HtmlAudioElement::new_with_src(src).ok();
            if let (Some(hover), Some(open)) = (make("assets/sounds/navigation.wav"), make("assets/sounds/navigation2.wav")) {
                *cell = Some((hover, open));
            }
        }
        let Some((hover, open)) = cell.as_ref() else { return };
        if kind == "open" {
            let _ = hover.pause();
        }
        let audio = if kind == "open" { open } else { hover };
        let _ = audio.pause();
        audio.set_current_time(0.0);
        audio.set_volume((f64::from(state.config.sound_volume) / 100.0).clamp(0.0, 1.0));
        if let Ok(promise) = audio.play() {
            ignore_rejection(&promise);
        }
    });
}

fn show_toast(text: &str) {
    let Some(toast) = q("#toast") else { return };
    toast.set_text_content(Some(text));
    toggle_class(&toast, "show", true);
    let timer = set_timeout(
        || {
            if let Some(t) = q("#toast") {
                toggle_class(&t, "show", false);
            }
        },
        4500,
    );
    let old = app(|a| a.toast_timer.replace(timer));
    clear(old);
}

fn fade(on: bool) {
    if let Some(f) = q("#fade") {
        toggle_class(&f, "show", on);
    }
}

fn show_modal(html: &str) {
    let previous = active_element();
    app(|a| a.modal_return = previous);
    let _ = body().insert_adjacent_html("beforeend", html);
    set_timeout(
        || {
            if let Some(modal) = q(".modal") {
                if let Some(first) = focusables(&modal).into_iter().next() {
                    focus_el(&first);
                }
            }
        },
        16,
    );
}

fn close_modal() -> bool {
    let Some(modal) = q(".modal") else { return false };
    modal.remove();
    if let Some(target) = app(|a| a.modal_return.take()) {
        if target.is_connected() {
            focus_el(&target);
        }
    }
    true
}

fn escape() {
    if close_modal() {
        return;
    }
    if page() == "settings" {
        app(|a| a.page = "home".into());
        render();
    } else {
        call("home", Value::Null, |_| {});
    }
}

// ------------------------------------------------------------------ focus et navigation spatiale

const FOCUSABLE: &str =
    "button:not([disabled]),a[href],input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex=\"-1\"])";
const TOKEN_ATTRS: [&str; 12] = [
    "data-settings-tab",
    "data-jellyfin-play",
    "data-moonlight-game",
    "data-moonlight-open",
    "data-hero-index",
    "data-open",
    "data-close",
    "data-page",
    "data-add",
    "data-art",
    "data-power",
    "data-setting",
];

fn focusables(scope: &Element) -> Vec<Element> {
    qa(scope, FOCUSABLE)
        .into_iter()
        .filter(|el| {
            let r = el.get_bounding_client_rect();
            let visible = window()
                .get_computed_style(el)
                .ok()
                .flatten()
                .map(|css| {
                    css.get_property_value("visibility").unwrap_or_default() != "hidden" && css.get_property_value("display").unwrap_or_default() != "none"
                })
                .unwrap_or(true);
            r.width() > 0.0 && r.height() > 0.0 && visible
        })
        .collect()
}

fn remember_focus() {
    let Some(el) = active_element() else { return };
    let root = root();
    if !root.contains(Some(&el)) {
        return;
    }
    let page = page();
    let token = TOKEN_ATTRS
        .iter()
        .find_map(|name| attr(&el, name).map(|value| FocusToken { attr: Some((name.to_string(), value)), index: None, page: page.clone() }))
        .or_else(|| focusables(&root).iter().position(|x| x == &el).map(|index| FocusToken { attr: None, index: Some(index), page: page.clone() }));
    if token.is_some() {
        app(|a| a.focus = token);
    }
}

fn restore_focus() {
    if active_id().is_some() || q(".modal,.art-gallery").is_some() {
        return;
    }
    let (token, page) = app(|a| (a.focus.clone(), a.page.clone()));
    let root = root();
    let mut target = None;
    if let Some(token) = token.filter(|t| t.page == page) {
        if let Some((name, value)) = &token.attr {
            target = qa(&root, &format!("[{name}]")).into_iter().find(|el| attr(el, name).as_deref() == Some(value.as_str()));
        } else if let Some(index) = token.index {
            target = focusables(&root).into_iter().nth(index);
        }
    }
    let target = target.or_else(|| root.query_selector("[data-jellyfin-play],.tv-grid button,.back,button").ok().flatten());
    if let Some(target) = target {
        focus_el(&target);
    }
}

struct Box2 {
    left: f64,
    right: f64,
    top: f64,
    height: f64,
    width: f64,
}

fn rect(el: &Element) -> Box2 {
    let r = el.get_bounding_client_rect();
    Box2 { left: r.left(), right: r.right(), top: r.top(), height: r.height(), width: r.width() }
}

fn middle(el: &Element) -> f64 {
    let r = rect(el);
    r.top + r.height / 2.0
}

fn go(el: &Element) -> bool {
    focus_el(el);
    scroll_nearest(el, false);
    play("hover");
    true
}

/// Paramètres : haut/bas restent dans la colonne, droite entre dans le réglage
/// en face, gauche revient à l'onglet actif. `None` : laisser la navigation générale.
fn settings_move(direction: &str) -> Option<bool> {
    let current = active_element()?;
    let root = root();
    let tabs = root.query_selector(".settings-tabs").ok().flatten()?;
    let pane = root.query_selector(".settings-pane").ok().flatten()?;
    if modal_open() {
        return None;
    }
    let in_tabs = tabs.contains(Some(&current));
    let in_pane = pane.contains(Some(&current));
    if !in_tabs && !in_pane {
        return None;
    }
    let here = rect(&current);
    let y = middle(&current);
    if direction == "up" || direction == "down" {
        let sign = if direction == "down" { 1.0 } else { -1.0 };
        let mut next: Vec<Element> =
            focusables(if in_tabs { &tabs } else { &pane }).into_iter().filter(|el| el != &current && (middle(el) - y) * sign > 4.0).collect();
        next.sort_by(|a, b| {
            let da = ((middle(a) - y).abs(), (rect(a).left - here.left).abs());
            let db = ((middle(b) - y).abs(), (rect(b).left - here.left).abs());
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        return next.first().map(go);
    }
    let same_row = |el: &Element| (middle(el) - y).abs() < rect(el).height.max(here.height) / 2.0 + 6.0;
    if in_tabs {
        if direction == "left" {
            return Some(false);
        }
        let mut targets = focusables(&pane);
        targets.sort_by(|a, b| ((middle(a) - y).abs(), rect(a).left).partial_cmp(&((middle(b) - y).abs(), rect(b).left)).unwrap_or(std::cmp::Ordering::Equal));
        return Some(targets.first().map(go).unwrap_or(false));
    }
    let mut row: Vec<Element> = focusables(&pane)
        .into_iter()
        .filter(|el| el != &current && same_row(el) && if direction == "left" { rect(el).right <= here.left + 2.0 } else { rect(el).left >= here.right - 2.0 })
        .collect();
    row.sort_by(|a, b| {
        let (ka, kb) = if direction == "left" { (-rect(a).right, -rect(b).right) } else { (rect(a).left, rect(b).left) };
        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(first) = row.first() {
        return Some(go(first));
    }
    if direction == "left" {
        let target = tabs.query_selector(".settings-tab.active").ok().flatten().or_else(|| focusables(&tabs).into_iter().next());
        return Some(target.map(|t| go(&t)).unwrap_or(false));
    }
    Some(false)
}

fn spatial_move(direction: &str) -> bool {
    if page() == "settings" {
        if let Some(handled) = settings_move(direction) {
            return handled;
        }
    }
    let scope = q(".modal").unwrap_or_else(root);
    let items = focusables(&scope);
    let current = active_element().filter(|el| items.contains(el));
    let Some(current) = current else {
        return match items.first() {
            Some(first) => {
                focus_el(first);
                true
            }
            None => false,
        };
    };
    let a = rect(&current);
    let (ax, ay) = (a.left + a.width / 2.0, a.top + a.height / 2.0);
    let inner_height = window().inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(1080.0);
    if a.top + a.height <= 0.0 || a.top >= inner_height {
        scroll_nearest(&current, false);
        return true;
    }
    let horizontal = direction == "left" || direction == "right";
    let mut winner: Option<&Element> = None;
    let mut score = f64::INFINITY;
    for item in items.iter().filter(|el| *el != &current) {
        let b = rect(item);
        let (dx, dy) = (b.left + b.width / 2.0 - ax, b.top + b.height / 2.0 - ay);
        let valid = match direction {
            "left" => dx < -2.0,
            "right" => dx > 2.0,
            "up" => dy < -2.0,
            _ => dy > 2.0,
        };
        if !valid {
            continue;
        }
        let (main, cross) = if horizontal { (dx.abs(), dy.abs()) } else { (dy.abs(), dx.abs()) };
        let value = main + cross * 2.4;
        if value < score {
            score = value;
            winner = Some(item);
        }
    }
    let Some(winner) = winner else { return false };
    focus_el(winner);
    let smooth = state().is_some_and(|s| !s.config.reduced_motion);
    scroll_nearest(winner, smooth);
    play("hover");
    true
}

fn handle_key(event: KeyboardEvent) {
    if event.default_prevented() {
        return;
    }
    let key = event.key();
    // Ctrl+Alt+Maj+Q : quitter RaspyTV (arrêt propre, la boucle kiosque s'arrête aussi).
    if event.ctrl_key() && event.alt_key() && event.shift_key() && key.eq_ignore_ascii_case("q") {
        event.prevent_default();
        notify("quit", Value::Null);
        return;
    }
    let target = event.target().and_then(|t| t.dyn_into::<Element>().ok());
    match key.as_str() {
        "Home" => {
            event.prevent_default();
            app(|a| a.page = "home".into());
            render();
            return;
        }
        "Escape" => {
            event.prevent_default();
            escape();
            return;
        }
        "Enter" => {
            if let Some(t) = target.as_ref().filter(|t| matches(t, "input[type=checkbox]")) {
                event.prevent_default();
                click_el(t);
            }
            return;
        }
        _ => {}
    }
    let direction = match key.as_str() {
        "ArrowLeft" => "left",
        "ArrowRight" => "right",
        "ArrowUp" => "up",
        "ArrowDown" => "down",
        _ => return,
    };
    if let Some(t) = &target {
        if matches(t, "textarea,input:not([type=checkbox]):not([type=radio]):not([type=range])") {
            return;
        }
        if matches(t, "input[type=range]") && (direction == "left" || direction == "right") {
            return;
        }
    }
    if spatial_move(direction) || target.as_ref().is_some_and(|t| matches(t, "select")) {
        event.prevent_default();
    }
}

// ------------------------------------------------------------------ actions

fn launch(id: String) {
    if app(|a| std::mem::replace(&mut a.launching, true)) {
        return;
    }
    play("open");
    fade(true);
    set_timeout(
        move || {
            call("app.launch", json!(id), |result| {
                fade(false);
                app(|a| a.launching = false);
                if let Err(error) = result {
                    show_toast(&error);
                }
            });
        },
        200,
    );
}

fn launch_moonlight(method: &'static str, params: Value) {
    if app(|a| std::mem::replace(&mut a.launching, true)) {
        return;
    }
    play("open");
    fade(true);
    set_timeout(
        move || {
            call(method, params, |result| {
                if let Err(error) = result {
                    show_toast(if error.is_empty() { "Moonlight indisponible" } else { &error });
                }
                set_timeout(|| fade(false), 220);
                app(|a| a.launching = false);
            });
        },
        160,
    );
}

fn on_click(event: MouseEvent) {
    let target = event.target();
    // Mode Tableau : un clic n'importe où le referme.
    if app(|a| a.gallery.open) {
        event.prevent_default();
        event.stop_immediate_propagation();
        close_gallery();
        return;
    }
    if closest(&target, "[data-art]").is_some() {
        event.prevent_default();
        event.stop_immediate_propagation();
        open_gallery();
        return;
    }
    let Some(b) = closest(&target, "button") else { return };
    let data = |name: &str| attr(&b, name);
    if let Some(tab) = data("data-settings-tab") {
        if app(|a| a.tab != tab) {
            app(|a| a.tab = tab);
            play("hover");
            render();
        }
        return;
    }
    if let Some(action) = data("data-action") {
        if let Some(modal) = q(".modal") {
            modal.remove();
        }
        show_modal(&render::power_confirm(&action));
        return;
    }
    if let Some(action) = data("data-power-confirm") {
        if let Some(modal) = b.closest(".modal").ok().flatten() {
            modal.remove();
        }
        call("system.action", json!(action), |result| {
            if let Err(error) = result {
                show_toast(&error);
            }
        });
        return;
    }
    if let Some(id) = data("data-remove-shortcut") {
        let name = state().and_then(|s| s.apps.into_iter().find(|a| a.id == id && !a.native)).map(|a| a.name);
        let Some(name) = name else {
            show_toast("Cette application native est protégée");
            return;
        };
        let _ = b.set_attribute("disabled", "");
        call("shortcut.remove", json!(id), move |result| match result {
            Ok(_) => {
                close_modal();
                show_toast(&format!("{name} a été supprimée"));
            }
            Err(error) => {
                if let Some(b) = q("[data-remove-shortcut]") {
                    let _ = b.remove_attribute("disabled");
                }
                show_toast(&error);
            }
        });
        return;
    }
    if data("data-hero-retry").is_some() {
        return load_hero(true);
    }
    if data("data-audio-refresh").is_some() {
        return load_audio_settings();
    }
    if data("data-moonlight-open").is_some() {
        return launch_moonlight("moonlight.open", Value::Null);
    }
    if let Some(name) = data("data-moonlight-game") {
        return launch_moonlight("moonlight.play", json!(name));
    }
    if let Some(id) = data("data-jellyfin-play") {
        play("open");
        fade(true);
        call("jellyfin.play", json!(id), |result| {
            set_timeout(|| fade(false), 280);
            if let Err(error) = result {
                show_toast(&error);
            }
        });
        return;
    }
    if let Some(index) = data("data-hero-index") {
        set_hero(index.parse().unwrap_or(0));
        hero_schedule();
        return;
    }
    if let Some(id) = data("data-open") {
        return launch(id);
    }
    if let Some(id) = data("data-close") {
        call("app.close", json!(id), |_| {});
        return;
    }
    if data("data-home").is_some() {
        app(|a| a.page = "home".into());
        render();
    }
    if let Some(target) = data("data-page") {
        app(|a| a.page = target);
        render();
    }
    if data("data-add").is_some() {
        show_modal(&render::shortcut_dialog());
    }
    if data("data-dismiss").is_some() {
        close_modal();
    }
    if data("data-close-all").is_some() {
        for app_item in state().map(|s| s.apps).unwrap_or_default().into_iter().filter(|a| a.open) {
            call("app.close", json!(app_item.id), |_| {});
        }
    }
}

fn on_change(event: Event) {
    let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return };
    if target.has_attribute("data-audio-output") {
        let Some(select) = target.dyn_ref::<HtmlSelectElement>() else { return };
        let value = select.value();
        if value.starts_with("bt:") {
            let name = select.selected_options().item(0).and_then(|o| o.text_content()).unwrap_or_default();
            show_toast(&format!("Connexion à {}…", name.split(" · ").next().unwrap_or("l’appareil")));
        }
        call("audio.output", json!(value), |result| match result {
            Ok(_) => {
                show_toast("Sortie audio activée");
                load_audio_settings();
            }
            Err(error) => show_toast(&error),
        });
        return;
    }
    if target.has_attribute("data-system-volume") {
        let Some(input) = target.dyn_ref::<HtmlInputElement>() else { return };
        let value: f64 = input.value().parse().unwrap_or(0.0);
        if let Some(output) = target.next_element_sibling() {
            output.set_text_content(Some(&format!("{value}%")));
        }
        call("audio.volume", json!(value), |result| {
            if let Err(error) = result {
                show_toast(&error);
            }
        });
        return;
    }
    let Some(field) = attr(&target, "data-setting") else { return };
    let value = if let Some(input) = target.dyn_ref::<HtmlInputElement>() {
        match input.type_().as_str() {
            "checkbox" => json!(input.checked()),
            "range" => {
                let v: f64 = input.value().parse().unwrap_or(0.0);
                if let Some(output) = target.next_element_sibling() {
                    output.set_text_content(Some(&format!("{v}%")));
                }
                json!(v)
            }
            _ => json!(input.value()),
        }
    } else if let Some(select) = target.dyn_ref::<HtmlSelectElement>() {
        let v = select.value();
        v.parse::<f64>().map(|n| json!(n)).unwrap_or(json!(v))
    } else {
        return;
    };
    call("settings.save", json!({ field: value }), |result| {
        if let Ok(next) = result {
            if let Ok(next) = serde_json::from_value::<State>(next) {
                accept_state(next);
            }
        }
    });
}

fn on_submit(event: Event) {
    let Some(form) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlFormElement>().ok()) else { return };
    if form.id() != "shortcut-form" {
        return;
    }
    event.prevent_default();
    let Ok(data) = web_sys::FormData::new_with_form(&form) else { return };
    let name = data.get("name").as_string().unwrap_or_default();
    let url = data.get("url").as_string().unwrap_or_default();
    if let Some(submit) = form.query_selector("button.primary").ok().flatten() {
        let _ = submit.set_attribute("disabled", "");
        submit.set_text_content(Some("Ajout…"));
    }
    call("shortcut.add", json!({"name": name, "url": url}), |result| match result {
        Ok(_) => {
            close_modal();
        }
        Err(error) => {
            if let Some(form) = q("#shortcut-form") {
                if let Some(message) = form.query_selector(".form-error").ok().flatten() {
                    message.set_text_content(Some(&error));
                    let _ = message.remove_attribute("hidden");
                }
                if let Some(submit) = form.query_selector("button.primary").ok().flatten() {
                    let _ = submit.remove_attribute("disabled");
                    submit.set_text_content(Some("Ajouter au Home"));
                }
            }
        }
    });
}

fn on_context_menu(event: MouseEvent) {
    let Some(card) = closest(&event.target(), ".app-card[data-open]") else { return };
    event.prevent_default();
    if !card.has_attribute("data-custom") {
        return;
    }
    let id = attr(&card, "data-open").unwrap_or_default();
    if let Some(item) = state().and_then(|s| s.apps.into_iter().find(|a| a.id == id && !a.native)) {
        show_modal(&render::delete_shortcut_dialog(&item));
    }
}

fn on_wheel(event: WheelEvent) {
    let Some(rail) = closest(&event.target(), ".tv-grid,.continue-row,.game-row") else { return };
    if event.delta_x().abs() >= event.delta_y().abs() || rail.scroll_width() <= rail.client_width() {
        return;
    }
    event.prevent_default();
    rail.set_scroll_left(rail.scroll_left() + event.delta_y() as i32);
}

const EDITABLE: &str =
    "input:not([type=button]):not([type=submit]):not([type=checkbox]):not([type=radio]):not([type=range]),textarea,[contenteditable=true],[role=textbox]";

fn is_editable(el: Option<Element>) -> bool {
    el.and_then(|e| e.closest(EDITABLE).ok().flatten()).is_some()
}

// ------------------------------------------------------------------ mode Tableau

fn open_gallery() {
    if app(|a| a.gallery.open) {
        return;
    }
    let previous = active_element();
    let generation = app(|a| {
        a.previous_focus = previous;
        a.gallery.open = true;
        a.gallery.closing = false;
        a.gallery.generation += 1;
        a.gallery.generation
    });
    toggle_class(&body(), "art-mode", true);
    notify("art.mode", json!(true));
    if let Some(video) = q("#background-video").and_then(|v| v.dyn_into::<HtmlMediaElement>().ok()) {
        let _ = video.pause();
    }
    let layer = document().create_element("div").expect("div");
    layer.set_class_name("art-gallery");
    let _ = layer.set_attribute("tabindex", "-1");
    let _ = layer.set_attribute("role", "dialog");
    let _ = layer.set_attribute("aria-modal", "true");
    let _ = layer.set_attribute("aria-label", "Mode Tableau. Cliquez ou appuyez sur Échap pour revenir.");
    let _ = body().append_child(&layer);
    focus_el(&layer);
    set_timeout(
        || {
            if let Some(layer) = q(".art-gallery") {
                toggle_class(&layer, "visible", true);
            }
        },
        32,
    );
    call("art.list", Value::Null, move |result| {
        let files: Vec<String> = result.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        if app(|a| a.gallery.generation != generation) {
            return;
        }
        if files.is_empty() {
            if let Some(layer) = q(".art-gallery") {
                layer.set_inner_html(r#"<p class="art-empty">Aucun tableau. Dépose des images dans ~/.config/raspytv-rs/art.</p>"#);
            }
            return;
        }
        let index = app(|a| {
            a.gallery.files = files;
            a.gallery.index % a.gallery.files.len()
        });
        display_art(index);
    });
}

fn schedule_art() {
    let old = app(|a| a.gallery.timer.take());
    clear(old);
    if !app(|a| a.gallery.open && !a.gallery.closing && !a.gallery.files.is_empty()) {
        return;
    }
    let timer = set_timeout(
        || {
            let next = app(|a| (a.gallery.index + 1) % a.gallery.files.len().max(1));
            display_art(next);
        },
        300_000,
    );
    app(|a| a.gallery.timer = Some(timer));
}

fn display_art(next: usize) {
    let Some((source, generation)) = app(|a| a.gallery.files.get(next).cloned().map(|s| (s, a.gallery.generation))) else { return };
    let Ok(image) = HtmlImageElement::new() else { return };
    image.set_alt(&format!("Tableau {}", next + 1));
    image.set_draggable(false);
    image.set_src(&source);
    let promise = image.decode();
    wasm_bindgen_futures::spawn_local(async move {
        let decoded = wasm_bindgen_futures::JsFuture::from(promise).await.is_ok();
        if !decoded {
            schedule_art();
            return;
        }
        let ok = app(|a| a.gallery.open && !a.gallery.closing && a.gallery.generation == generation);
        let Some(layer) = q(".art-gallery").filter(|_| ok) else { return };
        let old = layer.query_selector("img").ok().flatten();
        let _ = layer.append_child(&image);
        app(|a| a.gallery.index = next);
        set_timeout(
            move || {
                toggle_class(&image, "visible", true);
                if let Some(old) = &old {
                    toggle_class(old, "visible", false);
                    let old = old.clone();
                    set_timeout(move || old.remove(), 900);
                }
            },
            32,
        );
        schedule_art();
    });
}

fn close_gallery() {
    let started = app(|a| {
        if !a.gallery.open || a.gallery.closing {
            return None;
        }
        a.gallery.closing = true;
        a.gallery.generation += 1;
        Some(a.gallery.timer.take())
    });
    let Some(timer) = started else { return };
    clear(timer);
    toggle_class(&body(), "art-mode", false);
    sync_background();
    if let Some(layer) = q(".art-gallery") {
        toggle_class(&layer, "visible", false);
    }
    set_timeout(
        || {
            if let Some(layer) = q(".art-gallery") {
                layer.remove();
            }
            let previous = app(|a| {
                a.gallery.open = false;
                a.gallery.closing = false;
                a.previous_focus.take()
            });
            notify("art.mode", json!(false));
            hero_schedule();
            if let Some(previous) = previous.filter(|p| p.is_connected()) {
                focus_el(&previous);
            }
        },
        500,
    );
}

// ------------------------------------------------------------------ démarrage

fn listen<E: JsCast + 'static>(name: &str, capture: bool, passive: Option<bool>, handler: impl Fn(E) + 'static) {
    let closure = Closure::<dyn Fn(Event)>::new(move |event: Event| {
        if let Ok(event) = event.dyn_into::<E>() {
            handler(event);
        }
    });
    let options = AddEventListenerOptions::new();
    options.set_capture(capture);
    if let Some(passive) = passive {
        options.set_passive(passive);
    }
    let _ = document().add_event_listener_with_callback_and_add_event_listener_options(name, closure.as_ref().unchecked_ref(), &options);
    closure.forget();
}

/// Démonstration : `?section=apps` fait défiler jusqu'aux applications, `?dialog=reboot`
/// ouvre la confirmation d'alimentation (captures du README).
fn demo_extras() {
    if let Some(section) = demo_param("section") {
        set_timeout(
            move || {
                if let Some(el) = q(&format!(".{section}-section")) {
                    let root = root();
                    if let Some(html) = root.dyn_ref::<HtmlElement>() {
                        let _ = html.style().set_property("scroll-behavior", "auto");
                    }
                    let top = el.get_bounding_client_rect().top() + f64::from(root.scroll_top()) - 40.0;
                    root.set_scroll_top(top as i32);
                }
            },
            2500,
        );
    }
    if let Some(action) = demo_param("dialog") {
        set_timeout(move || show_modal(&render::power_confirm(&action)), 900);
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    if let Some(page) = demo_param("page").filter(|p| p == "settings") {
        app(|a| a.page = page);
    }
    if let Some(tab) = demo_param("tab") {
        app(|a| a.tab = tab);
    }
    listen::<MouseEvent>("click", true, None, on_click);
    listen::<Event>("change", false, None, on_change);
    listen::<Event>("submit", false, None, on_submit);
    listen::<MouseEvent>("contextmenu", false, None, on_context_menu);
    listen::<WheelEvent>("wheel", false, Some(false), on_wheel);
    listen::<KeyboardEvent>("keydown", true, None, |event| {
        if !app(|a| a.gallery.open) {
            return;
        }
        event.prevent_default();
        event.stop_immediate_propagation();
        if matches!(event.key().as_str(), "Escape" | "Home" | "Enter" | " ") {
            close_gallery();
        }
    });
    listen::<KeyboardEvent>("keydown", false, None, handle_key);
    listen::<MouseEvent>("pointerover", false, None, |event| {
        if let Some(button) = closest(&event.target(), "button") {
            let inside = event.related_target().and_then(|t| t.dyn_into::<web_sys::Node>().ok()).is_some_and(|n| button.contains(Some(&n)));
            if !inside {
                play("hover");
            }
        }
    });
    listen::<Event>("focusin", true, None, |event| {
        if is_editable(event.target().and_then(|t| t.dyn_into::<Element>().ok())) {
            notify("keyboard.focus", json!(true));
        }
    });
    listen::<Event>("focusout", true, None, |_| {
        set_timeout(|| notify("keyboard.focus", json!(is_editable(active_element()))), 180);
        if app(|a| a.audio_pending) {
            set_timeout(refresh_audio_when_idle, 0);
        }
    });
    listen::<Event>("visibilitychange", false, None, |_| {
        if state().is_none() {
            return;
        }
        sync_background();
        let empty = app(|a| a.hero_items.is_empty());
        if !document().hidden() && empty && page() == "home" && active_id().is_none() {
            load_hero(true);
        } else {
            hero_schedule();
        }
    });
    // Horloge et date, chaque seconde.
    let tick = Closure::<dyn Fn()>::new(|| {
        let clock24 = state().map(|s| s.config.clock24).unwrap_or(true);
        if let Some(c) = q("#clock") {
            c.set_text_content(Some(&time_text(clock24)));
        }
        if let Some(d) = q("#date") {
            d.set_text_content(Some(&date_text()));
        }
    });
    let _ = window().set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), 1000);
    tick.forget();
    if !has_ipc() {
        toggle_class(&html_root(), "demo", true);
    }
    call("state", Value::Null, |result| {
        if let Some(state) = result.ok().and_then(|v| serde_json::from_value::<State>(v).ok()) {
            accept_state(state);
            refresh_network();
            load_weather();
            load_backgrounds();
            if !has_ipc() {
                demo_extras();
            }
        }
    });
}
