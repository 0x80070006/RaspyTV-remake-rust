//! Surfaces GTK layer-shell affichées au-dessus de tout, jeu Moonlight compris
//! (portage de `layer-nav.py`, dans le même processus que l'interface) :
//!
//! - la barre latérale, qui se déploie au survol du bord gauche ou avec Share ;
//! - l'indicateur de volume, en bas à droite ;
//! - le panneau de la manette (PS maintenu) : lecture en cours, son, sortie audio, manette ;
//! - le clavier virtuel AZERTY.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gdk::keys::constants as keys;
use glib::Propagation;
use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer as ShellLayer, LayerShell};
use raspytv_core::protocol::{Command, LayerState, PanelState};
use tao::event_loop::EventLoopProxy;

use crate::app::{OskKey, UserEvent};

const PEEK_MS: u64 = 4000;
/// Survol à la souris : repli après ce délai sans mouvement, même pointeur posé sur le bord.
const HOVER_IDLE_MS: u64 = 6000;
const PANEL_IDLE: Duration = Duration::from_secs(60);
const VOLUME_SEGMENTS: usize = 20;
const PANEL_VOLUME_SEGMENTS: usize = 24;
const ART_SIZE: i32 = 168;

const CSS: &str = include_str!("layer.css");

// ------------------------------------------------------------------ outils

fn styled<W: IsA<gtk::Widget>>(widget: W, classes: &[&str]) -> W {
    let context = widget.style_context();
    for class in classes {
        context.add_class(class);
    }
    widget
}

fn icon(name: &str, size: i32) -> gtk::Image {
    let image = gtk::Image::from_icon_name(Some(name), gtk::IconSize::LargeToolbar);
    image.set_pixel_size(size);
    image
}

fn file_icon(path: &str, fallback: &str, size: i32) -> gtk::Image {
    if !path.is_empty() && Path::new(path).is_file() {
        if let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true) {
            return gtk::Image::from_pixbuf(Some(&pixbuf));
        }
    }
    icon(fallback, size)
}

fn label(text: &str, classes: &[&str], xalign: f32) -> gtk::Label {
    let widget = gtk::Label::new(Some(text));
    widget.set_xalign(xalign);
    styled(widget, classes)
}

fn clear(container: &gtk::Box) {
    for child in container.children() {
        container.remove(&child);
    }
}

fn focused_index(container: &gtk::Box) -> Option<usize> {
    container.children().iter().position(|child| child.has_focus() || child.downcast_ref::<gtk::Container>().and_then(|c| c.focus_child()).is_some())
}

fn focus_child(container: &gtk::Box, index: usize) -> bool {
    let children = container.children();
    let count = children.len();
    for offset in 0..count {
        let child = &children[(index + offset) % count];
        if child.can_focus() && child.is_visible() {
            child.grab_focus();
            return true;
        }
    }
    false
}

fn layer_window(keyboard: bool) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_accept_focus(keyboard);
    window.set_focus_on_map(false);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.set_app_paintable(true);
    window.init_layer_shell();
    window.set_layer(ShellLayer::Overlay);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
    window
}

fn segment_bar(count: usize) -> (gtk::Box, Vec<gtk::Box>) {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    bar.set_valign(gtk::Align::Center);
    let segments: Vec<gtk::Box> = (0..count).map(|_| styled(gtk::Box::new(gtk::Orientation::Horizontal, 0), &["seg"])).collect();
    for segment in &segments {
        bar.pack_start(segment, false, false, 0);
    }
    (bar, segments)
}

fn paint_segments(segments: &[gtk::Box], value: f64) {
    let lit = (value.clamp(0.0, 100.0) / 100.0 * segments.len() as f64).round() as usize;
    for (index, segment) in segments.iter().enumerate() {
        let context = segment.style_context();
        if index < lit {
            context.add_class("on");
        } else {
            context.remove_class("on");
        }
    }
}

fn volume_icon_name(value: f64, muted: bool) -> &'static str {
    if muted || value <= 0.0 {
        "audio-volume-muted-symbolic"
    } else if value < 34.0 {
        "audio-volume-low-symbolic"
    } else if value < 67.0 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

fn set_icon(image: &gtk::Image, name: &str, size: i32) {
    image.set_from_icon_name(Some(name), gtk::IconSize::LargeToolbar);
    image.set_pixel_size(size);
}

thread_local! {
    static FADES: RefCell<std::collections::HashMap<usize, gtk::TickCallbackId>> = RefCell::new(std::collections::HashMap::new());
}

/// Fondu d'opacité ; une seule animation par widget. `hide = false` garde la fenêtre affichée.
fn fade(widget: &impl IsA<gtk::Widget>, window: &gtk::Window, target: f64, on_done: Option<Box<dyn Fn()>>, hide: bool) {
    let widget = widget.as_ref().clone();
    let key = widget.as_ptr() as usize;
    if let Some(previous) = FADES.with(|f| f.borrow_mut().remove(&key)) {
        previous.remove();
    }
    if target > 0.0 && !window.is_visible() {
        widget.set_opacity(0.0);
        window.show_all();
    }
    let start = widget.opacity();
    let started = glib::monotonic_time();
    let duration = if target > start { 140_000.0 } else { 200_000.0 };
    let window = window.clone();
    let id = widget.add_tick_callback(move |widget, _| {
        let progress = ((glib::monotonic_time() - started) as f64 / duration).min(1.0);
        widget.set_opacity(start + (target - start) * (1.0 - (1.0 - progress).powi(2)));
        if progress < 1.0 {
            return glib::ControlFlow::Continue;
        }
        FADES.with(|f| f.borrow_mut().remove(&(widget.as_ptr() as usize)));
        if target == 0.0 && hide {
            window.hide();
        }
        if let Some(done) = &on_done {
            done();
        }
        glib::ControlFlow::Break
    });
    FADES.with(|f| f.borrow_mut().insert(key, id));
}

fn timeout(ms: u64, f: impl FnOnce() + 'static) -> glib::SourceId {
    let f = Cell::new(Some(f));
    glib::timeout_add_local(Duration::from_millis(ms), move || {
        if let Some(f) = f.take() {
            f();
        }
        glib::ControlFlow::Break
    })
}

fn cancel(slot: &RefCell<Option<glib::SourceId>>) {
    if let Some(id) = slot.borrow_mut().take() {
        id.remove();
    }
}

/// Entrée / Espace / ✕ : clique le bouton qui a le focus. L'activation par défaut de GTK
/// passe par une capture du clavier, refusée aux surfaces layer-shell sous labwc.
fn activate_focused(window: &gtk::Window, key: &gdk::keys::Key) -> bool {
    if *key != keys::Return && *key != keys::KP_Enter && *key != keys::space && *key != keys::ISO_Enter {
        return false;
    }
    match window.focused_widget().and_then(|w| w.downcast::<gtk::Button>().ok()) {
        Some(button) => {
            button.clicked();
            true
        }
        None => false,
    }
}

fn focusable_widgets(widget: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
    if !widget.is_visible() || !widget.is_mapped() || !widget.is_sensitive() {
        return;
    }
    if widget.can_focus() && (widget.is::<gtk::Button>() || widget.is::<gtk::Scale>() || widget.is::<gtk::EventBox>()) {
        out.push(widget.clone());
        return;
    }
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        for child in container.children() {
            focusable_widgets(&child, out);
        }
    }
}

/// Navigation spatiale dans une surface layer-shell (la navigation GTK par défaut n'y
/// répond pas) : le voisin le plus proche dans la direction demandée.
fn spatial_focus(window: &gtk::Window, direction: gtk::DirectionType) -> bool {
    let mut items = Vec::new();
    if let Some(child) = window.child() {
        focusable_widgets(&child, &mut items);
    }
    let center = |w: &gtk::Widget| {
        let a = w.allocation();
        w.translate_coordinates(window, 0, 0).map(|(x, y)| (f64::from(x) + f64::from(a.width()) / 2.0, f64::from(y) + f64::from(a.height()) / 2.0))
    };
    let current = window.focused_widget().and_then(|f| items.iter().find(|w| **w == f || f.is_ancestor(*w)).cloned());
    let Some(current) = current else {
        return items.first().map(|w| w.grab_focus()).is_some();
    };
    let Some((ax, ay)) = center(&current) else { return false };
    let horizontal = matches!(direction, gtk::DirectionType::Left | gtk::DirectionType::Right);
    let winner = items
        .iter()
        .filter(|w| **w != current)
        .filter_map(|w| center(w).map(|(x, y)| (w, x - ax, y - ay)))
        .filter(|(_, dx, dy)| match direction {
            gtk::DirectionType::Left => *dx < -2.0,
            gtk::DirectionType::Right => *dx > 2.0,
            gtk::DirectionType::Up => *dy < -2.0,
            _ => *dy > 2.0,
        })
        .map(|(w, dx, dy)| {
            let (main, cross) = if horizontal { (dx.abs(), dy.abs()) } else { (dy.abs(), dx.abs()) };
            (w, main + cross * 2.4)
        })
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    match winner {
        Some((w, _)) => {
            w.grab_focus();
            true
        }
        None => false,
    }
}

fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    let (h, m, r) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{r:02}")
    } else {
        format!("{m}:{r:02}")
    }
}

/// Batterie de la manette PlayStation : (capacité, état, adresse).
fn controller_battery() -> Option<(u8, String, String)> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("ps-controller-battery-") || n.starts_with("sony_controller_battery_"))
        })
        .collect();
    entries.sort();
    entries.into_iter().find_map(|path| {
        let capacity: u8 = std::fs::read_to_string(path.join("capacity")).ok()?.trim().parse().ok()?;
        let status = std::fs::read_to_string(path.join("status")).ok()?.trim().to_string();
        let name = path.file_name()?.to_str()?.to_string();
        let address = name.rsplit(['-', '_']).next().unwrap_or("").to_uppercase();
        let address = if address.len() == 12 && !address.contains(':') {
            address.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(":")
        } else {
            name.split("battery-").last().or_else(|| name.split("battery_").last()).unwrap_or("").to_uppercase()
        };
        Some((capacity.min(100), status, address))
    })
}

fn status_label(status: &str) -> &str {
    match status {
        "Charging" => "En charge",
        "Discharging" => "Sur batterie",
        "Full" => "Chargée",
        "Not charging" => "Branchée",
        other => other,
    }
}

// ------------------------------------------------------------------ ensemble

struct Shared {
    proxy: EventLoopProxy<UserEvent>,
    osd: Osd,
    sidebar: Sidebar,
    panel: Panel,
    keyboard: Keyboard,
    panel_flag: PathBuf,
}

pub struct Layer {
    shared: Rc<Shared>,
}

impl Shared {
    fn send(self: &Rc<Self>, command: Command) {
        // Un choix fait à la manette dans la barre la referme, sauf le volume.
        let keep = matches!(command, Command::Volume(_) | Command::NavPeek(_) | Command::TvPanelState(_));
        let _ = self.proxy.send_event(UserEvent::Command(command));
        if self.sidebar.peeking.get() && !keep {
            let me = self.clone();
            glib::idle_add_local_once(move || me.end_peek());
        }
    }
}

impl Layer {
    pub fn new(proxy: EventLoopProxy<UserEvent>, panel_flag: PathBuf, _web: PathBuf) -> Self {
        let provider = gtk::CssProvider::new();
        if let Err(error) = provider.load_from_data(CSS.as_bytes()) {
            raspytv_core::rlog!("layer css failure {error}");
        }
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(&screen, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        let shared = Rc::new(Shared { proxy, osd: Osd::new(), sidebar: Sidebar::new(), panel: Panel::new(), keyboard: Keyboard::new(), panel_flag });
        let _ = std::fs::remove_file(&shared.panel_flag); // un panneau ne peut pas être resté ouvert
        Sidebar::connect(&shared);
        Panel::connect(&shared);
        Keyboard::connect(&shared);
        shared.sidebar.window.set_size_request(20, -1);
        shared.sidebar.window.show_all();
        shared.osd.box_.set_opacity(0.0);
        shared.osd.window.hide();
        shared.panel.dim.hide();
        shared.sidebar.rail.set_opacity(0.0);
        shared.sidebar.drawer.set_opacity(0.0);
        shared.collapse();
        Self { shared }
    }

    pub fn update(&self, state: &LayerState) {
        self.shared.rebuild(state);
    }

    pub fn apply_panel(&self, state: &PanelState) {
        if self.shared.panel.is_open.get() {
            self.shared.apply_panel_state(state);
        } else {
            *self.shared.panel.state.borrow_mut() = state.clone();
        }
    }

    pub fn toggle_panel(&self) {
        if self.shared.panel.is_open.get() {
            self.shared.close_panel();
        } else {
            self.shared.open_panel();
        }
    }

    pub fn toggle_peek(&self) {
        let shared = &self.shared;
        if shared.sidebar.peeking.get() {
            shared.end_peek();
        } else if !shared.panel.is_open.get() {
            shared.begin_peek();
        }
    }

    pub fn expand(&self) {
        self.shared.expand();
    }

    pub fn collapse(&self) {
        self.shared.collapse();
    }

    /// Remonte la barre au-dessus d'une fenêtre qui vient de s'ouvrir (Moonlight).
    pub fn raise(&self) {
        self.shared.sidebar.window.present();
    }

    pub fn show_keyboard(&self) {
        self.shared.keyboard.show();
    }

    pub fn hide_keyboard(&self) {
        self.shared.keyboard.window.hide();
    }

    pub fn set_controller_bluetooth(&self, connected: bool) {
        self.shared.panel.controller_bluetooth.set(connected);
        self.shared.update_controller();
    }
}

// ------------------------------------------------------------------ indicateur de volume

struct Osd {
    window: gtk::Window,
    box_: gtk::Box,
    icon: gtk::Image,
    segments: Vec<gtk::Box>,
    value: gtk::Label,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Osd {
    fn new() -> Self {
        let window = layer_window(false);
        // En bas à droite : en haut, il masquerait l'horloge de l'accueil.
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
        window.set_layer_shell_margin(Edge::Bottom, 36);
        window.set_layer_shell_margin(Edge::Right, 36);
        let box_ = styled(gtk::Box::new(gtk::Orientation::Horizontal, 12), &["osd"]);
        let image = icon("audio-volume-high-symbolic", 20);
        let (bar, segments) = segment_bar(VOLUME_SEGMENTS);
        let value = label("0 %", &["osd-value"], 1.0);
        box_.pack_start(&image, false, false, 0);
        box_.pack_start(&bar, false, false, 0);
        box_.pack_start(&value, false, false, 0);
        window.add(&box_);
        Self { window, box_, icon: image, segments, value, timer: RefCell::new(None) }
    }
}

impl Shared {
    fn show_osd(self: &Rc<Self>, value: f64, muted: bool) {
        if self.panel.is_open.get() {
            return; // le panneau affiche déjà le volume
        }
        let osd = &self.osd;
        let value = value.clamp(0.0, 100.0);
        set_icon(&osd.icon, volume_icon_name(value, muted), 20);
        osd.value.set_text(&if muted { "Muet".to_string() } else { format!("{} %", value as i32) });
        paint_segments(&osd.segments, value);
        let context = osd.box_.style_context();
        if muted {
            context.add_class("muted");
        } else {
            context.remove_class("muted");
        }
        fade(&osd.box_, &osd.window, 1.0, None, true);
        cancel(&osd.timer);
        let me = self.clone();
        *osd.timer.borrow_mut() = Some(timeout(1600, move || {
            me.osd.timer.borrow_mut().take();
            me.hide_osd();
        }));
    }

    fn hide_osd(&self) {
        fade(&self.osd.box_, &self.osd.window, 0.0, None, true);
    }
}

// ------------------------------------------------------------------ barre latérale

struct Sidebar {
    window: gtk::Window,
    hotspot: gtk::EventBox,
    rail: gtk::Box,
    drawer: gtk::Box,
    screen_height: i32,
    data: RefCell<LayerState>,
    expanded: Cell<bool>,
    peeking: Cell<bool>,
    drawer_kind: RefCell<Option<&'static str>>,
    collapse_timer: RefCell<Option<glib::SourceId>>,
    last_volume: Cell<Option<(u8, bool)>>,
}

impl Sidebar {
    fn new() -> Self {
        let window = layer_window(false);
        for edge in [Edge::Left, Edge::Top, Edge::Bottom] {
            window.set_anchor(edge, true);
        }
        let screen_height = gdk::Display::default().and_then(|d| d.primary_monitor().or_else(|| d.monitor(0))).map(|m| m.geometry().height()).unwrap_or(1080);
        let hotspot = styled(gtk::EventBox::new(), &["hotspot"]);
        hotspot.set_visible_window(true);
        hotspot.set_size_request(20, screen_height);
        let shell = styled(gtk::Box::new(gtk::Orientation::Horizontal, 0), &["shell"]);
        let rail = styled(gtk::Box::new(gtk::Orientation::Vertical, 6), &["rail"]);
        rail.set_valign(gtk::Align::Center);
        let drawer = styled(gtk::Box::new(gtk::Orientation::Vertical, 4), &["drawer"]);
        drawer.set_valign(gtk::Align::Center);
        shell.pack_start(&rail, false, false, 0);
        shell.pack_start(&drawer, false, false, 0);
        hotspot.add(&shell);
        window.add(&hotspot);
        hotspot.add_events(
            gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK | gdk::EventMask::POINTER_MOTION_MASK | gdk::EventMask::BUTTON_PRESS_MASK,
        );
        Self {
            window,
            hotspot,
            rail,
            drawer,
            screen_height,
            data: RefCell::new(LayerState::default()),
            expanded: Cell::new(false),
            peeking: Cell::new(false),
            drawer_kind: RefCell::new(None),
            collapse_timer: RefCell::new(None),
            last_volume: Cell::new(None),
        }
    }

