//! Fils de fond : socket de commande, surveillance du son, manette, signaux.

use std::time::Duration;

#[cfg(unix)]
use raspytv_core::protocol;
use raspytv_core::rlog;
use tao::event_loop::EventLoopProxy;

use crate::app::{App, UserEvent};

/// Si la version Electron écoute déjà `raspytv-nav.sock`, on ne lui prend pas.
pub fn choose_prefix() -> String {
    #[cfg(unix)]
    {
        let paths = raspytv_core::paths::Paths::from_env();
        let socket = paths.nav_socket("raspytv");
        if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            rlog!("socket {} déjà utilisé (version Electron ?) : préfixe raspytv-rs", socket.display());
            return "raspytv-rs".into();
        }
    }
    "raspytv".into()
}

pub fn start(app: &App, proxy: EventLoopProxy<UserEvent>) {
    #[cfg(unix)]
    nav_socket(app.paths.nav_socket(&crate::socket_prefix()), proxy.clone());
    #[cfg(target_os = "linux")]
    signals(proxy.clone());
    audio_watch(proxy.clone());
    if std::env::var("RASPYTV_GAMEPAD").map(|v| v != "0").unwrap_or(true) {
        gamepad(proxy);
    }
    let _ = app;
}

/// `$XDG_RUNTIME_DIR/raspytv-nav.sock` : une commande JSON par connexion (format Electron).
#[cfg(unix)]
fn nav_socket(path: std::path::PathBuf, proxy: EventLoopProxy<UserEvent>) {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => return rlog!("nav socket failure {error}"),
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    rlog!("socket de commande : {}", path.display());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(700)));
            let mut raw = Vec::new();
            let _ = stream.take(4096).read_to_end(&mut raw);
            let value = serde_json::from_slice::<serde_json::Value>(&raw).ok();
            // Mode débogage seulement : exécuter un script dans une vue et journaliser le résultat.
            if crate::app::debug() && value.as_ref().and_then(|v| v["action"].as_str()) == Some("debug-eval") {
                let value = value.unwrap_or_default();
                let id = value["id"].as_str().unwrap_or("").to_string();
                let js = value["js"].as_str().unwrap_or("").to_string();
                let _ = proxy.send_event(UserEvent::Run(Box::new(move |app| app.debug_eval(&id, &js))));
                continue;
            }
            match value.as_ref().and_then(protocol::parse) {
                Some(command) => {
                    let _ = proxy.send_event(UserEvent::Command(command));
                }
                None => rlog!("nav command ignored"),
            }
        }
    });
}

/// SIGTERM / SIGINT : arrêt propre (code 0, la boucle kiosque s'arrête).
/// SIGUSR2 : déploie la barre latérale (compatibilité avec la version Electron).
#[cfg(target_os = "linux")]
fn signals(proxy: EventLoopProxy<UserEvent>) {
    for signal in [libc_sig::SIGTERM, libc_sig::SIGINT] {
        let proxy = proxy.clone();
        glib::unix_signal_add(signal, move || {
            let _ = proxy.send_event(UserEvent::Run(Box::new(|app| {
                rlog!("arrêt demandé");
                app.shutdown();
                std::process::exit(0);
            })));
            glib::ControlFlow::Continue
        });
    }
    glib::unix_signal_add(libc_sig::SIGUSR2, move || {
        let _ = proxy.send_event(UserEvent::Run(Box::new(|app| {
            if let Some(layer) = &app.layer {
                layer.expand();
            }
        })));
        glib::ControlFlow::Continue
    });
}

#[cfg(target_os = "linux")]
mod libc_sig {
    pub const SIGINT: i32 = 2;
    pub const SIGTERM: i32 = 15;
    pub const SIGUSR2: i32 = 12;
}

/// `pactl subscribe` : chaque changement (casque, sortie, volume) relit l'état, 250 ms plus tard.
fn audio_watch(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || loop {
        let child = std::process::Command::new("pactl")
            .arg("subscribe")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            // Pas de pactl (Windows, par exemple) : inutile d'insister souvent.
            std::thread::sleep(Duration::from_secs(60));
            continue;
        };
        if let Some(stdout) = child.stdout.take() {
            use std::io::BufRead;
            let (sender, receiver) = std::sync::mpsc::channel::<()>();
            let debounce_proxy = proxy.clone();
            std::thread::spawn(move || {
                while receiver.recv().is_ok() {
                    std::thread::sleep(Duration::from_millis(250));
                    while receiver.try_recv().is_ok() {}
                    let _ = debounce_proxy.send_event(UserEvent::Run(Box::new(|app| app.refresh_audio())));
                }
            });
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if raspytv_core::audio::subscribe_line_matters(&line) {
                    let _ = sender.send(());
                }
            }
        }
        let _ = child.wait();
        std::thread::sleep(Duration::from_secs(3));
    });
}

/// Manette (gilrs) : croix, ✕ (valider) et ○ (retour), comme la boucle `navigator.getGamepads` d'Electron.
/// Les boutons PS, Share, Options restent au service TelecomHand (socket).
fn gamepad(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let mut gilrs = match gilrs::Gilrs::new() {
            Ok(gilrs) => gilrs,
            Err(error) => return rlog!("manette indisponible : {error}"),
        };
        loop {
            while let Some(gilrs::Event { event, .. }) = gilrs.next_event_blocking(Some(Duration::from_secs(1))) {
                if let gilrs::EventType::ButtonPressed(button, _) = event {
                    let key = match button {
                        gilrs::Button::DPadUp => "ArrowUp",
                        gilrs::Button::DPadDown => "ArrowDown",
                        gilrs::Button::DPadLeft => "ArrowLeft",
                        gilrs::Button::DPadRight => "ArrowRight",
                        gilrs::Button::South => "Enter",
                        gilrs::Button::East => "Escape",
                        _ => continue,
                    };
                    let _ = proxy.send_event(UserEvent::Gamepad(key.into()));
                }
            }
        }
    });
}
