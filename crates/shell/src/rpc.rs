//! Appels de l'accueil (`window.ipc.postMessage({id, method, params})`).
//!
//! Chaque méthode vérifie ses paramètres ; les réponses repartent par
//! `window.__raspyHome.receive({t:'reply', …})`.

use raspytv_core::config::{valid_icon, valid_shortcut, Config, Shortcut};
use raspytv_core::{network, rlog, stats, sys, weather};
use serde_json::{json, Value};

use crate::app::{App, ENGINE};
use crate::protocol;

fn random_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("custom-{:x}{:04x}", nanos, std::process::id() & 0xffff)
}

fn string(params: &Value, max: usize) -> Result<String, String> {
    params.as_str().filter(|s| !s.is_empty() && s.len() <= max).map(String::from).ok_or_else(|| "Paramètre invalide".to_string())
}

pub fn handle(app: &mut App, body: &str) {
    let Ok(message) = serde_json::from_str::<Value>(body) else { return };
    let id = message.get("id").and_then(Value::as_u64);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("").to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result: Option<Result<Value, String>> = match method.as_str() {
        "state" => Some(Ok(app.state_json())),
        "settings.save" => Some(if params.is_object() {
            let next = app.config().patched(&params, app.defaults());
            app.update_config(next);
            Ok(app.state_json())
        } else {
            Err("Réglages invalides".into())
        }),
        "app.launch" => Some(string(&params, 200).and_then(|id| app.launch(&id)).map(|_| Value::Null)),
        "app.close" => Some(string(&params, 200).map(|id| {
            app.close_service(&id, false, "user");
            Value::Null
        })),
        "home" => {
            app.show_home();
            Some(Ok(Value::Null))
        }
        "tv.settings" => {
            app.tv_settings_press();
            Some(Ok(Value::Null))
        }
        "shell.page" => {
            app.set_shell_page(params.as_str().unwrap_or(""));
            None
        }
        "art.mode" => {
            app.set_art_mode(params.as_bool().unwrap_or(false));
            None
        }
        "keyboard.focus" => {
            app.keyboard_focus(params.as_bool().unwrap_or(false));
            None
        }
        "quit" => {
            rlog!("quit requested from home");
            app.shutdown();
            std::process::exit(0);
        }
        "art.list" => Some(Ok(protocol::art(app.shared()))),
        "backgrounds.list" => Some(Ok(protocol::backgrounds(app.shared()))),
        "weather" => {
            let site = app.site().clone();
            let cache = app.paths.weather_cache();
            app.spawn(move || weather::fetch(&site, &cache), move |app, value| app.reply(id, Ok(value)));
            None
        }
        "network" => {
            app.spawn(network::status, move |app, status| app.reply(id, Ok(json!(status))));
            None
        }
        "stats" => {
            let (resident, limit) = app.resident();
            app.spawn(move || stats::collect(resident, limit, ENGINE), move |app, stats| app.reply(id, Ok(json!(stats))));
            None
        }
        "audio.get" => {
            let work = app.audio_outputs();
            app.spawn(work, move |app, state| app.reply(id, Ok(json!(state))));
            None
        }
        "audio.output" => match string(&params, 300) {
            Ok(output) => {
                app.spawn(
                    move || raspytv_core::audio::set_output(&output),
                    move |app, result| {
                        app.refresh_audio();
                        app.reply(id, result.map(|s| json!(s)));
                    },
                );
                None
            }
            Err(error) => Some(Err(error)),
        },
        "audio.volume" => {
            let value = params.as_f64().unwrap_or(0.0);
            app.spawn(
                move || raspytv_core::audio::set_volume(value),
                move |app, result| {
                    app.refresh_audio();
                    app.reply(id, result.map(|s| json!(s)));
                },
            );
            None
        }
        "jellyfin.hero" => {
            app.jellyfin_hero(id, params.get("force") == Some(&Value::Bool(true)));
            None
        }
        "jellyfin.play" => Some(string(&params, 64).and_then(|item| app.play_jellyfin(&item)).map(|_| json!(true))),
        "moonlight.games" => {
            app.moonlight_games(id);
            None
        }
        "moonlight.open" => {
            app.launch_native(String::new(), id);
            None
        }
        "moonlight.play" => match string(&params, 200) {
            Ok(name) => {
                app.launch_native(name, id);
                None
            }
            Err(error) => Some(Err(error)),
        },
        "system.action" => match string(&params, 16) {
            Ok(action) => {
                app.spawn(move || sys::power(&action), move |app, result| app.reply(id, result.map(|_| json!(true))));
                None
            }
            Err(error) => Some(Err(error)),
        },
        "shortcut.add" => {
            add_shortcut(app, id, &params);
            None
        }
        "shortcut.remove" => Some(string(&params, 200).and_then(|sid| {
            let mut config: Config = app.config().clone();
            let index = config.shortcuts.iter().position(|s| s.id == sid).ok_or("Seules les applications personnalisées peuvent être supprimées")?;
            app.close_service(&sid, true, "shortcut-removed");
            config.shortcuts.remove(index);
            config.app_order.retain(|x| *x != sid);
            config.recent.retain(|x| *x != sid);
            app.update_config(config);
            Ok(json!(true))
        })),
        _ => Some(Err(format!("Méthode inconnue : {method}"))),
    };
    if let Some(result) = result {
        app.reply(id, result);
    }
}

fn add_shortcut(app: &mut App, id: Option<u64>, params: &Value) {
    let icon_dir = app.paths.icons();
    let icon = params.get("icon").and_then(Value::as_str).filter(|i| valid_icon(i, &icon_dir)).map(String::from);
    let item = Shortcut {
        id: random_id(),
        name: params.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        url: params.get("url").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        icon,
        color: "#4f8cff".into(),
        description: "Raccourci personnalisé".into(),
    };
    if !valid_shortcut(&item) {
        return app.reply(id, Err("Nom ou URL invalide".into()));
    }
    app.spawn(
        move || {
            let mut item = item;
            if item.icon.is_none() {
                item.icon = raspytv_core::icon::fetch(&item.url, &icon_dir);
            }
            item
        },
        move |app, item| {
            let mut config = app.config().clone();
            config.app_order.push(item.id.clone());
            config.shortcuts.push(item.clone());
            app.update_config(config);
            app.reply(id, Ok(json!(item)));
        },
    );
}