    fn connect(shared: &Rc<Shared>) {
        let sidebar = &shared.sidebar;
        let me = shared.clone();
        sidebar.hotspot.connect_enter_notify_event(move |_, _| {
            me.hover();
            Propagation::Proceed
        });
        let me = shared.clone();
        sidebar.hotspot.connect_motion_notify_event(move |_, _| {
            me.hover();
            Propagation::Proceed
        });
        let me = shared.clone();
        sidebar.hotspot.connect_button_press_event(move |_, _| {
            me.hover();
            Propagation::Proceed
        });
        let me = shared.clone();
        sidebar.hotspot.connect_leave_notify_event(move |_, event| {
            if event.detail() != gdk::NotifyType::Inferior {
                me.leave();
            }
            Propagation::Proceed
        });
        let me = shared.clone();
        sidebar.window.connect_key_press_event(move |_, event| {
            if !me.sidebar.peeking.get() {
                return Propagation::Proceed;
            }
            if event.keyval() == keys::Escape {
                me.end_peek();
                return Propagation::Stop;
            }
            me.restart_peek_timer();
            if activate_focused(&me.sidebar.window, &event.keyval()) {
                return Propagation::Stop;
            }
            let key = event.keyval();
            let direction = if key == keys::Up {
                Some(gtk::DirectionType::Up)
            } else if key == keys::Down {
                Some(gtk::DirectionType::Down)
            } else if key == keys::Left {
                Some(gtk::DirectionType::Left)
            } else if key == keys::Right {
                Some(gtk::DirectionType::Right)
            } else {
                None
            };
            if let Some(direction) = direction {
                spatial_focus(&me.sidebar.window, direction);
                return Propagation::Stop;
            }
            Propagation::Proceed
        });
    }
}

impl Shared {
    fn rail_item(self: &Rc<Self>, icon_name: &str, text: &str, active: bool, on_click: impl Fn(&Rc<Shared>) + 'static) {
        let button = styled(gtk::Button::new(), &["rail-item"]);
        button.set_relief(gtk::ReliefStyle::None);
        button.set_tooltip_text(Some(text));
        if active {
            button.style_context().add_class("active");
        }
        button.set_halign(gtk::Align::Center);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        let image = icon(icon_name, 22);
        image.set_halign(gtk::Align::Center);
        let caption = label(text, &[], 0.5);
        caption.set_halign(gtk::Align::Center);
        caption.set_justify(gtk::Justification::Center);
        content.pack_start(&image, false, false, 0);
        content.pack_start(&caption, false, false, 0);
        button.add(&content);
        let me = self.clone();
        button.connect_clicked(move |_| on_click(&me));
        self.sidebar.rail.pack_start(&button, false, false, 0);
    }

    fn volume_row(self: &Rc<Self>) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_halign(gtk::Align::Center);
        for (icon_name, delta, tooltip) in
            [("audio-volume-low-symbolic", -5.0, "Baisser le volume"), ("audio-volume-high-symbolic", 5.0, "Augmenter le volume")]
        {
            let button = styled(gtk::Button::new(), &["rail-volume"]);
            button.set_relief(gtk::ReliefStyle::None);
            button.set_tooltip_text(Some(tooltip));
            let glyph = icon(icon_name, 18);
            button.add(&glyph);
            let me = self.clone();
            button.connect_clicked(move |_| {
                if let Some(volume) = me.sidebar.data.borrow().system_volume {
                    let muted = me.sidebar.data.borrow().system_muted;
                    me.show_osd(f64::from(volume) + delta, muted);
                }
                me.send(Command::Volume(delta));
            });
            row.pack_start(&button, false, false, 0);
        }
        self.sidebar.rail.pack_start(&row, false, false, 0);
    }

    fn separator(&self) {
        self.sidebar.rail.pack_start(&styled(gtk::Box::new(gtk::Orientation::Horizontal, 0), &["rail-separator"]), false, false, 0);
    }

    fn build_drawer(self: &Rc<Self>) {
        let sidebar = &self.sidebar;
        clear(&sidebar.drawer);
        let kind = *sidebar.drawer_kind.borrow();
        let Some(kind) = kind else {
            sidebar.drawer.hide();
            self.update_width();
            return;
        };
        let native = kind == "native";
        sidebar.drawer.pack_start(&label(if native { "APPLICATIONS" } else { "RACCOURCIS" }, &["drawer-title"], 0.0), false, false, 0);
        let data = sidebar.data.borrow().clone();
        let active = if data.native_mode { None } else { data.active_id.clone() };
        let apps: Vec<_> = data.apps.iter().filter(|a| a.native == native).cloned().collect();
        for app in &apps {
            let button = styled(gtk::Button::new(), &["drawer-item"]);
            button.set_relief(gtk::ReliefStyle::None);
            if Some(&app.id) == active.as_ref() {
                button.style_context().add_class("active");
            }
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            content.pack_start(&file_icon(&app.icon, "application-x-executable-symbolic", 30), false, false, 0);
            content.pack_start(&label(&app.name, &[], 0.0), true, true, 0);
            button.add(&content);
            let me = self.clone();
            let id = app.id.clone();
            button.connect_clicked(move |_| me.send(Command::Launch(id.clone())));
            sidebar.drawer.pack_start(&button, false, false, 0);
        }
        if apps.is_empty() {
            sidebar.drawer.pack_start(&label("Aucune application", &["drawer-empty"], 0.0), false, false, 0);
        }
        sidebar.drawer.show_all();
        self.update_width();
    }

    fn toggle_drawer(self: &Rc<Self>, kind: &'static str) {
        {
            let mut current = self.sidebar.drawer_kind.borrow_mut();
            *current = if *current == Some(kind) { None } else { Some(kind) };
        }
        self.build_drawer();
        if self.sidebar.peeking.get() {
            self.restart_peek_timer();
            if self.sidebar.drawer_kind.borrow().is_some() {
                focus_child(&self.sidebar.drawer, 1);
            }
        }
    }

    fn rebuild(self: &Rc<Self>, state: &LayerState) {
        let sidebar = &self.sidebar;
        *sidebar.data.borrow_mut() = state.clone();
        let (rail_focus, drawer_focus) = (focused_index(&sidebar.rail), focused_index(&sidebar.drawer));
        clear(&sidebar.rail);
        let active = if state.native_mode { None } else { state.active_id.clone() };
        let native_active = state.apps.iter().any(|a| a.native && Some(&a.id) == active.as_ref());
        let custom_active = state.apps.iter().any(|a| !a.native && Some(&a.id) == active.as_ref());
        let drawer = *sidebar.drawer_kind.borrow();
        self.rail_item("go-home-symbolic", "Accueil", active.is_none() && !state.native_mode, |s| s.send(Command::Home));
        self.rail_item("view-app-grid-symbolic", "Apps", native_active || drawer == Some("native"), |s| s.toggle_drawer("native"));
        self.rail_item("starred-symbolic", "Raccourcis", custom_active || drawer == Some("custom"), |s| s.toggle_drawer("custom"));
        self.rail_item("input-gaming-symbolic", "Jeux", state.native_mode, |s| s.send(Command::Moonlight));
        if state.native_mode {
            self.rail_item("input-keyboard-symbolic", "Clavier", false, |s| s.send(Command::Keyboard));
        }
        self.separator();
        self.volume_row();
        self.separator();
        self.rail_item("preferences-system-symbolic", "Réglages", false, |s| s.send(Command::Settings));
        self.build_drawer();
        if state.art_mode {
            sidebar.window.hide();
            return;
        }
        sidebar.window.show_all();
        if sidebar.drawer_kind.borrow().is_none() {
            sidebar.drawer.hide();
        }
        if sidebar.peeking.get() {
            match (drawer_focus, rail_focus) {
                (Some(index), _) if sidebar.drawer_kind.borrow().is_some() => {
                    focus_child(&sidebar.drawer, index);
                }
                (_, Some(index)) => {
                    focus_child(&sidebar.rail, index);
                }
                _ => {}
            }
        }
        if let Some(volume) = state.system_volume {
            let current = (volume, state.system_muted);
            if sidebar.last_volume.get().is_some_and(|last| last != current) {
                self.show_osd(f64::from(volume), state.system_muted);
            }
            sidebar.last_volume.set(Some(current));
        }
        if !sidebar.expanded.get() {
            self.finish_collapse();
        }
    }

