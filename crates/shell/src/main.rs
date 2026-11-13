//! RaspyTV : interface Smart TV pour Raspberry Pi, en Rust.
//!
//! Un seul processus : la fenêtre kiosque (tao + wry), une vue web par service,
//! la barre latérale / le panneau PS / le clavier en GTK layer-shell (Linux),
//! et des fils de fond (socket de commande, `pactl subscribe`, manette).

mod app;
mod background;
mod host;
#[cfg(target_os = "linux")]
mod layer;
mod protocol;
mod rpc;

use raspytv_core::paths::Paths;
use raspytv_core::rlog;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Fullscreen, WindowBuilder};

use crate::app::{App, UserEvent};

/// Préfixe des sockets : `raspytv`, ou `raspytv-rs` si la version Electron les occupe déjà.
pub fn socket_prefix() -> String {
    static PREFIX: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PREFIX.get_or_init(background::choose_prefix).clone()
}

/// `RASPYTV_SELFTEST=<id>` : ouvre le service, revient à l'accueil puis quitte (vérification rapide).
fn self_test(app: &App) {
    let Ok(id) = std::env::var("RASPYTV_SELFTEST") else { return };
    rlog!("self-test : {id}");
    app.after(std::time::Duration::from_secs(4), move |app| {
        let result = app.launch(&id);
        rlog!("self-test launch {id} : {result:?}");
    });
    app.after(std::time::Duration::from_secs(14), |app| {
        app.show_home();
        rlog!("self-test : retour à l'accueil");
    });
    app.after(std::time::Duration::from_secs(16), |app| {
        rlog!("self-test terminé : {}", app.state_json()["apps"].as_array().map(|a| a.iter().filter(|x| x["open"] == true).count()).unwrap_or(0));
        app.shutdown();
        std::process::exit(0);
    });
}

fn main() {
    let paths = Paths::from_env();
    paths.ensure();
    raspytv_core::log::init(paths.logs());
    rlog!("RaspyTV {} démarre (web : {})", raspytv_core::VERSION, paths.web.display());
    if std::env::args().any(|a| a == "--version") {
        println!("raspytv {}", raspytv_core::VERSION);
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        rlog!("panic {info}");
    }));

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let windowed = std::env::args().any(|a| a == "--windowed");
    let window = WindowBuilder::new()
        .with_title("RaspyTV")
        .with_decorations(windowed)
        .with_fullscreen((!windowed).then_some(Fullscreen::Borderless(None)))
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 720.0))
        .build(&event_loop)
        .expect("fenêtre principale");

    let mut app = App::new(window, proxy.clone(), paths);
    #[cfg(target_os = "linux")]
    {
        app.layer = Some(layer::Layer::new(proxy.clone(), app.paths.panel_flag(&socket_prefix()), app.paths.web.clone()));
        app.write_layer_state();
    }
    background::start(&app, proxy);
    self_test(&app);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => rlog!("interface prête"),
            Event::UserEvent(event) => match event {
                UserEvent::Run(job) => job(&mut app),
                UserEvent::HomeIpc(body) => rpc::handle(&mut app, &body),
                UserEvent::ServiceIpc { id, body } => app.on_service_ipc(&id, &body),
                UserEvent::PageLoad { id, finished, url } => app.on_page_load(&id, finished, &url),
                UserEvent::Command(command) => app.handle_command(command),
                UserEvent::Gamepad(key) => app.on_gamepad(&key),
                UserEvent::Osk(key) => app.on_osk(key),
            },
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => app.resize(),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.shutdown();
                *control_flow = ControlFlow::Exit;
            }
            Event::LoopDestroyed => app.shutdown(),
            _ => {}
        }
    });
}
