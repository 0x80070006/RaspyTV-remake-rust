//! Placement des vues web dans la fenêtre.
//!
//! - Linux (Wayland) : chaque vue est une page d'une `gtk::Stack` ; l'écran de
//!   chargement est posé par-dessus dans un `gtk::Overlay`. Pas de fenêtres enfants.
//! - Windows (développement) : vues enfants WebView2 superposées, affichées ou
//!   masquées, redimensionnées avec la fenêtre.

use tao::window::Window;
use wry::{WebView, WebViewBuilder};

pub struct Host {
    #[cfg(target_os = "linux")]
    stack: gtk::Stack,
    #[cfg(target_os = "linux")]
    loading: gtk::Box,
}

#[cfg(target_os = "linux")]
impl Host {
    pub fn new(window: &Window) -> Self {
        use gtk::prelude::*;
        use tao::platform::unix::WindowExtUnix;
        let vbox = window.default_vbox().expect("gtk vbox");
        let overlay = gtk::Overlay::new();
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::None);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        overlay.add(&stack);
        let loading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        loading.set_hexpand(true);
        loading.set_vexpand(true);
        overlay.add_overlay(&loading);
        vbox.pack_start(&overlay, true, true, 0);
        overlay.show_all();
        loading.hide();
        Self { stack, loading }
    }

    pub fn build(&self, name: &str, builder: WebViewBuilder, _window: &Window) -> wry::Result<WebView> {
        use gtk::prelude::*;
        use wry::WebViewBuilderExtUnix;
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        self.stack.add_named(&container, name);
        container.show_all();
        builder.build_gtk(&container)
    }

    pub fn build_loading(&self, builder: WebViewBuilder, _window: &Window) -> wry::Result<WebView> {
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(&self.loading)
    }

    pub fn remove(&self, name: &str) {
        use gtk::prelude::*;
        if let Some(child) = self.stack.child_by_name(name) {
            self.stack.remove(&child);
            unsafe { child.destroy() };
        }
    }

    pub fn present(&self, name: &str, _views: &[(&str, &WebView)]) {
        use gtk::prelude::*;
        self.stack.set_visible_child_name(name);
    }

    pub fn loading(&self, visible: bool, _view: Option<&WebView>) {
        use gtk::prelude::*;
        if visible {
            self.loading.show_all();
        } else {
            self.loading.hide();
        }
    }

    pub fn resize(&self, _window: &Window, _views: &[&WebView]) {}
}

#[cfg(not(target_os = "linux"))]
fn full_bounds(window: &Window) -> wry::Rect {
    let size = window.inner_size();
    wry::Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(size.width, size.height).into() }
}

#[cfg(not(target_os = "linux"))]
impl Host {
    pub fn new(_window: &Window) -> Self {
        Self {}
    }

    pub fn build(&self, _name: &str, builder: WebViewBuilder, window: &Window) -> wry::Result<WebView> {
        builder.with_bounds(full_bounds(window)).build_as_child(window)
    }

    pub fn build_loading(&self, builder: WebViewBuilder, window: &Window) -> wry::Result<WebView> {
        builder.with_bounds(full_bounds(window)).with_visible(false).build_as_child(window)
    }

    pub fn remove(&self, _name: &str) {}

    pub fn present(&self, name: &str, views: &[(&str, &WebView)]) {
        for (id, view) in views {
            let _ = view.set_visible(*id == name);
        }
    }

    pub fn loading(&self, visible: bool, view: Option<&WebView>) {
        if let Some(view) = view {
            let _ = view.set_visible(visible);
        }
    }

    pub fn resize(&self, window: &Window, views: &[&WebView]) {
        for view in views {
            let _ = view.set_bounds(full_bounds(window));
        }
    }
}