    fn update_width(&self) {
        let sidebar = &self.sidebar;
        if sidebar.expanded.get() {
            let width = if sidebar.drawer_kind.borrow().is_some() { 96 + 262 } else { 96 };
            sidebar.window.set_size_request(width, -1);
            sidebar.window.resize(width, sidebar.screen_height);
        }
    }

    fn finish_collapse(&self) {
        let sidebar = &self.sidebar;
        if sidebar.expanded.get() {
            return;
        }
        *sidebar.drawer_kind.borrow_mut() = None;
        sidebar.drawer.hide();
        sidebar.rail.hide();
        sidebar.window.set_size_request(20, -1);
        sidebar.window.resize(20, sidebar.screen_height);
    }

    fn expand(self: &Rc<Self>) {
        cancel(&self.sidebar.collapse_timer);
        if self.sidebar.expanded.get() || self.sidebar.data.borrow().art_mode {
            return;
        }
        self.sidebar.expanded.set(true);
        self.sidebar.rail.show_all();
        self.update_width();
        fade(&self.sidebar.rail, &self.sidebar.window, 1.0, None, true);
        self.sidebar.drawer.set_opacity(1.0);
    }

    fn collapse(self: &Rc<Self>) {
        self.sidebar.collapse_timer.borrow_mut().take();
        self.release_keyboard();
        self.sidebar.expanded.set(false);
        let me = self.clone();
        // La fenêtre reste visible : sa bande de 20 px sert à détecter le survol du bord.
        fade(&self.sidebar.rail, &self.sidebar.window, 0.0, Some(Box::new(move || me.finish_collapse())), false);
    }

    fn schedule_collapse(self: &Rc<Self>, ms: u64) {
        cancel(&self.sidebar.collapse_timer);
        let me = self.clone();
        *self.sidebar.collapse_timer.borrow_mut() = Some(timeout(ms, move || me.collapse()));
    }

    fn hover(self: &Rc<Self>) {
        self.expand();
        // Un pointeur immobile sur le bord ne garde pas la barre ouverte indéfiniment.
        self.schedule_collapse(if self.sidebar.peeking.get() { PEEK_MS } else { HOVER_IDLE_MS });
    }

    fn leave(self: &Rc<Self>) {
        self.schedule_collapse(800);
    }

    fn restart_peek_timer(self: &Rc<Self>) {
        self.schedule_collapse(PEEK_MS);
    }

    fn begin_peek(self: &Rc<Self>) {
        let sidebar = &self.sidebar;
        sidebar.peeking.set(true);
        sidebar.window.set_accept_focus(true);
        sidebar.window.set_keyboard_mode(KeyboardMode::Exclusive);
        self.expand();
        self.restart_peek_timer();
        let me = self.clone();
        glib::idle_add_local_once(move || {
            focus_child(&me.sidebar.rail, 0);
        });
        self.send(Command::NavPeek(true));
    }

    fn release_keyboard(self: &Rc<Self>) {
        let sidebar = &self.sidebar;
        if !sidebar.peeking.get() {
            return;
        }
        sidebar.peeking.set(false);
        sidebar.window.set_keyboard_mode(KeyboardMode::None);
        sidebar.window.set_accept_focus(false);
        self.send(Command::NavPeek(false));
    }

    fn end_peek(self: &Rc<Self>) {
        cancel(&self.sidebar.collapse_timer);
        self.collapse();
    }
}

// ------------------------------------------------------------------ panneau de la manette

struct Panel {
    dim: gtk::Window,
    dim_box: gtk::Box,
    window: gtk::Window,
    outer: gtk::Box,
    is_open: Cell<bool>,
    state: RefCell<PanelState>,
    art_path: RefCell<String>,
    outputs_signature: RefCell<String>,
    seek_hold_until: Cell<Option<Instant>>,
    volume_hold_until: Cell<Option<Instant>>,
    seek_timer: RefCell<Option<glib::SourceId>>,
    volume_timer: RefCell<Option<glib::SourceId>>,
    tick_timer: RefCell<Option<glib::SourceId>>,
    last_input: Cell<Instant>,
    position: Cell<f64>,
    controller_address: RefCell<String>,
    controller_bluetooth: Cell<bool>,
    untouched: Cell<bool>,
    power_asking: Cell<bool>,
    ticks: Cell<u32>,
    // carte « lecture en cours »
    media_card: gtk::Box,
    media_app: gtk::Label,
    media_box: gtk::Box,
    art: gtk::Image,
    title: gtk::Label,
    artist: gtk::Label,
    elapsed: gtk::Label,
    scale: gtk::Scale,
    duration: gtk::Label,
    previous_button: gtk::Button,
    play_button: gtk::Button,
    play_icon: gtk::Image,
    next_button: gtk::Button,
    media_empty: gtk::Label,
    volume: gtk::EventBox,
    volume_box: gtk::Box,
    volume_icon: gtk::Image,
    volume_bar: gtk::Box,
    volume_segments: Vec<gtk::Box>,
    volume_label: gtk::Label,
    volume_value: Cell<f64>,
    outputs: gtk::Box,
    // carte « manette »
    controller_card: gtk::Box,
    connection: gtk::Label,
    battery_box: gtk::Box,
    battery_value: gtk::Label,
    battery_status: gtk::Label,
    battery_bar: gtk::ProgressBar,
    off_button: gtk::Button,
    power_button: gtk::Button,
    power_box: gtk::Box,
    power_off: gtk::Button,
    power_reboot: gtk::Button,
    power_cancel: gtk::Button,
}

fn control(icon_name: &str, tooltip: &str, main: bool) -> (gtk::Button, gtk::Image) {
    let button = styled(gtk::Button::new(), if main { &["control", "main"] } else { &["control"] });
    button.set_relief(gtk::ReliefStyle::None);
    button.set_tooltip_text(Some(tooltip));
    let image = icon(icon_name, if main { 28 } else { 22 });
    button.add(&image);
    (button, image)
}

fn action_button(icon_name: &str, text: &str, danger: bool) -> gtk::Button {
    let button = styled(gtk::Button::new(), if danger { &["action", "danger"] } else { &["action"] });
    button.set_relief(gtk::ReliefStyle::None);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.pack_start(&icon(icon_name, 18), false, false, 0);
    content.pack_start(&label(text, &[], 0.0), false, false, 0);
    button.add(&content);
    button
}

impl Panel {
    fn new() -> Self {
        // Le voile est créé avant le panneau : à ordre égal, la dernière surface passe devant.
        let dim = layer_window(false);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            dim.set_anchor(edge, true);
        }
        dim.set_exclusive_zone(-1);
        let dim_events = gtk::EventBox::new();
        dim_events.set_visible_window(false);
        let dim_box = styled(gtk::Box::new(gtk::Orientation::Horizontal, 0), &["dim"]);
        dim_box.set_hexpand(true);
        dim_box.set_vexpand(true);
        dim_events.add(&dim_box);
        dim.add(&dim_events);
        dim.set_widget_name("dim");

        let window = layer_window(true);
        let outer = styled(gtk::Box::new(gtk::Orientation::Horizontal, 20), &["panel"]);

