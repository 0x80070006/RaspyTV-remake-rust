//! État et comportement de RaspyTV (équivalent de `main.js`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use raspytv_core::apps::{self, App as Service};
use raspytv_core::audio::{self, BluetoothDevice};
use raspytv_core::config::Config;
use raspytv_core::jellyfin::{self, HomeData, Session};
use raspytv_core::moonlight::{self, Games};
use raspytv_core::paths::Paths;
use raspytv_core::protocol::{Command, LayerState, MediaInfo, NavApp, PanelAudio, PanelOutput, PanelState};
use raspytv_core::site::Site;
use raspytv_core::{hero, media, rlog, sys, vault};
use serde_json::{json, Value};
use tao::event_loop::EventLoopProxy;
use tao::window::{Fullscreen, Window};
use wry::{WebContext, WebView, WebViewBuilder};

use crate::host::Host;
use crate::protocol::{self, Shared};

pub type Job = Box<dyn FnOnce(&mut App) + Send>;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum UserEvent {
    /// Suite d'un travail en arrière-plan, exécutée sur le fil principal.
    Run(Job),
    /// Message de l'accueil (`window.ipc.postMessage`).
    HomeIpc(String),
    /// Message d'un service (seuls quelques messages sont acceptés).
    ServiceIpc {
        id: String,
        body: String,
    },
    PageLoad {
        id: String,
        finished: bool,
        url: String,
    },
    Command(Command),
    /// Touche de la manette (gilrs).
    Gamepad(String),
    /// Touche du clavier virtuel.
    Osk(OskKey),
}

#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum OskKey {
    Text(String),
    Key(String),
    Close,
}