        // --- lecture en cours et son
        let media_card = styled(gtk::Box::new(gtk::Orientation::Vertical, 14), &["card"]);
        media_card.set_size_request(660, -1);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.pack_start(&label("LECTURE EN COURS", &["card-title"], 0.0), true, true, 0);
        let media_app = label("", &["card-meta"], 1.0);
        header.pack_start(&media_app, false, false, 0);
        media_card.pack_start(&header, false, false, 0);
        let media_box = gtk::Box::new(gtk::Orientation::Vertical, 14);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        let art = styled(gtk::Image::new(), &["art"]);
        art.set_size_request(ART_SIZE, ART_SIZE);
        row.pack_start(&art, false, false, 0);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
        text.set_valign(gtk::Align::Center);
        let title = label("", &["media-title"], 0.0);
        title.set_line_wrap(true);
        title.set_lines(2);
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.set_max_width_chars(28);
        let artist = label("", &["media-artist"], 0.0);
        artist.set_ellipsize(pango::EllipsizeMode::End);
        artist.set_max_width_chars(36);
        text.pack_start(&title, false, false, 0);
        text.pack_start(&artist, false, false, 0);
        row.pack_start(&text, true, true, 0);
        media_box.pack_start(&row, false, false, 0);
        let progress = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let elapsed = label("0:00", &["time"], 0.0);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        scale.set_draw_value(false);
        scale.set_increments(5.0, 15.0);
        let duration = label("0:00", &["time"], 1.0);
        progress.pack_start(&elapsed, false, false, 0);
        progress.pack_start(&scale, true, true, 0);
        progress.pack_start(&duration, false, false, 0);
        media_box.pack_start(&progress, false, false, 0);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        controls.set_halign(gtk::Align::Center);
        let (previous_button, _) = control("media-skip-backward-symbolic", "Précédent", false);
        let (play_button, play_icon) = control("media-playback-start-symbolic", "Lecture / pause", true);
        let (next_button, _) = control("media-skip-forward-symbolic", "Suivant", false);
        for button in [&previous_button, &play_button, &next_button] {
            controls.pack_start(button, false, false, 0);
        }
        media_box.pack_start(&controls, false, false, 0);
        media_card.pack_start(&media_box, false, false, 0);
        let media_empty = label(
            "Rien ne joue pour l’instant.\nLance une musique dans NouTube Music : elle continuera\nen arrière-plan pendant que tu navigues.",
            &["media-empty"],
            0.0,
        );
        media_card.pack_start(&media_empty, false, false, 0);
        media_card.pack_start(&label("SON", &["section-title"], 0.0), false, false, 0);
        let volume = gtk::EventBox::new();
        volume.set_can_focus(true);
        volume.set_visible_window(false);
        let volume_box = styled(gtk::Box::new(gtk::Orientation::Horizontal, 12), &["volume-control"]);
        let volume_icon = icon("audio-volume-high-symbolic", 22);
        let (volume_bar, volume_segments) = segment_bar(PANEL_VOLUME_SEGMENTS);
        let volume_label = label("0 %", &["volume-value"], 1.0);
        volume_box.pack_start(&volume_icon, false, false, 0);
        volume_box.pack_start(&volume_bar, true, true, 0);
        volume_box.pack_start(&volume_label, false, false, 0);
        volume.add(&volume_box);
        volume.add_events(gdk::EventMask::BUTTON_PRESS_MASK | gdk::EventMask::KEY_PRESS_MASK);
        media_card.pack_start(&volume, false, false, 0);
        media_card.pack_start(&label("SORTIE AUDIO", &["section-title"], 0.0), false, false, 0);
        let outputs = gtk::Box::new(gtk::Orientation::Vertical, 6);
        media_card.pack_start(&outputs, false, false, 0);

        // --- manette et alimentation
        let controller_card = styled(gtk::Box::new(gtk::Orientation::Vertical, 14), &["card"]);
        controller_card.set_size_request(380, -1);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.pack_start(&label("MANETTE", &["card-title"], 0.0), true, true, 0);
        let connection = label("", &["card-meta"], 1.0);
        header.pack_start(&connection, false, false, 0);
        controller_card.pack_start(&header, false, false, 0);
        let battery_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let battery_value = label("—", &["battery-value"], 0.0);
        let battery_status = label("", &["battery-status"], 0.0);
        let battery_bar = gtk::ProgressBar::new();
        battery_box.pack_start(&battery_value, false, false, 0);
        battery_box.pack_start(&battery_status, false, false, 0);
        battery_box.pack_start(&battery_bar, false, false, 6);
        controller_card.pack_start(&battery_box, false, false, 0);
        let off_button = action_button("input-gaming-symbolic", "Éteindre la manette", false);
        controller_card.pack_start(&off_button, false, false, 4);
        let power_button = action_button("system-shutdown-symbolic", "Alimentation", false);
        controller_card.pack_start(&power_button, false, false, 0);
        let power_box = styled(gtk::Box::new(gtk::Orientation::Vertical, 8), &["power-box"]);
        power_box.pack_start(&label("Éteindre ou redémarrer le Raspberry Pi ?", &["power-question"], 0.0), false, false, 2);
        let power_off = action_button("system-shutdown-symbolic", "Éteindre le Raspberry Pi", true);
        let power_reboot = action_button("view-refresh-symbolic", "Redémarrer", true);
        let power_cancel = action_button("window-close-symbolic", "Annuler", false);
        power_box.pack_start(&power_off, false, false, 0);
        power_box.pack_start(&power_reboot, false, false, 0);
        power_box.pack_start(&power_cancel, false, false, 0);
        controller_card.pack_start(&power_box, false, false, 0);
        controller_card.pack_start(&gtk::Box::new(gtk::Orientation::Vertical, 0), true, true, 0);
        controller_card.pack_start(&label("✕ valider   ○ ou PS fermer\n‹ › régler   ↑ ↓ naviguer", &["hint"], 0.0), false, false, 0);

        outer.pack_start(&media_card, false, false, 0);
        outer.pack_start(&controller_card, false, false, 0);
        window.add(&outer);

        Self {
            dim,
            dim_box,
            window,
            outer,
            is_open: Cell::new(false),
            state: RefCell::new(PanelState::default()),
            art_path: RefCell::new("\u{0}".into()),
            outputs_signature: RefCell::new(String::new()),
            seek_hold_until: Cell::new(None),
            volume_hold_until: Cell::new(None),
            seek_timer: RefCell::new(None),
            volume_timer: RefCell::new(None),
            tick_timer: RefCell::new(None),
            last_input: Cell::new(Instant::now()),
            position: Cell::new(0.0),
            controller_address: RefCell::new(String::new()),
            controller_bluetooth: Cell::new(false),
            untouched: Cell::new(false),
            power_asking: Cell::new(false),
            ticks: Cell::new(0),
            media_card,
            media_app,
            media_box,
            art,
            title,
            artist,
            elapsed,
            scale,
            duration,
            previous_button,
            play_button,
            play_icon,
            next_button,
            media_empty,
            volume,
            volume_box,
            volume_icon,
            volume_bar,
            volume_segments,
            volume_label,
            volume_value: Cell::new(0.0),
            outputs,
            controller_card,
            connection,
            battery_box,
            battery_value,
            battery_status,
            battery_bar,
            off_button,
            power_button,
            power_box,
            power_off,
            power_reboot,
            power_cancel,
        }
    }

    fn connect(shared: &Rc<Shared>) {
        let p = &shared.panel;
        p.dim.add_events(gdk::EventMask::BUTTON_PRESS_MASK);
        let me = shared.clone();
        p.dim.connect_button_press_event(move |_, _| {
            me.close_panel();
            Propagation::Stop
        });
        let me = shared.clone();
        p.window.connect_key_press_event(move |_, event| me.panel_key(event));
        for (button, op) in [(&p.previous_button, "previous"), (&p.play_button, "toggle"), (&p.next_button, "next")] {
            let me = shared.clone();
            button.connect_clicked(move |_| {
                me.panel.last_input.set(Instant::now());
                me.send(Command::Media { op: op.into(), value: 0.0 });
            });
        }
        let me = shared.clone();
        p.scale.connect_change_value(move |_, _, value| {
            me.on_seek(value);
            Propagation::Proceed
        });
        let me = shared.clone();
        p.scale.connect_key_press_event(move |widget, event| me.move_vertically(widget.upcast_ref(), event));
        let me = shared.clone();
        p.volume.connect_key_press_event(move |widget, event| {
            let key = event.keyval();
            let value = me.panel.volume_value.get();
            if key == keys::Left || key == keys::minus || key == keys::KP_Subtract {
                me.change_volume(value - 2.0);
                return Propagation::Stop;
            }
            if key == keys::Right || key == keys::plus || key == keys::KP_Add {
                me.change_volume(value + 2.0);
                return Propagation::Stop;
            }
            me.move_vertically(widget.upcast_ref(), event)
        });
        let me = shared.clone();
        p.volume.connect_button_press_event(move |widget, event| {
            widget.grab_focus();
            let (x, y) = event.position();
            let width = me.panel.volume_bar.allocation().width();
            if let Some((bx, _)) = widget.translate_coordinates(&me.panel.volume_bar, x as i32, y as i32) {
                if width > 0 {
                    me.change_volume((f64::from(bx) / f64::from(width) * 100.0).round());
                }
            }
            Propagation::Stop
        });
        let focus_box = p.volume_box.clone();
        p.volume.connect_focus_in_event(move |_, _| {
            focus_box.style_context().add_class("focused");
            Propagation::Proceed
        });
        let focus_box = p.volume_box.clone();
        p.volume.connect_focus_out_event(move |_, _| {
            focus_box.style_context().remove_class("focused");
            Propagation::Proceed
        });
        let me = shared.clone();
        p.off_button.connect_clicked(move |_| {
            let address = me.panel.controller_address.borrow().clone();
            me.close_panel();
            if !address.is_empty() {
                std::thread::spawn(move || {
                    let _ = raspytv_core::sys::run("bluetoothctl", &["disconnect", &address], Duration::from_secs(10));
                });
            }
        });
        let me = shared.clone();
        p.power_button.connect_clicked(move |_| {
            me.panel.power_asking.set(true);
            me.refresh_visibility();
            me.panel.power_cancel.grab_focus();
        });
        let me = shared.clone();
        p.power_cancel.connect_clicked(move |_| {
            me.panel.power_asking.set(false);
            me.refresh_visibility();
            me.panel.power_button.grab_focus();
        });
        for (button, op) in [(&p.power_off, "poweroff"), (&p.power_reboot, "reboot")] {
            let me = shared.clone();
            button.connect_clicked(move |_| {
                me.panel.power_asking.set(false);
                me.close_panel();
                me.send(Command::SystemPower(op.into()));
            });
        }
    }
}

impl Shared {
    /// Haut/Bas quittent un réglage horizontal au lieu de modifier sa valeur.
    fn move_vertically(&self, widget: &gtk::Widget, event: &gdk::EventKey) -> Propagation {
        let direction = if event.keyval() == keys::Up {
            gtk::DirectionType::Up
        } else if event.keyval() == keys::Down {
            gtk::DirectionType::Down
        } else {
            return Propagation::Proceed;
        };
        if let Some(window) = widget.toplevel().and_then(|t| t.downcast::<gtk::Window>().ok()) {
            spatial_focus(&window, direction);
        }
        Propagation::Stop
    }

    fn panel_key(self: &Rc<Self>, event: &gdk::EventKey) -> Propagation {
        let p = &self.panel;
        p.last_input.set(Instant::now());
        p.untouched.set(false);
        let key = event.keyval();
        if key == keys::Escape {
            self.close_panel();
            return Propagation::Stop;
        }
        if activate_focused(&p.window, &key) {
            return Propagation::Stop;
        }
        if key == keys::Up || key == keys::Down {
            spatial_focus(&p.window, if key == keys::Up { gtk::DirectionType::Up } else { gtk::DirectionType::Down });
            return Propagation::Stop;
        }
        if key == keys::Left || key == keys::Right {
            return if self.switch_column(key == keys::Right) { Propagation::Stop } else { Propagation::Proceed };
        }
        Propagation::Proceed
    }

    /// ‹ › passent d'une carte à l'autre, sauf sur le volume et la barre de lecture qu'elles règlent.
    fn switch_column(&self, right: bool) -> bool {
        let p = &self.panel;
        let Some(focus) = p.window.focused_widget() else { return false };
        if focus == *p.volume.upcast_ref::<gtk::Widget>() || focus == *p.scale.upcast_ref::<gtk::Widget>() {
            return false;
        }
        // Précédent / Lecture / Suivant : d'abord le voisin sur la même ligne.
        let row = p.play_button.parent();
        if row.is_some() && focus.parent() == row {
            let neighbour = row.as_ref().and_then(|r| r.downcast_ref::<gtk::Box>()).map(|r| r.children()).unwrap_or_default();
            let index = neighbour.iter().position(|w| *w == focus).unwrap_or(0);
            let next = if right { neighbour.get(index + 1) } else { index.checked_sub(1).and_then(|i| neighbour.get(i)) };
            if let Some(next) = next {
                next.grab_focus();
                return true;
            }
        }
        let inside = |card: &gtk::Box| focus == *card.upcast_ref::<gtk::Widget>() || focus.is_ancestor(card);
        let targets: Vec<gtk::Widget> = if right && inside(&p.media_card) {
            if p.power_asking.get() {
                vec![p.power_cancel.clone().upcast()]
            } else {
                vec![p.off_button.clone().upcast(), p.power_button.clone().upcast()]
            }
        } else if !right && inside(&p.controller_card) {
            let mut list: Vec<gtk::Widget> = Vec::new();
            if p.media_box.is_visible() {
                list.push(p.play_button.clone().upcast());
            }
            let outputs = p.outputs.children();
            list.extend(outputs.iter().filter(|c| c.style_context().has_class("active")).cloned());
            list.extend(outputs);
            list.push(p.volume.clone().upcast());
            list
        } else {
            return false;
        };
        for target in targets {
            if target.is_visible() && target.is_sensitive() && target.is_mapped() {
                target.grab_focus();
                return true;
            }
        }
        false
    }

    fn open_panel(self: &Rc<Self>) {
        let p = &self.panel;
        if p.is_open.get() {
            return;
        }
        if self.sidebar.peeking.get() {
            self.end_peek();
        }
        self.hide_osd();
        p.is_open.set(true);
        p.last_input.set(Instant::now());
        p.untouched.set(true);
        let _ = std::fs::write(&self.panel_flag, b"");
        self.detect_connection();
        let state = p.state.borrow().clone();
        self.apply_panel_state(&state);
        p.power_asking.set(false);
        fade(&p.dim_box, &p.dim, 1.0, None, true);
        p.outer.set_opacity(0.0);
        p.window.show_all();
        // Après show_all, qui réaffiche tout : ce qui doit rester caché est caché de nouveau.
        self.update_controller();
        self.refresh_visibility();
        p.window.set_keyboard_mode(KeyboardMode::Exclusive);
        p.window.present();
        fade(&p.outer, &p.window, 1.0, None, true);
        let me = self.clone();
        glib::idle_add_local_once(move || me.initial_focus());
        let me = self.clone();
        *p.tick_timer.borrow_mut() = Some(glib::timeout_add_local(Duration::from_millis(250), move || me.panel_tick()));
        self.send(Command::TvPanelState(true));
    }

    /// Sans lecture, le focus part de la sortie audio active plutôt que du volume.
    fn initial_focus(&self) {
        let p = &self.panel;
        if p.media_box.is_visible() {
            p.play_button.grab_focus();
            return;
        }
        let outputs: Vec<gtk::Widget> = p.outputs.children().into_iter().filter(|c| c.is_visible()).collect();
        let target = outputs.iter().find(|c| c.style_context().has_class("active")).or(outputs.first()).cloned().unwrap_or_else(|| p.volume.clone().upcast());
        target.grab_focus();
    }

    fn close_panel(self: &Rc<Self>) {
        let p = &self.panel;
        if !p.is_open.get() {
            return;
        }
        p.is_open.set(false);
        p.power_asking.set(false);
        p.window.set_keyboard_mode(KeyboardMode::None);
        fade(&p.outer, &p.window, 0.0, None, true);
        fade(&p.dim_box, &p.dim, 0.0, None, true);
        cancel(&p.tick_timer);
        let _ = std::fs::remove_file(&self.panel_flag);
        self.send(Command::TvPanelState(false));
    }