#[derive(Debug, Clone, PartialEq)]
enum Place {
    Home,
    Settings,
    App(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Target {
    Home,
    View(String),
}

struct ServiceView {
    service: Service,
    webview: WebView,
    /// Le vrai contenu a fini de charger (pas une page technique intermédiaire).
    ready: bool,
    /// Jellyfin : page de restauration de session en cours.
    restoring: bool,
    /// Lecture Jellyfin demandée avant la fin du chargement.
    pending: Option<String>,
}

#[derive(Default)]
struct HeroCache {
    data: Option<HomeData>,
    at: Option<Instant>,
    day: String,
}

#[derive(Default)]
struct MoonlightCache {
    data: Option<Games>,
    at: Option<Instant>,
}

#[derive(Default)]
struct BluetoothCache {
    at: Option<Instant>,
    list: Vec<BluetoothDevice>,
}

#[derive(Default)]
struct Probe {
    generation: u64,
    expected: usize,
    results: Vec<(String, Value)>,
    done: bool,
    then: Option<Job>,
}

pub struct App {
    pub proxy: EventLoopProxy<UserEvent>,
    pub window: Window,
    host: Host,
    home: WebView,
    loading: Option<WebView>,
    pub paths: Paths,
    site: Site,
    defaults: Vec<Service>,
    config: Config,
    contexts: HashMap<String, WebContext>,
    views: HashMap<String, ServiceView>,
    last_used: HashMap<String, Instant>,
    active_id: Option<String>,
    shell_page: String,
    tv_return_home: Option<Place>,
    tv_return_settings: Option<Place>,
    nav_peek_open: bool,
    panel_open: bool,
    panel_generation: u64,
    art_mode: bool,
    native: Option<u32>,
    system_volume: Option<u8>,
    system_muted: bool,
    last_audio_key: String,
    keyboard_target: Option<Target>,
    keyboard_generation: u64,
    system_keyboard: bool,
    shared: Shared,
    hero_cache: Arc<Mutex<HeroCache>>,
    moonlight_cache: Arc<Mutex<MoonlightCache>>,
    bluetooth: Arc<Mutex<BluetoothCache>>,
    loading_shown_at: Instant,
    loading_generation: u64,
    panel_source: Option<String>,
    panel_art: Arc<Mutex<(String, PathBuf)>>,
    probe: Probe,
    tv_adapter: String,
    noutube: String,
    #[cfg(target_os = "linux")]
    pub layer: Option<crate::layer::Layer>,
    pub quitting: bool,
}

/// Le navigateur intégré s'annonce comme un Chrome ordinaire : Google refuse la
/// connexion aux navigateurs qu'il identifie comme « intégrés ».
const CHROME_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

pub const ENGINE: &str = if cfg!(target_os = "linux") { "WebKitGTK" } else { "WebView2" };

fn read_inject(paths: &Paths, name: &str) -> String {
    std::fs::read_to_string(paths.web.join("inject").join(name)).unwrap_or_else(|error| {
        rlog!("script {name} indisponible : {error}");
        String::new()
    })
}

impl App {
    pub fn new(window: Window, proxy: EventLoopProxy<UserEvent>, paths: Paths) -> Self {
        let site = Site::load(&paths);
        let defaults = apps::default_apps(&site);
        let config = Config::load(&paths.config_file(), &defaults);
        let session = vault::load(&paths.vault(), &paths.vault_key());
        let shared = Shared {
            web: paths.web.clone(),
            icons: paths.icons(),
            art: paths.art_dirs().to_vec(),
            backgrounds: paths.background_dirs().to_vec(),
            jellyfin_origin: site.jellyfin_origin(),
            session: Arc::new(RwLock::new(session)),
        };
        let host = Host::new(&window);
        let home_dir = paths.web_data("home");
        raspytv_core::fsutil::ensure_private_dir(&home_dir);
        let mut home_context = WebContext::new(Some(home_dir));
        let home = Self::build_home(&host, &window, &proxy, &shared, &mut home_context);
        let loading = WebViewBuilder::new_with_web_context(&mut home_context)
            .with_url(protocol::home_url("loading.html"))
            .with_background_color((7, 9, 16, 255))
            .with_custom_protocol(protocol::SCHEME.into(), {
                let shared = shared.clone();
                move |_id, request| protocol::handle(&shared, request)
            });
        let loading = host.build_loading(loading, &window).map_err(|e| rlog!("loading view failure {e}")).ok();
        host.present("home", &[("home", &home)]);
        let tv_adapter = read_inject(&paths, "tv-input-adapter.js");
        let noutube = read_inject(&paths, "noutube/noutube-content.js");
        let mut app = Self {
            proxy,
            window,
            host,
            home,
            loading,
            site,
            defaults,
            config,
            contexts: HashMap::from([("home".to_string(), home_context)]),
            views: HashMap::new(),
            last_used: HashMap::new(),
            active_id: None,
            shell_page: "home".into(),
            tv_return_home: None,
            tv_return_settings: None,
            nav_peek_open: false,
            panel_open: false,
            panel_generation: 0,
            art_mode: false,
            native: None,
            system_volume: None,
            system_muted: false,
            last_audio_key: String::new(),
            keyboard_target: None,
            keyboard_generation: 0,
            system_keyboard: false,
            shared,
            hero_cache: Arc::default(),
            moonlight_cache: Arc::default(),
            bluetooth: Arc::default(),
            loading_shown_at: Instant::now(),
            loading_generation: 0,
            panel_source: None,
            panel_art: Arc::new(Mutex::new((String::new(), PathBuf::new()))),
            probe: Probe::default(),
            tv_adapter,
            noutube,
            #[cfg(target_os = "linux")]
            layer: None,
            quitting: false,
            paths,
        };
        app.write_layer_state();
        app.refresh_audio();
        app.import_youtube_session();
        app
    }

    fn build_home(host: &Host, window: &Window, proxy: &EventLoopProxy<UserEvent>, shared: &Shared, context: &mut WebContext) -> WebView {
        let ipc_proxy = proxy.clone();
        let protocol_shared = shared.clone();
        let builder = WebViewBuilder::new_with_web_context(context)
            .with_url(protocol::home_url("index.html"))
            .with_background_color((3, 5, 11, 255))
            .with_devtools(cfg!(debug_assertions))
            .with_autoplay(true)
            .with_custom_protocol(protocol::SCHEME.into(), move |_id, request| protocol::handle(&protocol_shared, request))
            .with_ipc_handler(move |request| {
                if protocol::is_home_url(&request.uri().to_string()) {
                    let _ = ipc_proxy.send_event(UserEvent::HomeIpc(request.into_body()));
                }
            })
            // L'accueil ne quitte jamais ses propres pages.
            .with_navigation_handler(|url| protocol::is_home_url(&url) || url.starts_with("about:"))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny);
        host.build("home", builder, window).expect("vue d'accueil")
    }

    // ------------------------------------------------------------------ outils

    pub fn spawn<R: Send + 'static>(&self, work: impl FnOnce() -> R + Send + 'static, then: impl FnOnce(&mut App, R) + Send + 'static) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = work();
            let _ = proxy.send_event(UserEvent::Run(Box::new(move |app| then(app, result))));
        });
    }

    pub fn after(&self, delay: Duration, then: impl FnOnce(&mut App) + Send + 'static) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = proxy.send_event(UserEvent::Run(Box::new(then)));
        });
    }

    /// Exécute un script dans une vue et rappelle `then` avec le résultat (JSON brut).
    fn eval_view(&self, id: &str, script: &str, then: impl FnOnce(&mut App, String) + Send + 'static) {
        let Some(view) = self.views.get(id) else { return };
        let proxy = self.proxy.clone();
        let then = Mutex::new(Some(then));
        let result = view.webview.evaluate_script_with_callback(script, move |value| {
            if let Some(then) = then.lock().ok().and_then(|mut t| t.take()) {
                let _ = proxy.send_event(UserEvent::Run(Box::new(move |app| then(app, value))));
            }
        });
        if let Err(error) = result {
            rlog!("script failure {error}");
        }
    }

    fn eval(&self, id: &str, script: &str) {
        if let Some(view) = self.views.get(id) {
            let _ = view.webview.evaluate_script(script);
        }
    }

    fn send_home(&self, message: Value) {
        let text = Value::String(message.to_string());
        let _ = self.home.evaluate_script(&format!("window.__raspyHome&&window.__raspyHome.receive({text})"));
    }

    pub fn reply(&self, id: Option<u64>, result: Result<Value, String>) {
        let Some(id) = id else { return };
        let message = match result {
            Ok(value) => json!({"t": "reply", "id": id, "ok": true, "value": value}),
            Err(error) => json!({"t": "reply", "id": id, "ok": false, "error": error}),
        };
        self.send_home(message);
    }

    pub fn toast(&self, text: &str) {
        self.send_home(json!({"t": "toast", "text": text}));
    }

    fn all_apps(&self) -> Vec<Service> {
        apps::all_apps(&self.defaults, &self.config)
    }

    fn app_by_id(&self, id: &str) -> Option<Service> {
        self.all_apps().into_iter().find(|a| a.id == id)
    }

    fn is_background(&self, id: &str) -> bool {
        self.app_by_id(id).is_some_and(|a| a.background)
    }

    fn save_config(&mut self) {
        if let Err(error) = self.config.save(&self.paths.config_file()) {
            rlog!("config save failure {error}");
            self.toast("Impossible d’enregistrer les paramètres.");
        }
        self.emit_state();
    }

    pub fn state_json(&self) -> Value {
        let apps: Vec<Value> = self
            .all_apps()
            .into_iter()
            .map(|a| {
                let open = self.views.contains_key(&a.id);
                let mut value = serde_json::to_value(&a).unwrap_or(Value::Null);
                value["open"] = json!(open);
                value
            })
            .collect();
        json!({
            "config": self.config.to_value(),
            "apps": apps,
            "activeId": self.active_id,
            "nativeMode": self.native.is_some(),
            "artMode": self.art_mode,
            "site": {"weatherLabel": self.site.weather_label, "moonlight": !self.site.moonlight_host.is_empty()},
        })
    }

    pub fn emit_state(&mut self) {
        self.write_layer_state();
        self.send_home(json!({"t": "state", "value": self.state_json()}));
    }

    fn layer_state(&self) -> LayerState {
        let web = self.paths.web.clone();
        LayerState {
            active_id: self.active_id.clone(),
            native_mode: self.native.is_some(),
            art_mode: self.art_mode,
            system_volume: self.system_volume,
            system_muted: self.system_muted,
            apps: self
                .all_apps()
                .into_iter()
                .map(|a| NavApp {
                    icon: if a.icon.starts_with("assets/") {
                        web.join(&a.icon).display().to_string()
                    } else {
                        a.icon.strip_prefix("file://").unwrap_or("").to_string()
                    },
                    id: a.id,
                    name: a.name,
                    native: a.native,
                    color: if a.color.is_empty() { "#4f8cff".into() } else { a.color },
                })
                .collect(),
        }
    }

    /// Publie l'état pour la barre latérale (et `nav-state.json`, pour les outils externes).
    pub fn write_layer_state(&mut self) {
        let state = self.layer_state();
        let _ = raspytv_core::fsutil::atomic_write(&self.paths.nav_state(), serde_json::to_string(&state).unwrap_or_default().as_bytes());
        #[cfg(target_os = "linux")]
        if let Some(layer) = &self.layer {
            layer.update(&state);
        }
    }

    // ------------------------------------------------------------------ affichage

    fn present(&self, name: &str) {
        let mut list: Vec<(&str, &WebView)> = vec![("home", &self.home)];
        list.extend(self.views.iter().map(|(id, v)| (id.as_str(), &v.webview)));
        self.host.present(name, &list);
    }

    pub fn resize(&self) {
        let mut list: Vec<&WebView> = vec![&self.home];
        list.extend(self.views.values().map(|v| &v.webview));
        list.extend(self.loading.as_ref());
        self.host.resize(&self.window, &list);
    }

    fn show_loading(&mut self, label: &str) {
        self.loading_generation += 1;
        self.loading_shown_at = Instant::now();
        if let Some(view) = &self.loading {
            let label = Value::String(label.chars().take(80).collect());
            let _ = view.evaluate_script(&format!("document.querySelector('[data-label]').textContent={label}"));
        }
        self.host.loading(true, self.loading.as_ref());
    }

    /// Au moins 360 ms d'écran de chargement : pas de clignotement.
    fn hide_loading_soon(&mut self, delay: u64) {
        let elapsed = self.loading_shown_at.elapsed().as_millis() as u64;
        let remaining = 360u64.saturating_sub(elapsed).max(delay);
        let generation = self.loading_generation;
        self.after(Duration::from_millis(remaining), move |app| {
            if app.loading_generation == generation {
                app.host.loading(false, app.loading.as_ref());
            }
        });
    }

    fn pause_view(&self, id: &str) {
        self.eval(id, &media::mute_script(true));
        self.eval(id, media::PAUSE_SCRIPT);
    }

    /// Une application d'arrière-plan (NouTube Music) continue de jouer quand on la quitte…
    fn pause_active_media(&self) {
        if let Some(id) = &self.active_id {
            if !self.is_background(id) {
                self.pause_view(id);
            }
        }
    }

    /// … mais se tait dès qu'une autre source de son prend la main.
    fn pause_background_media(&self, except: Option<&str>) {
        for id in self.views.keys() {
            if Some(id.as_str()) != except && self.is_background(id) {
                self.pause_view(id);
            }
        }
    }

    fn restore_fullscreen(&self) {
        self.window.set_visible(true);
        self.window.set_maximized(false);
        self.window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        self.window.set_focus();
    }

    pub fn show_home(&mut self) {
        let from_jellyfin = self.active_id.as_deref() == Some("jellyfin");
        self.show_loading("Accueil RaspyTV");
        self.stop_native();
        self.hide_keyboard();
        self.pause_active_media();
        if from_jellyfin {
            self.capture_jellyfin();
            if let Ok(mut cache) = self.hero_cache.lock() {
                cache.at = None;
            }
        }
        self.active_id = None;
        self.present("home");
        self.restore_fullscreen();
        let _ = self.home.focus();
        self.emit_state();
        self.hide_loading_soon(120);
    }

    // ------------------------------------------------------------------ services

    /// Contexte web (dossier de données) d'un service, partagé par NouTube et NouTube Music.
    fn take_context(&mut self, partition: &str) -> WebContext {
        self.contexts.remove(partition).unwrap_or_else(|| {
            let dir = self.paths.web_data(partition);
            raspytv_core::fsutil::ensure_private_dir(&dir);
            WebContext::new(Some(dir))
        })
    }

    fn create_view(&mut self, service: &Service) -> Option<ServiceView> {
        let jellyfin = service.id == "jellyfin";
        let restoring = jellyfin && self.shared.session.read().map(|s| s.is_some()).unwrap_or(false);
        let origin = url_origin(&service.url);
        let first_url = if restoring { format!("{origin}/System/Info/Public") } else { service.url.clone() };
        let id = service.id.clone();
        let init = media::service_init_script(&service.id, &self.tv_adapter);
        let noutube = (service.noutube && !self.noutube.is_empty()).then(|| media::noutube_init_script(&self.noutube));
        let ipc_proxy = self.proxy.clone();
        let load_proxy = self.proxy.clone();
        let (ipc_id, load_id) = (id.clone(), id.clone());
        let user_agent = service.noutube.then_some(CHROME_USER_AGENT);
        let partition = service.partition().to_string();
        let mut context = self.take_context(&partition);
        let mut builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(first_url)
            .with_background_color((7, 9, 16, 255))
            .with_autoplay(true)
            .with_devtools(cfg!(debug_assertions))
            .with_initialization_script_for_main_only(init, true)
            .with_ipc_handler(move |request| {
                let _ = ipc_proxy.send_event(UserEvent::ServiceIpc { id: ipc_id.clone(), body: request.into_body() });
            })
            .with_on_page_load_handler(move |event, url| {
                let finished = matches!(event, wry::PageLoadEvent::Finished);
                let _ = load_proxy.send_event(UserEvent::PageLoad { id: load_id.clone(), finished, url });
            })
            // Fenêtres surgissantes refusées ; seules les adresses http(s) sont suivies.
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_navigation_handler(|url| {
                url.starts_with("https://") || url.starts_with("http://") || url.starts_with("about:") || url.starts_with("blob:") || url.starts_with("data:")
            });
        if let Some(script) = noutube {
            builder = builder.with_initialization_script_for_main_only(script, true);
        }
        if let Some(agent) = user_agent {
            builder = builder.with_user_agent(agent);
        }
        let built = self.host.build(&id, builder, &self.window);
        self.contexts.insert(partition, context);
        match built {
            Ok(webview) => Some(ServiceView { service: service.clone(), webview, ready: false, restoring, pending: None }),
            Err(error) => {
                rlog!("view failure {id} {error}");
                self.toast(&format!("{} ne peut pas s’ouvrir.", service.name));
                None
            }
        }
    }

    fn evict_inactive(&mut self, keep: &str) {
        let limit = usize::from(self.config.max_resident_apps);
        while self.views.len() >= limit {
            // La musique d'arrière-plan n'est jamais fermée pour faire de la place.
            let candidate = self
                .views
                .keys()
                .filter(|id| id.as_str() != keep && Some(id.as_str()) != self.active_id.as_deref() && !self.is_background(id))
                .min_by_key(|id| self.last_used.get(*id).copied())
                .cloned();
            let Some(candidate) = candidate else { return };
            self.close_service(&candidate, true, "memory-limit");
        }
    }

    pub fn launch(&mut self, id: &str) -> Result<(), String> {
        let service = self.app_by_id(id).ok_or("Application inconnue")?;
        self.tv_return_home = None;
        self.tv_return_settings = None;
        if service.media && !service.background {
            self.pause_background_media(Some(id));
        }
        self.show_loading(&service.name);
        self.stop_native();
        self.restore_fullscreen();
        self.hide_keyboard();
        if self.active_id.as_deref().is_some_and(|a| a != id) {
            self.pause_active_media();
        }
        if !self.views.contains_key(id) {
            self.evict_inactive(id);
            let view = self.create_view(&service).ok_or("Ouverture impossible")?;
            self.views.insert(id.to_string(), view);
            rlog!("opened {id}");
        }
        self.eval(id, &media::mute_script(false));
        self.present(id);
        self.active_id = Some(id.to_string());
        self.last_used.insert(id.to_string(), Instant::now());
        self.config.touch_recent(id);
        self.save_config();
        if let Some(view) = self.views.get(id) {
            let _ = view.webview.focus();
            if view.ready {
                self.hide_loading_soon(100);
            } else {
                let id = id.to_string();
                self.after(Duration::from_secs(15), move |app| {
                    if app.active_id.as_deref() == Some(id.as_str()) {
                        app.hide_loading_soon(0);
                    }
                });
            }
        }
        Ok(())
    }

    pub fn close_service(&mut self, id: &str, silent: bool, reason: &str) {
        let Some(view) = self.views.remove(id) else { return };
        drop(view);
        self.host.remove(id);
        self.last_used.remove(id);
        if self.keyboard_target == Some(Target::View(id.to_string())) {
            self.hide_keyboard();
        }
        if self.active_id.as_deref() == Some(id) {
            self.show_home();
        }
        rlog!("closed {id} ({reason})");
        if !silent {
            self.emit_state();
        }
    }

    pub fn on_page_load(&mut self, id: &str, finished: bool, url: &str) {
        let Some(view) = self.views.get_mut(id) else { return };
        if !finished {
            return;
        }
        if cfg!(debug_assertions) {
            rlog!("page {id} {url}");
        }
        if view.restoring {
            view.restoring = false;
            let service_url = view.service.url.clone();
            self.restore_jellyfin(id.to_string(), service_url);
            return;
        }
        let first = !view.ready;
        view.ready = true;
        let pending = view.pending.take();
        if self.active_id.as_deref() == Some(id) && first {
            self.hide_loading_soon(100);
        }
        if id == "jellyfin" {
            let delay = if first { 2500 } else { 1800 };
            self.after(Duration::from_millis(delay), |app| app.capture_jellyfin());
            if let Some(item) = pending {
                self.run_jellyfin_play(&item);
            }
        }
        let _ = url;
    }

    pub fn on_service_ipc(&mut self, id: &str, body: &str) {
        let Ok(message) = serde_json::from_str::<Value>(body) else { return };
        match message.get("t").and_then(Value::as_str).unwrap_or("") {
            "key" => match message.get("key").and_then(Value::as_str) {
                Some("Escape") if id == "jellyfin" => {
                    self.eval_view(id, jellyfin::ESCAPE_SCRIPT, |app, closed| {
                        if closed.trim() != "true" {
                            app.show_home();
                        }
                    });
                }
                Some("Escape") | Some("Home") => self.show_home(),
                _ => {}
            },
            "osk" => {
                if message.get("open") == Some(&Value::Bool(true)) {
                    self.show_keyboard(Target::View(id.to_string()));
                } else {
                    self.hide_keyboard_soon(Target::View(id.to_string()));
                }
            }
            "noutube" => {
                let host: String = message.get("host").and_then(Value::as_str).unwrap_or("").chars().take(60).collect();
                let active = message.get("active") == Some(&Value::Bool(true));
                rlog!("noutube {} {host}", if active { "actif" } else { "INACTIF" });
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ Jellyfin

    /// Réinstalle la session dans le stockage local, puis remplace la page technique par Jellyfin Web.
    fn restore_jellyfin(&mut self, id: String, service_url: String) {
        let session = self.shared.session.read().ok().and_then(|s| s.clone());
        let origin = self.shared.jellyfin_origin.clone();
        let local = self.site.jellyfin_origin();
        self.spawn(
            move || session.and_then(|s| jellyfin::Client::new(&origin).validate(&s)).map(|valid| jellyfin::restore_script(&origin, &local, &valid)),
            move |app, script| {
                let target = Value::String(service_url);
                let script = script.unwrap_or_else(|| "true".into());
                // location.replace : la page technique ne reste pas dans l'historique.
                app.eval(&id, &format!("{script};location.replace({target})"));
            },
        );
    }

    /// Relit la session de Jellyfin Web et la conserve chiffrée si elle est valide.
    fn capture_jellyfin(&mut self) {
        if !self.views.contains_key("jellyfin") {
            return;
        }
        self.eval_view("jellyfin", jellyfin::CAPTURE_SCRIPT, |app, raw| {
            let raw: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            let raw: Value = raw.as_str().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(raw);
            let candidates = jellyfin::candidates_from_storage(raw.get("cinematic").and_then(Value::as_str), raw.get("jellyfin").and_then(Value::as_str));
            if candidates.is_empty() {
                return;
            }
            let origin = app.shared.jellyfin_origin.clone();
            app.spawn(
                move || {
                    let client = jellyfin::Client::new(&origin);
                    candidates.iter().find_map(|c| client.validate(c))
                },
                |app, valid| {
                    if let Some(valid) = valid {
                        app.store_session(valid);
                    }
                },
            );
        });
    }

    fn store_session(&mut self, session: Session) {
        let same = self.shared.session.read().ok().is_some_and(|s| s.as_ref() == Some(&session));
        if same {
            return;
        }
        if vault::save(&session, &self.paths.vault(), &self.paths.vault_key()) {
            rlog!("jellyfin session saved (encrypted)");
        }
        if let Ok(mut current) = self.shared.session.write() {
            *current = Some(session);
        }
    }

    pub fn jellyfin_hero(&mut self, id: Option<u64>, force: bool) {
        let cache = self.hero_cache.clone();
        let session = self.shared.session.read().ok().and_then(|s| s.clone());
        let origin = self.shared.jellyfin_origin.clone();
        let daily = self.paths.daily_hero();
        let timezone = self.site.timezone.clone();
        self.spawn(
            move || {
                let today = hero::local_day_key(&timezone);
                if let Ok(cache) = cache.lock() {
                    if let (Some(data), Some(at)) = (&cache.data, cache.at) {
                        let ttl = if !data.items.is_empty() {
                            900
                        } else if data.needs_login {
                            600
                        } else {
                            60
                        };
                        if !force && cache.day == today && at.elapsed() < Duration::from_secs(ttl) {
                            return (data.clone(), None);
                        }
                    }
                }
                let client = jellyfin::Client::new(&origin);
                let valid = session.as_ref().and_then(|s| client.validate(s));
                let mut data = match &valid {
                    None => HomeData {
                        needs_login: true,
                        error: Some(if session.is_some() { "authentication" } else { "credentials" }.into()),
                        ..HomeData::default()
                    },
                    Some(valid) => client.home(valid).unwrap_or_else(|error| HomeData { error: Some(error), ..HomeData::default() }),
                };
                if !data.items.is_empty() {
                    data.items = hero::daily_selection(&daily, &data.items, &timezone);
                }
                if let Ok(mut cache) = cache.lock() {
                    *cache = HeroCache { data: Some(data.clone()), at: Some(Instant::now()), day: today };
                }
                (data, valid)
            },
            move |app, (data, valid)| {
                if let Some(error) = &data.error {
                    rlog!("jellyfin hero unavailable: {error}");
                }
                if let Some(valid) = valid {
                    app.store_session(valid);
                }
                app.reply(id, Ok(serde_json::to_value(&data).unwrap_or(Value::Null)));
            },
        );
    }

    pub fn play_jellyfin(&mut self, item: &str) -> Result<(), String> {
        let server = self.shared.session.read().ok().and_then(|s| s.as_ref().map(|s| s.server_id.clone())).unwrap_or_default();
        jellyfin::play_script(item, &server).ok_or("Identifiant Jellyfin invalide")?;
        self.launch("jellyfin")?;
        let ready = self.views.get("jellyfin").is_some_and(|v| v.ready);
        if ready {
            self.run_jellyfin_play(item);
        } else if let Some(view) = self.views.get_mut("jellyfin") {
            view.pending = Some(item.to_string());
        }
        Ok(())
    }

    fn run_jellyfin_play(&self, item: &str) {
        let server = self.shared.session.read().ok().and_then(|s| s.as_ref().map(|s| s.server_id.clone())).unwrap_or_default();
        if let Some(script) = jellyfin::play_script(item, &server) {
            self.eval("jellyfin", &script);
        }
    }

    // ------------------------------------------------------------------ YouTube

    /// Session YouTube déposée dans `import/youtube-cookies.txt` : importée puis effacée.
    pub fn import_youtube_session(&mut self) {
        let file = self.paths.youtube_import();
        let Ok(text) = std::fs::read_to_string(&file) else { return };
        let _ = std::fs::write(&file, b"");
        let _ = std::fs::remove_file(&file);
        let now = jiff_now();
        let cookies = raspytv_core::cookies::parse_cookies_txt(&text, now);
        let mut imported = 0;
        // Une vue temporaire porte les cookies dans le contexte web partagé de NouTube.
        let service = self.app_by_id("youtube");
        let views: Vec<String> = self.views.iter().filter(|(_, v)| v.service.noutube).map(|(id, _)| id.clone()).collect();
        let target = views.first().cloned();
        let created = target.is_none();
        let target = match target {
            Some(id) => id,
            None => match service.and_then(|s| self.create_view(&s).map(|v| (s.id.clone(), v))) {
                Some((id, view)) => {
                    self.views.insert(id.clone(), view);
                    id
                }
                None => return,
            },
        };
        if let Some(view) = self.views.get(&target) {
            for cookie in cookies.iter().map(|c| c.normalized()) {
                let mut builder = wry::cookie::Cookie::build((cookie.name.clone(), cookie.value.clone()))
                    .domain(cookie.domain.clone())
                    .path(cookie.path.clone())
                    .secure(cookie.secure)
                    .http_only(cookie.http_only)
                    .same_site(if cookie.secure { wry::cookie::SameSite::None } else { wry::cookie::SameSite::Lax });
                if let Some(expires) = cookie.expires.and_then(|e| wry::cookie::time::OffsetDateTime::from_unix_timestamp(e).ok()) {
                    builder = builder.expires(expires);
                }
                match view.webview.set_cookie(&builder.build()) {
                    Ok(()) => imported += 1,
                    Err(error) => rlog!("youtube cookie refused {} {}: {error}", cookie.domain, cookie.name),
                }
            }
        }
        rlog!("youtube session imported: {imported}/{}", cookies.len());
        if created {
            self.close_service(&target, true, "cookie-import");
        } else {
            for id in views {
                if let Some(view) = self.views.get(&id) {
                    let _ = view.webview.reload();
                }
            }
        }
        self.toast(if imported > 0 { "Session YouTube importée : NouTube est connecté." } else { "Aucun cookie YouTube valable dans le fichier importé." });
    }

    // ------------------------------------------------------------------ Moonlight

    pub fn moonlight_games(&self, id: Option<u64>) {
        let cache = self.moonlight_cache.clone();
        let host = self.site.moonlight_host.clone();
        self.spawn(
            move || {
                if let Ok(cache) = cache.lock() {
                    if let (Some(data), Some(at)) = (&cache.data, cache.at) {
                        if at.elapsed() < Duration::from_secs(600) {
                            return data.clone();
                        }
                    }
                }
                let games = moonlight::list_games(&host);
                if !games.games.is_empty() {
                    rlog!("moonlight games loaded: {}", games.games.len());
                }
                if let Ok(mut cache) = cache.lock() {
                    *cache = MoonlightCache { data: Some(games.clone()), at: Some(Instant::now()) };
                }
                games
            },
            move |app, games| app.reply(id, Ok(serde_json::to_value(games).unwrap_or(Value::Null))),
        );
    }

    pub fn launch_native(&mut self, game: String, id: Option<u64>) {
        self.tv_return_home = None;
        self.tv_return_settings = None;
        self.pause_background_media(None);
        if self.native.is_some() {
            self.toast("Moonlight est déjà ouvert.");
            self.reply(id, Ok(json!(false)));
            return;
        }
        let host = self.site.moonlight_host.clone();
        let cache = self.moonlight_cache.clone();
        self.spawn(
            move || {
                if !game.is_empty() {
                    let known = cache.lock().ok().and_then(|c| c.data.clone()).unwrap_or_else(|| moonlight::list_games(&host));
                    if !known.games.iter().any(|g| g.name == game) {
                        return Err("Jeu Moonlight introuvable".to_string());
                    }
                }
                moonlight::prepare_kiosk().map_err(|error| {
                    rlog!("moonlight kiosk setup failure {error}");
                    "Impossible de préparer Moonlight. Relance RaspyTV.".to_string()
                })?;
                Ok(moonlight::stream_args(&host, &game))
            },
            move |app, args| match args {
                Ok(args) => {
                    let started = app.start_native(args);
                    app.reply(id, Ok(json!(started)));
                }
                Err(error) => {
                    app.toast(&error);
                    app.reply(id, Err(error));
                }
            },
        );
    }

    fn start_native(&mut self, args: Vec<String>) -> bool {
        if self.native.is_some() {
            return false;
        }
        self.pause_active_media();
        // La surface reste affichée (maximisée) : la masquer pendant le flux était instable.
        self.window.set_fullscreen(None);
        self.window.set_maximized(true);
        let mut command = std::process::Command::new(moonlight::BIN);
        command.args(&args).env("SDL_MOUSE_AUTO_CAPTURE", "0").env("SDL_VIDEO_X11_NET_WM_BYPASS_COMPOSITOR", "0");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        match command.spawn() {
            Ok(mut child) => {
                let pid = child.id();
                self.native = Some(pid);
                self.emit_state();
                #[cfg(target_os = "linux")]
                if let Some(layer) = &self.layer {
                    layer.raise();
                }
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let _ = child.wait();
                    let _ = proxy.send_event(UserEvent::Run(Box::new(move |app| {
                        if app.native == Some(pid) {
                            app.native = None;
                            app.restore_from_native();
                        }
                    })));
                });
                true
            }
            Err(error) => {
                rlog!("moonlight launch failure {error}");
                self.restore_from_native();
                self.toast("Impossible de lancer Moonlight.");
                false
            }
        }
    }

    fn restore_from_native(&mut self) {
        self.set_system_keyboard(Some(false));
        self.restore_fullscreen();
        self.active_id = None;
        self.present("home");
        let _ = self.home.focus();
        self.emit_state();
    }

    fn stop_native(&mut self) {
        if let Some(pid) = self.native.take() {
            let _ = sys::run("kill", &["-TERM", &format!("-{pid}")], Duration::from_secs(2));
            let _ = sys::run("kill", &["-TERM", &pid.to_string()], Duration::from_secs(2));
            self.set_system_keyboard(Some(false));
            self.emit_state();
        }
        // Un client Qt orphelin ne doit pas continuer à jouer derrière l'accueil.
        std::thread::spawn(|| {
            let _ = sys::run("pkill", &["-TERM", "-x", "moonlight-qt"], Duration::from_secs(2));
        });
    }

    /// Clavier système (squeekboard) pendant Moonlight. `None` : bascule.
    fn set_system_keyboard(&mut self, visible: Option<bool>) {
        let visible = visible.unwrap_or(!self.system_keyboard);
        if !visible && !self.system_keyboard {
            return;
        }
        self.system_keyboard = visible;
        std::thread::spawn(move || {
            if sys::run("pgrep", &["-x", "squeekboard"], Duration::from_secs(2)).map(|o| !o.success()).unwrap_or(true) && visible {
                let _ = std::process::Command::new("/usr/bin/squeekboard")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                std::thread::sleep(Duration::from_millis(650));
            }
            let value = if visible { "true" } else { "false" };
            if let Err(error) = sys::checked(
                "busctl",
                &["--user", "call", "sm.puri.OSK0", "/sm/puri/OSK0", "sm.puri.OSK0", "SetVisible", "b", value],
                Duration::from_millis(2500),
            ) {
                rlog!("squeekboard toggle failure {error}");
            }
        });
    }

    // ------------------------------------------------------------------ clavier virtuel

    fn show_keyboard(&mut self, target: Target) {
        self.keyboard_generation += 1;
        self.keyboard_target = Some(target);
        #[cfg(target_os = "linux")]
        if let Some(layer) = &self.layer {
            layer.show_keyboard();
        }
    }

    pub fn hide_keyboard(&mut self) {
        self.keyboard_generation += 1;
        self.keyboard_target = None;
        #[cfg(target_os = "linux")]
        if let Some(layer) = &self.layer {
            layer.hide_keyboard();
        }
    }

    fn hide_keyboard_soon(&mut self, target: Target) {
        let generation = self.keyboard_generation;
        self.after(Duration::from_millis(240), move |app| {
            if app.keyboard_generation == generation && app.keyboard_target.as_ref() == Some(&target) {
                app.hide_keyboard();
            }
        });
    }

    fn keyboard_webview(&self) -> Option<&WebView> {
        match self.keyboard_target.as_ref()? {
            Target::Home => Some(&self.home),
            Target::View(id) => self.views.get(id).map(|v| &v.webview),
        }
    }

    pub fn on_osk(&mut self, key: OskKey) {
        if matches!(key, OskKey::Close) {
            return self.hide_keyboard();
        }
        let script = match &key {
            OskKey::Text(text) if text.chars().count() <= 4 => Some(media::insert_text_script(text)),
            OskKey::Key(name) => media::special_key_script(name),
            _ => None,
        };
        // Garde le clavier ouvert : la saisie ne doit pas le refermer.
        self.keyboard_generation += 1;
        match (self.keyboard_webview(), script) {
            (Some(view), Some(script)) => {
                let _ = view.focus();
                let _ = view.evaluate_script(&script);
            }
            (None, _) => self.hide_keyboard(),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ commandes TV

    fn current_place(&self) -> Place {
        match &self.active_id {
            Some(id) if self.views.contains_key(id) => Place::App(id.clone()),
            _ if self.shell_page == "settings" => Place::Settings,
            _ => Place::Home,
        }
    }

    fn show_shell(&mut self, page: &str) {
        self.hide_keyboard();
        if self.active_id.is_some() || self.native.is_some() {
            self.show_home();
        }
        self.shell_page = page.to_string();
        self.send_home(json!({"t": "navigate", "page": page}));
        let _ = self.home.focus();
    }

    fn restore_place(&mut self, place: Option<Place>) {
        match place {
            Some(Place::App(id)) if self.app_by_id(&id).is_some() => {
                let _ = self.launch(&id);
            }
            Some(Place::Settings) => self.show_shell("settings"),
            _ => self.show_shell("home"),
        }
    }

    fn close_nav_peek(&mut self) {
        if self.nav_peek_open {
            self.nav_peek_open = false;
            self.toggle_nav_peek();
        }
    }

    fn toggle_nav_peek(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(layer) = self.layer.as_ref().filter(|_| !self.art_mode) {
            layer.toggle_peek();
        }
    }

    pub fn tv_home_press(&mut self) {
        self.close_nav_peek();
        if self.active_id.is_some() || self.shell_page == "settings" {
            let from = self.current_place();
            self.show_shell("home");
            self.tv_return_home = Some(from);
            return;
        }
        if let Some(target) = self.tv_return_home.take() {
            self.restore_place(Some(target));
        }
    }

    pub fn tv_settings_press(&mut self) {
        self.close_nav_peek();
        if self.active_id.is_none() && self.shell_page == "settings" {
            let target = self.tv_return_settings.take();
            return self.restore_place(target.or(Some(Place::Home)));
        }
        let from = self.current_place();
        self.show_shell("settings");
        self.tv_return_settings = Some(from);
    }

    fn tv_back(&mut self) {
        if self.nav_peek_open {
            return self.close_nav_peek();
        }
        if let Some(target) = self.keyboard_target.clone() {
            self.hide_keyboard();
            if let Target::View(id) = target {
                self.eval(&id, media::KEYBOARD_CLOSED);
            }
            return;
        }
        if self.native.is_some() {
            return;
        }
        let Some(id) = self.active_id.clone().filter(|id| self.views.contains_key(id)) else {
            return self.send_home(json!({"t": "tvBack"}));
        };
        let key = id.clone();
        self.eval_view(&key, media::EXIT_FULLSCREEN, move |app, left| {
            if left.trim() == "true" {
                return;
            }
            let can_go_back = app.views.get(&id).and_then(|v| v.webview.can_go_back().ok()).unwrap_or(false);
            if can_go_back {
                if let Some(view) = app.views.get(&id) {
                    let _ = view.webview.go_back();
                }
                return;
            }
            app.tv_return_home = None;
            app.show_shell("home");
        });
    }

    fn tv_play_pause(&mut self) {
        let mut candidates: Vec<String> = self.active_id.iter().cloned().collect();
        // Depuis l'accueil, □ pilote la musique qui joue en arrière-plan.
        candidates.extend(self.views.keys().filter(|id| Some(*id) != self.active_id.as_ref() && self.is_background(id)).cloned());
        self.try_play_pause(candidates);
    }

    fn try_play_pause(&mut self, mut candidates: Vec<String>) {
        if candidates.is_empty() {
            return playerctl("play-pause");
        }
        let id = candidates.remove(0);
        if !self.views.contains_key(&id) {
            return self.try_play_pause(candidates);
        }
        let key = id.clone();
        self.eval_view(&key, media::PLAY_PAUSE, move |app, handled| {
            if handled.trim() == "true" {
                app.eval(&id, &media::mute_script(false));
            } else {
                app.try_play_pause(candidates);
            }
        });
    }

    fn focus_current(&self) {
        if self.native.is_some() {
            return;
        }
        match self.active_id.as_ref().and_then(|id| self.views.get(id)) {
            Some(view) => {
                let _ = view.webview.focus();
            }
            None => {
                let _ = self.home.focus();
            }
        }
    }

    pub fn on_gamepad(&mut self, key: &str) {
        if self.native.is_some() || self.panel_open || self.nav_peek_open {
            return;
        }
        match self.active_id.clone() {
            None => {
                let key = Value::String(key.to_string());
                let _ = self.home.evaluate_script(&format!("window.__raspyHome&&window.__raspyHome.tv_key({key})"));
            }
            Some(id) => {
                if key == "Escape" {
                    return self.tv_back();
                }
                if let Some(script) = media::key_script(key) {
                    self.eval(&id, &script);
                }
            }
        }
    }

    pub fn handle_command(&mut self, command: Command) {
        match command {
            Command::TvState => self.write_panel_state(None),
            Command::TvPanel => {
                if self.panel_open {
                    self.toggle_panel();
                } else {
                    // L'état est préparé avant l'ouverture : la lecture en cours s'affiche d'emblée.
                    let generation = self.panel_generation;
                    self.write_panel_state(Some(Box::new(move |app: &mut App| {
                        if app.panel_generation == generation && !app.panel_open {
                            app.toggle_panel();
                        }
                    })));
                }
            }
            Command::TvPanelState(open) => {
                self.panel_open = open;
                self.panel_generation += 1;
                if open {
                    self.panel_tick(self.panel_generation);
                } else {
                    self.focus_current();
                }
            }
            Command::Media { op, value } => self.media_command(&op, value),
            Command::VolumeSet(value) => self.spawn(move || audio::set_volume(value), |app, result| app.apply_volume(result)),
            Command::Volume(delta) => self.spawn(move || audio::change_volume(delta), |app, result| app.apply_volume(result)),
            Command::AudioOutput(id) => self.spawn(
                move || audio::set_output(&id),
                |app, result| match result {
                    Ok(_) => app.refresh_audio(),
                    Err(error) => {
                        rlog!("audio output failure {error}");
                        app.toast(&error);
                    }
                },
            ),
            // Alimentation demandée depuis le panneau, après confirmation.
            Command::SystemPower(op) => {
                rlog!("system power {op} requested from panel");
                self.spawn(
                    move || sys::power(&op),
                    |app, result| {
                        if let Err(error) = result {
                            app.toast(&error);
                        }
                    },
                );
            }
            Command::NavPeek(open) => {
                self.nav_peek_open = open;
                if !open {
                    self.focus_current();
                }
            }
            Command::TvHome => self.tv_home_press(),
            Command::TvSettings => self.tv_settings_press(),
            Command::TvBack => self.tv_back(),
            Command::TvPlayPause => self.tv_play_pause(),
            Command::TvContext => {
                if let Some(id) = self.active_id.clone() {
                    self.eval(&id, media::CONTEXT);
                }
            }
            Command::TvNavPeek => self.toggle_nav_peek(),
            Command::NoutubeImportSession => self.import_youtube_session(),
            Command::Home => self.show_home(),
            Command::Settings => {
                self.show_home();
                self.shell_page = "settings".into();
                self.send_home(json!({"t": "navigate", "page": "settings"}));
            }
            Command::Moonlight => {
                if self.native.is_none() {
                    self.launch_native(String::new(), None);
                }
            }
            Command::MoonlightGame(name) => {
                if self.native.is_none() {
                    self.launch_native(name, None);
                }
            }
            Command::Keyboard => {
                if self.native.is_some() {
                    self.set_system_keyboard(None);
                }
            }
            Command::Launch(id) => {
                if self.app_by_id(&id).is_some() {
                    let _ = self.launch(&id);
                }
            }
        }
    }

    fn toggle_panel(&self) {
        #[cfg(target_os = "linux")]
        if let Some(layer) = &self.layer {
            layer.toggle_panel();
        }
    }

    fn apply_volume(&mut self, result: Result<audio::AudioState, String>) {
        match result {
            Ok(state) => {
                self.system_volume = Some(state.volume);
                self.system_muted = state.muted;
                self.write_layer_state();
                if self.panel_open {
                    self.write_panel_state(None);
                }
            }
            Err(error) => rlog!("volume change failure {error}"),
        }
    }

    // ------------------------------------------------------------------ son

    /// Relu à chaque événement `pactl subscribe` (nouveau casque, volume, sortie).
    pub fn refresh_audio(&mut self) {
        self.spawn(audio::state, |app, state| {
            if state.unavailable {
                return;
            }
            let outputs: Vec<(String, bool)> = state.sinks.iter().map(|s| (s.id.clone(), s.active)).collect();
            let outputs = format!("{outputs:?}");
            let key = format!("{outputs}|{}|{}", state.volume, state.muted);
            if key == app.last_audio_key {
                return;
            }
            let outputs_changed = !app.last_audio_key.starts_with(&outputs);
            app.last_audio_key = key;
            if outputs_changed {
                if let Ok(mut cache) = app.bluetooth.lock() {
                    cache.at = None;
                }
                app.send_home(json!({"t": "audioChanged"}));
            }
            app.system_volume = Some(state.volume);
            app.system_muted = state.muted;
            app.write_layer_state();
            if app.panel_open {
                app.write_panel_state(None);
            }
        });
    }

    pub fn audio_outputs(&self) -> impl FnOnce() -> audio::AudioState + Send + 'static {
        let cache = self.bluetooth.clone();
        move || {
            let devices = {
                let fresh = cache.lock().ok().and_then(|c| c.at.filter(|at| at.elapsed() < Duration::from_secs(10)).map(|_| c.list.clone()));
                match fresh {
                    Some(list) => list,
                    None => {
                        let list = audio::bluetooth_devices();
                        if let Ok(mut c) = cache.lock() {
                            *c = BluetoothCache { at: Some(Instant::now()), list: list.clone() };
                        }
                        list
                    }
                }
            };
            audio::outputs(&devices)
        }
    }

    // ------------------------------------------------------------------ panneau PS

    fn panel_tick(&mut self, generation: u64) {
        if !self.panel_open || self.panel_generation != generation {
            return;
        }
        self.write_panel_state(None);
        self.after(Duration::from_secs(1), move |app| app.panel_tick(generation));
    }

    /// Lecture en cours (NouTube Music d'abord) et son, pour le panneau.
    fn write_panel_state(&mut self, then: Option<Job>) {
        let mut order: Vec<String> = self.views.keys().filter(|id| self.is_background(id)).cloned().collect();
        if let Some(active) = self.active_id.clone().filter(|id| self.views.contains_key(id) && !order.contains(id)) {
            order.push(active);
        }
        let generation = self.probe.generation + 1;
        self.probe = Probe { generation, expected: order.len(), results: Vec::new(), done: false, then };
        for id in order.clone() {
            let order = order.clone();
            let key = id.clone();
            self.eval_view(&key, media::MEDIA_PROBE, move |app, raw| {
                if app.probe.generation != generation {
                    return;
                }
                let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                let value: Value = value.as_str().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(value);
                app.probe.results.push((id, value));
                if app.probe.results.len() >= app.probe.expected {
                    app.finish_probe(generation, order.clone());
                }
            });
        }
        let order_for_timeout: Vec<String> = self.views.keys().cloned().collect();
        if self.probe.expected == 0 {
            self.finish_probe(generation, Vec::new());
        } else {
            self.after(Duration::from_millis(1500), move |app| app.finish_probe(generation, order_for_timeout));
        }
    }

    fn finish_probe(&mut self, generation: u64, order: Vec<String>) {
        if self.probe.generation != generation || self.probe.done {
            return;
        }
        self.probe.done = true;
        let then = self.probe.then.take();
        let results = std::mem::take(&mut self.probe.results);
        let ranked: Vec<&(String, Value)> = order.iter().filter_map(|id| results.iter().find(|(r, _)| r == id)).collect();
        let usable = |v: &Value| v.is_object() && (v["title"].as_str().is_some_and(|t| !t.is_empty()) || v["duration"].as_f64().unwrap_or(0.0) > 0.0);
        let source = ranked
            .iter()
            .find(|(_, v)| usable(v) && v["playing"] == Value::Bool(true))
            .or_else(|| ranked.iter().find(|(_, v)| usable(v)))
            .map(|(id, v)| (id.clone(), v.clone()));
        self.panel_source = source.as_ref().map(|(id, _)| id.clone());
        let app_name = source.as_ref().and_then(|(id, _)| self.app_by_id(id)).map(|a| a.name).unwrap_or_default();
        let outputs = self.audio_outputs();
        let art = self.panel_art.clone();
        let data = self.paths.data.clone();
        self.spawn(
            move || {
                let audio = outputs();
                let media = source.map(|(_, v)| MediaInfo {
                    app: app_name,
                    title: v["title"].as_str().unwrap_or("").into(),
                    artist: v["artist"].as_str().unwrap_or("").into(),
                    artwork: cache_artwork(&art, &data, v["artwork"].as_str().unwrap_or("")),
                    position: v["position"].as_f64().unwrap_or(0.0).max(0.0),
                    duration: v["duration"].as_f64().unwrap_or(0.0).max(0.0),
                    playing: v["playing"].as_bool().unwrap_or(false),
                });
                PanelState {
                    media,
                    audio: PanelAudio {
                        volume: audio.volume,
                        muted: audio.muted,
                        outputs: audio
                            .sinks
                            .iter()
                            .map(|s| PanelOutput {
                                id: s.id.clone(),
                                name: s.name.clone(),
                                detail: s.detail.clone(),
                                kind: s.kind.clone(),
                                headset: s.headset,
                                active: s.active,
                            })
                            .collect(),
                        bluetooth: audio.bluetooth.iter().map(|d| (d.id.clone(), d.name.clone())).collect(),
                    },
                }
            },
            move |app, state| {
                let _ = raspytv_core::fsutil::atomic_write(&app.paths.panel_state(), serde_json::to_string(&state).unwrap_or_default().as_bytes());
                #[cfg(target_os = "linux")]
                if let Some(layer) = &app.layer {
                    layer.apply_panel(&state);
                }
                if let Some(then) = then {
                    then(app);
                }
            },
        );
    }

    fn media_command(&mut self, op: &str, value: f64) {
        let Some(script) = media::media_command(op, value) else { return };
        let target =
            self.panel_source.clone().filter(|id| self.views.contains_key(id)).or_else(|| self.active_id.clone().filter(|id| self.views.contains_key(id)));
        let Some(id) = target else {
            match op {
                "toggle" => playerctl("play-pause"),
                "next" => playerctl("next"),
                "previous" => playerctl("previous"),
                _ => {}
            }
            return;
        };
        if op == "toggle" {
            self.eval(&id, &media::mute_script(false));
        }
        let op = op.to_string();
        self.eval_view(&id, &script, move |app, outcome| {
            if op == "next" || op == "previous" {
                rlog!("media {op}: {outcome}");
            }
            if app.panel_open {
                app.write_panel_state(None);
            }
        });
    }

    // ------------------------------------------------------------------ divers

    pub fn set_art_mode(&mut self, enabled: bool) {
        self.art_mode = enabled;
        #[cfg(target_os = "linux")]
        if let Some(layer) = &self.layer {
            layer.collapse();
        }
        self.emit_state();
    }

    pub fn set_shell_page(&mut self, page: &str) {
        if page == "home" || page == "settings" {
            self.shell_page = page.to_string();
        }
    }

    pub fn keyboard_focus(&mut self, editable: bool) {
        if editable {
            self.show_keyboard(Target::Home);
        } else {
            self.hide_keyboard_soon(Target::Home);
        }
    }

    pub fn resident(&self) -> (usize, u8) {
        (self.views.len(), self.config.max_resident_apps)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    pub fn site(&self) -> &Site {
        &self.site
    }

    pub fn update_config(&mut self, next: Config) {
        self.config = next;
        while self.views.len() > usize::from(self.config.max_resident_apps) {
            let before = self.views.len();
            let keep = self.active_id.clone().unwrap_or_default();
            self.evict_inactive(&keep);
            if self.views.len() == before {
                break;
            }
        }
        self.save_config();
    }

    pub fn defaults(&self) -> &[Service] {
        &self.defaults
    }

    pub fn shutdown(&mut self) {
        self.quitting = true;
        self.stop_native();
        let _ = std::fs::remove_file(self.paths.panel_flag(&crate::socket_prefix()));
    }
}

fn url_origin(url: &str) -> String {
    match url.find("://") {
        Some(scheme) => {
            let rest = &url[scheme + 3..];
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            url[..scheme + 3 + end].to_string()
        }
        None => url.to_string(),
    }
}

fn jiff_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn playerctl(command: &'static str) {
    std::thread::spawn(move || {
        if let Ok(output) = sys::run("playerctl", &[command], Duration::from_secs(3)) {
            if output.status != Some(0) && output.status != Some(1) {
                rlog!("playerctl {command} failure {}", output.error());
            }
        }
    });
}

/// Jaquette téléchargée pour le panneau GTK (qui ne lit que des fichiers locaux).
fn cache_artwork(cache: &Arc<Mutex<(String, PathBuf)>>, data: &std::path::Path, url: &str) -> String {
    if !url.starts_with("https://") {
        return String::new();
    }
    if let Ok(current) = cache.lock() {
        if current.0 == url && current.1.is_file() {
            return current.1.display().to_string();
        }
    }
    let agent = raspytv_core::http::agent(Duration::from_secs(5));
    let Some((mime, bytes)) = raspytv_core::http::get_bytes(&agent, url, &[], 3 * 1024 * 1024) else { return String::new() };
    if !mime.starts_with("image/") {
        return String::new();
    }
    let hash: u64 = url.bytes().fold(1469598103934665603u64, |h, b| (h ^ u64::from(b)).wrapping_mul(1099511628211));
    let file = data.join(format!("panel-art-{:012x}", hash & 0xffff_ffff_ffff));
    if raspytv_core::fsutil::atomic_write(&file, &bytes).is_err() {
        return String::new();
    }
    if let Ok(mut current) = cache.lock() {
        if current.1 != file && current.1.is_file() {
            let _ = std::fs::remove_file(&current.1);
        }
        *current = (url.to_string(), file.clone());
    }
    file.display().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn origins() {
        assert_eq!(super::url_origin("http://jf.example:8096/web/#/home"), "http://jf.example:8096");
        assert_eq!(super::url_origin("https://www.youtube.com"), "https://www.youtube.com");
    }
}