    fn panel_tick(self: &Rc<Self>) -> glib::ControlFlow {
        let p = &self.panel;
        if !p.is_open.get() {
            p.tick_timer.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        if p.last_input.get().elapsed() > PANEL_IDLE {
            p.tick_timer.borrow_mut().take();
            let me = self.clone();
            glib::idle_add_local_once(move || me.close_panel());
            return glib::ControlFlow::Break;
        }
        // Entre deux mises à jour, la position avance d'elle-même : la barre reste fluide.
        let media = p.state.borrow().media.clone();
        if let Some(media) = media.filter(|m| m.playing) {
            if p.seek_hold_until.get().is_none_or(|t| Instant::now() > t) {
                let limit = if media.duration > 0.0 { media.duration } else { p.position.get() + 0.25 };
                p.position.set((p.position.get() + 0.25).min(limit));
                self.show_position(p.position.get(), media.duration);
            }
        }
        let ticks = p.ticks.get().wrapping_add(1);
        p.ticks.set(ticks);
        if ticks.is_multiple_of(4) {
            self.update_controller();
        }
        glib::ControlFlow::Continue
    }

    fn apply_panel_state(self: &Rc<Self>, state: &PanelState) {
        let p = &self.panel;
        *p.state.borrow_mut() = state.clone();
        if let Some(media) = &state.media {
            p.media_app.set_text(&media.app);
            p.title.set_text(if media.title.is_empty() { "Sans titre" } else { &media.title });
            p.artist.set_text(&media.artist);
            self.set_art(&media.artwork);
            set_icon(&p.play_icon, if media.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" }, 28);
            if p.seek_hold_until.get().is_none_or(|t| Instant::now() > t) {
                p.position.set(media.position);
                self.show_position(media.position, media.duration);
            }
        } else {
            p.media_app.set_text("");
        }
        if p.volume_hold_until.get().is_none_or(|t| Instant::now() > t) {
            self.set_volume_value(f64::from(state.audio.volume), state.audio.muted);
        }
        self.build_outputs(state);
        let had_media = p.media_box.is_visible();
        self.refresh_visibility();
        self.update_controller();
        // La lecture en cours arrive souvent juste après l'ouverture : le focus la rejoint.
        if state.media.is_some() && !had_media && p.is_open.get() && p.untouched.get() {
            p.play_button.grab_focus();
        }
    }

    fn refresh_visibility(&self) {
        let p = &self.panel;
        let has_media = p.state.borrow().media.is_some();
        p.media_box.set_visible(has_media);
        p.media_empty.set_visible(!has_media);
        p.off_button.set_visible(p.controller_bluetooth.get() && !p.controller_address.borrow().is_empty());
        p.power_box.set_visible(p.power_asking.get());
        p.power_button.set_visible(!p.power_asking.get());
    }

    fn show_position(&self, position: f64, duration: f64) {
        let p = &self.panel;
        p.scale.set_range(0.0, duration.max(1.0));
        p.scale.set_value(position.min(duration.max(1.0)));
        p.elapsed.set_text(&format_time(position));
        p.duration.set_text(&if duration > 0.0 { format_time(duration) } else { "—".into() });
    }

    fn set_art(&self, path: &str) {
        let p = &self.panel;
        if *p.art_path.borrow() == path {
            return;
        }
        *p.art_path.borrow_mut() = path.to_string();
        let pixbuf = (!path.is_empty()).then(|| gdk_pixbuf::Pixbuf::from_file_at_scale(path, ART_SIZE, ART_SIZE, true).ok()).flatten();
        match pixbuf {
            Some(pixbuf) => p.art.set_from_pixbuf(Some(&pixbuf)),
            None => {
                p.art.set_from_icon_name(Some("audio-x-generic-symbolic"), gtk::IconSize::Dialog);
                p.art.set_pixel_size(72);
            }
        }
    }

    fn set_volume_value(&self, value: f64, muted: bool) {
        let p = &self.panel;
        let value = value.clamp(0.0, 100.0).round();
        p.volume_value.set(value);
        p.volume_label.set_text(&if muted { "Muet".to_string() } else { format!("{value} %") });
        set_icon(&p.volume_icon, volume_icon_name(value, muted), 22);
        paint_segments(&p.volume_segments, value);
    }

    fn change_volume(self: &Rc<Self>, value: f64) {
        let p = &self.panel;
        self.set_volume_value(value, false);
        p.last_input.set(Instant::now());
        p.volume_hold_until.set(Some(Instant::now() + Duration::from_millis(1200)));
        cancel(&p.volume_timer);
        let me = self.clone();
        let value = p.volume_value.get();
        *p.volume_timer.borrow_mut() = Some(timeout(120, move || {
            me.panel.volume_timer.borrow_mut().take();
            me.send(Command::VolumeSet(value));
        }));
    }

    fn on_seek(self: &Rc<Self>, value: f64) {
        let p = &self.panel;
        p.last_input.set(Instant::now());
        p.position.set(value.max(0.0));
        p.seek_hold_until.set(Some(Instant::now() + Duration::from_millis(1600)));
        p.elapsed.set_text(&format_time(p.position.get()));
        cancel(&p.seek_timer);
        let me = self.clone();
        *p.seek_timer.borrow_mut() = Some(timeout(350, move || {
            me.panel.seek_timer.borrow_mut().take();
            let position = (me.panel.position.get() * 10.0).round() / 10.0;
            me.send(Command::Media { op: "seek".into(), value: position });
        }));
    }

    fn build_outputs(self: &Rc<Self>, state: &PanelState) {
        let p = &self.panel;
        let signature = serde_json::to_string(&(&state.audio.outputs, &state.audio.bluetooth)).unwrap_or_default();
        if *p.outputs_signature.borrow() == signature {
            return;
        }
        *p.outputs_signature.borrow_mut() = signature;
        let focus = focused_index(&p.outputs);
        clear(&p.outputs);
        for output in &state.audio.outputs {
            let icon_name = if output.headset || output.kind == "bluetooth" {
                "audio-headphones-symbolic"
            } else if output.kind == "hdmi" {
                "video-display-symbolic"
            } else {
                "audio-speakers-symbolic"
            };
            self.output_button(icon_name, &output.name, if output.active { "Active" } else { "" }, &output.id, output.active);
        }
        for (id, name) in &state.audio.bluetooth {
            self.output_button("bluetooth-active-symbolic", name, "Se connecter", id, false);
        }
        if state.audio.outputs.is_empty() && state.audio.bluetooth.is_empty() {
            p.outputs.pack_start(&label("Aucune sortie audio détectée", &["card-meta"], 0.0), false, false, 0);
        }
        p.outputs.show_all();
        if let (Some(index), true) = (focus, p.is_open.get()) {
            let count = p.outputs.children().len();
            if count > 0 {
                focus_child(&p.outputs, index.min(count - 1));
            }
        }
    }

    fn output_button(self: &Rc<Self>, icon_name: &str, name: &str, state_text: &str, id: &str, active: bool) {
        let button = styled(gtk::Button::new(), if active { &["output", "active"] } else { &["output"] });
        button.set_relief(gtk::ReliefStyle::None);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.pack_start(&icon(icon_name, 20), false, false, 0);
        let name_label = label(if name.is_empty() { "Sortie audio" } else { name }, &[], 0.0);
        name_label.set_ellipsize(pango::EllipsizeMode::End);
        content.pack_start(&name_label, true, true, 0);
        let state_label = label(state_text, &["output-state"], 1.0);
        content.pack_start(&state_label, false, false, 0);
        button.add(&content);
        let me = self.clone();
        let id = id.to_string();
        button.connect_clicked(move |_| {
            if active {
                return;
            }
            state_label.set_text(if id.starts_with("bt:") { "Connexion…" } else { "Activation…" });
            me.send(Command::AudioOutput(id.clone()));
        });
        self.panel.outputs.pack_start(&button, false, false, 0);
    }

    fn update_controller(&self) {
        let p = &self.panel;
        let battery = controller_battery();
        *p.controller_address.borrow_mut() = battery.as_ref().map(|b| b.2.clone()).unwrap_or_default();
        let style = p.battery_value.style_context();
        match battery {
            None => {
                style.remove_class("battery-value");
                style.add_class("battery-none");
                p.battery_value.set_text("Aucune manette connectée");
                p.battery_status.set_text("Appuie sur PS pour l’allumer.");
                p.battery_bar.hide();
                p.connection.set_text("");
            }
            Some((capacity, status, address)) => {
                style.remove_class("battery-none");
                style.add_class("battery-value");
                p.battery_bar.show();
                p.battery_value.set_text(&format!("{capacity} %"));
                p.battery_status.set_text(status_label(&status));
                p.battery_bar.set_fraction(f64::from(capacity) / 100.0);
                let context = p.battery_box.style_context();
                if capacity <= 15 && status != "Charging" {
                    context.add_class("battery-low");
                } else {
                    context.remove_class("battery-low");
                }
                p.connection.set_text(if p.controller_bluetooth.get() {
                    "Bluetooth"
                } else if !address.is_empty() {
                    "USB"
                } else {
                    ""
                });
            }
        }
        self.refresh_visibility();
    }

    /// `bluetoothctl` est lent : interrogé une fois à l'ouverture, hors de l'interface.
    fn detect_connection(&self) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let address = controller_battery().map(|b| b.2).unwrap_or_default();
            let connected = !address.is_empty() && raspytv_core::sys::quiet("bluetoothctl", &["info", &address]).contains("Connected: yes");
            let _ = proxy.send_event(UserEvent::Run(Box::new(move |app| {
                if let Some(layer) = &app.layer {
                    layer.set_controller_bluetooth(connected);
                }
            })));
        });
    }
}

// ------------------------------------------------------------------ clavier virtuel

struct Keyboard {
    window: gtk::Window,
    letters: gtk::Box,
    symbols: gtk::Box,
    shift: Cell<bool>,
    caps: Cell<bool>,
    modifiers: RefCell<Vec<(gtk::Button, &'static str)>>,
}

/// (affiché, valeur, valeur avec Maj)
type Key = (&'static str, &'static str, Option<&'static str>);

const LETTER_ROWS: [&[Key]; 4] = [
    &[
        ("&", "&", Some("1")),
        ("é", "é", Some("2")),
        ("\"", "\"", Some("3")),
        ("'", "'", Some("4")),
        ("(", "(", Some("5")),
        ("-", "-", Some("6")),
        ("è", "è", Some("7")),
        ("_", "_", Some("8")),
        ("ç", "ç", Some("9")),
        ("à", "à", Some("0")),
        (")", ")", Some("°")),
        ("=", "=", Some("+")),
    ],
    &[
        ("A", "a", None),
        ("Z", "z", None),
        ("E", "e", None),
        ("R", "r", None),
        ("T", "t", None),
        ("Y", "y", None),
        ("U", "u", None),
        ("I", "i", None),
        ("O", "o", None),
        ("P", "p", None),
        ("^", "^", Some("¨")),
        ("$", "$", Some("£")),
    ],
    &[
        ("Q", "q", None),
        ("S", "s", None),
        ("D", "d", None),
        ("F", "f", None),
        ("G", "g", None),
        ("H", "h", None),
        ("J", "j", None),
        ("K", "k", None),
        ("L", "l", None),
        ("M", "m", None),
        ("ù", "ù", Some("%")),
    ],
    &[
        ("<", "<", Some(">")),
        ("W", "w", None),
        ("X", "x", None),
        ("C", "c", None),
        ("V", "v", None),
        ("B", "b", None),
        ("N", "n", None),
        (",", ",", Some("?")),
        (";", ";", Some(".")),
        (":", ":", Some("/")),
        ("!", "!", Some("§")),
    ],
];

const SYMBOL_ROWS: [&str; 4] = ["1234567890", "@#€_&-+=()[]", "/\\:;\"'?!*%", ".,<>{}|~`^"];

impl Keyboard {
    fn new() -> Self {
        let window = layer_window(false);
        for edge in [Edge::Left, Edge::Right, Edge::Bottom] {
            window.set_anchor(edge, true);
        }
        let height = gdk::Display::default()
            .and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)))
            .map(|m| (f64::from(m.geometry().height()) * 0.3).round() as i32)
            .unwrap_or(300)
            .clamp(250, 330);
        window.set_size_request(-1, height);
        Self {
            window,
            letters: gtk::Box::new(gtk::Orientation::Vertical, 6),
            symbols: gtk::Box::new(gtk::Orientation::Vertical, 6),
            shift: Cell::new(false),
            caps: Cell::new(false),
            modifiers: RefCell::new(Vec::new()),
        }
    }

    fn show(&self) {
        self.window.show_all();
        self.symbols.hide();
    }

    fn connect(shared: &Rc<Shared>) {
        let k = &shared.keyboard;
        let root = styled(gtk::Box::new(gtk::Orientation::Vertical, 8), &["keyboard"]);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.pack_start(&label("Clavier", &["kb-title"], 0.0), false, false, 0);
        header.pack_start(&label("AZERTY · hors ligne", &["kb-hint"], 0.0), true, true, 0);
        let close = shared.key_button("Fermer ×", &["kb-key", "kb-done"], |s| s.osk(OskKey::Close));
        header.pack_start(&close, false, false, 0);
        root.pack_start(&header, false, false, 0);
        // Lettres
        for (index, row) in LETTER_ROWS.iter().enumerate() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.set_homogeneous(false);
            if index == 1 {
                line.pack_start(&shared.key_button("Tab", &["kb-key", "kb-mod"], |s| s.osk(OskKey::Key("tab".into()))), true, true, 0);
            }
            if index == 2 {
                let caps = shared.key_button("Verr. Maj", &["kb-key", "kb-mod"], |s| {
                    s.keyboard.caps.set(!s.keyboard.caps.get());
                    s.paint_modifiers();
                });
                k.modifiers.borrow_mut().push((caps.clone(), "caps"));
                line.pack_start(&caps, true, true, 0);
            }
            if index == 3 {
                let shift = shared.key_button("⇧ Maj", &["kb-key", "kb-mod"], |s| {
                    s.keyboard.shift.set(!s.keyboard.shift.get());
                    s.paint_modifiers();
                });
                k.modifiers.borrow_mut().push((shift.clone(), "shift"));
                line.pack_start(&shift, true, true, 0);
            }
            for &(shown, value, shifted) in row.iter() {
                let text = match shifted {
                    Some(s) => format!("{shown} {s}"),
                    None => shown.to_string(),
                };
                line.pack_start(&shared.key_button(&text, &["kb-key"], move |s| s.type_key(value, shifted)), true, true, 0);
            }
            let trailing: Option<(&str, &str)> = match index {
                0 => Some(("⌫", "backspace")),
                2 => Some(("Entrée", "enter")),
                3 => Some(("Suppr", "delete")),
                _ => None,
            };
            if let Some((text, key)) = trailing {
                line.pack_start(&shared.key_button(text, &["kb-key", "kb-mod"], move |s| s.osk(OskKey::Key(key.into()))), true, true, 0);
            }
            k.letters.pack_start(&line, true, true, 0);
        }
        k.letters.pack_start(&shared.bottom_row(true), true, true, 0);
        // Chiffres et symboles
        for (index, row) in SYMBOL_ROWS.iter().enumerate() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            for c in row.chars() {
                let value = c.to_string();
                line.pack_start(&shared.key_button(&value.clone(), &["kb-key"], move |s| s.osk(OskKey::Text(value.clone()))), true, true, 0);
            }
            let trailing = match index {
                0 => Some(("⌫", "backspace")),
                2 => Some(("Entrée", "enter")),
                3 => Some(("Suppr", "delete")),
                _ => None,
            };
            if let Some((text, key)) = trailing {
                line.pack_start(&shared.key_button(text, &["kb-key", "kb-mod"], move |s| s.osk(OskKey::Key(key.into()))), true, true, 0);
            }
            k.symbols.pack_start(&line, true, true, 0);
        }
        k.symbols.pack_start(&shared.bottom_row(false), true, true, 0);
        root.pack_start(&k.letters, true, true, 0);
        root.pack_start(&k.symbols, true, true, 0);
        k.window.add(&root);
    }
}

impl Shared {
    fn key_button(self: &Rc<Self>, text: &str, classes: &[&str], on_click: impl Fn(&Rc<Shared>) + 'static) -> gtk::Button {
        let button = styled(gtk::Button::with_label(text), classes);
        button.set_relief(gtk::ReliefStyle::None);
        button.set_can_focus(false);
        let me = self.clone();
        button.connect_clicked(move |_| on_click(&me));
        button
    }

    fn bottom_row(self: &Rc<Self>, letters: bool) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let switch = self.key_button(if letters { "?123" } else { "ABC" }, &["kb-key", "kb-mod"], move |s| {
            let k = &s.keyboard;
            if letters {
                k.letters.hide();
                k.symbols.show_all();
            } else {
                k.symbols.hide();
                k.letters.show_all();
            }
        });
        line.pack_start(&switch, true, true, 0);
        for text in ["/", "@", ".fr", ".com"] {
            line.pack_start(&self.key_button(text, &["kb-key"], move |s| s.osk(OskKey::Text(text.into()))), true, true, 0);
        }
        let space = self.key_button("Espace", &["kb-key", "kb-space"], |s| s.osk(OskKey::Text(" ".into())));
        line.pack_start(&space, true, true, 0);
        line.set_child_packing(&space, true, true, 0, gtk::PackType::Start);
        let arrows: &[(&str, &str)] =
            if letters { &[("←", "left"), ("→", "right")] } else { &[("←", "left"), ("↑", "up"), ("↓", "down"), ("→", "right")] };
        for &(text, key) in arrows {
            line.pack_start(&self.key_button(text, &["kb-key"], move |s| s.osk(OskKey::Key(key.into()))), true, true, 0);
        }
        line.pack_start(&self.key_button("Terminé", &["kb-key", "kb-done"], |s| s.osk(OskKey::Close)), true, true, 0);
        line
    }

    fn type_key(self: &Rc<Self>, value: &str, shifted: Option<&str>) {
        let k = &self.keyboard;
        let letter = value.chars().count() == 1 && value.chars().all(char::is_alphabetic);
        let text = if letter {
            if k.caps.get() != k.shift.get() {
                value.to_uppercase()
            } else {
                value.to_string()
            }
        } else if k.shift.get() {
            shifted.unwrap_or(value).to_string()
        } else {
            value.to_string()
        };
        self.osk(OskKey::Text(text));
        if k.shift.get() {
            k.shift.set(false);
            self.paint_modifiers();
        }
    }

    fn paint_modifiers(&self) {
        let k = &self.keyboard;
        for (button, kind) in k.modifiers.borrow().iter() {
            let on = if *kind == "caps" { k.caps.get() } else { k.shift.get() };
            let context = button.style_context();
            if on {
                context.add_class("active");
            } else {
                context.remove_class("active");
            }
        }
    }

    fn osk(&self, key: OskKey) {
        let _ = self.proxy.send_event(UserEvent::Osk(key));
    }
}
