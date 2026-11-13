//! Son du système via `pactl` (PipeWire / PulseAudio) et casques Bluetooth via `bluetoothctl`.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::sys;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sink {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub headset: bool,
    pub detail: String,
    pub volume: u8,
    pub muted: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothDevice {
    pub id: String,
    pub mac: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
    pub sinks: Vec<Sink>,
    pub active_id: String,
    pub volume: u8,
    pub muted: bool,
    #[serde(default)]
    pub unavailable: bool,
    /// Casques jumelés mais pas encore connectés (proposés pour s'y connecter).
    pub bluetooth: Vec<BluetoothDevice>,
}

/// Nature d'une sortie d'après son bus réel : un casque USB n'est pas un casque Bluetooth.
pub fn sink_kind(name: &str, properties: &Value) -> (&'static str, bool, &'static str) {
    let prop = |k: &str| properties.get(k).and_then(Value::as_str).unwrap_or("");
    let bus = prop("device.bus");
    let headset = matches!(prop("device.form_factor"), "headset" | "headphone");
    if bus == "bluetooth" || name.starts_with("bluez_") {
        ("bluetooth", true, "Bluetooth")
    } else if bus == "usb" || name.to_lowercase().contains("usb") {
        ("usb", headset, "USB")
    } else if name.to_lowercase().contains("hdmi") {
        ("hdmi", false, "HDMI")
    } else {
        ("internal", headset, "Intégrée")
    }
}

/// Interprète `pactl --format=json list sinks`.
pub fn parse_sinks(json: &str, default_sink: &str) -> Vec<Sink> {
    let list: Vec<Value> = serde_json::from_str(json).unwrap_or_default();
    list.iter()
        .map(|sink| {
            let name = sink.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let props = sink.get("properties").cloned().unwrap_or(Value::Null);
            let (kind, headset, detail) = sink_kind(&name, &props);
            let volume = sink
                .get("volume")
                .and_then(Value::as_object)
                .and_then(|channels| channels.values().next())
                .and_then(|c| c.get("value_percent"))
                .and_then(Value::as_str)
                .and_then(|p| p.trim_end_matches('%').trim().parse::<f64>().ok())
                .unwrap_or(0.0)
                .clamp(0.0, 100.0) as u8;
            let label = sink.get("description").and_then(Value::as_str).or_else(|| props.get("node.nick").and_then(Value::as_str)).unwrap_or(&name).to_string();
            Sink {
                active: name == default_sink.trim(),
                id: name,
                name: label,
                kind: kind.into(),
                headset,
                detail: detail.into(),
                volume,
                muted: sink.get("mute").and_then(Value::as_bool).unwrap_or(false),
            }
        })
        .collect()
}

pub fn state() -> AudioState {
    let sinks = sys::checked("pactl", &["--format=json", "list", "sinks"], Duration::from_secs(3));
    let default = sys::checked("pactl", &["get-default-sink"], Duration::from_secs(3));
    match (sinks, default) {
        (Ok(json), Ok(default)) => {
            let sinks = parse_sinks(&json, &default);
            let active = sinks.iter().find(|s| s.active).or(sinks.first()).cloned();
            AudioState {
                active_id: active.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
                volume: active.as_ref().map(|s| s.volume).unwrap_or(0),
                muted: active.as_ref().map(|s| s.muted).unwrap_or(false),
                sinks,
                ..AudioState::default()
            }
        }
        (Err(error), _) | (_, Err(error)) => {
            crate::rlog!("audio state unavailable: {error}");
            AudioState { unavailable: true, ..AudioState::default() }
        }
    }
}

pub fn set_volume(value: f64) -> Result<AudioState, String> {
    let volume = value.round().clamp(0.0, 100.0) as u8;
    sys::checked("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{volume}%")], Duration::from_secs(3))?;
    if volume > 0 {
        sys::checked("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "0"], Duration::from_secs(3))?;
    }
    Ok(state())
}

pub fn change_volume(delta: f64) -> Result<AudioState, String> {
    let current = state();
    set_volume(f64::from(current.volume) + delta.clamp(-20.0, 20.0))
}

pub fn valid_mac(mac: &str) -> bool {
    let parts: Vec<&str> = mac.split(':').collect();
    parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `bluetoothctl devices Paired` → (adresse, nom).
pub fn parse_paired(listing: &str) -> Vec<(String, String)> {
    listing
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ')?;
            valid_mac(mac).then(|| (mac.to_uppercase(), name.trim().chars().take(80).collect()))
        })
        .take(12)
        .collect()
}

/// Casques et enceintes : icône audio ou service « Audio Sink » (0000110b).
pub fn is_audio_device(info: &str) -> bool {
    let lower = info.to_lowercase();
    lower.contains("icon: audio-") || lower.contains("0000110b-")
}

pub fn sink_matches_device(sink_id: &str, mac: &str) -> bool {
    sink_id.to_uppercase().contains(&mac.replace(':', "_"))
}

pub fn bluetooth_devices() -> Vec<BluetoothDevice> {
    parse_paired(&sys::quiet("bluetoothctl", &["devices", "Paired"]))
        .into_iter()
        .filter_map(|(mac, name)| {
            let info = sys::quiet("bluetoothctl", &["info", &mac]);
            is_audio_device(&info).then(|| BluetoothDevice { id: format!("bt:{mac}"), connected: info.contains("Connected: yes"), mac, name })
        })
        .collect()
}

/// État complet : sorties actives et casques jumelés absents de la liste.
pub fn outputs(bluetooth: &[BluetoothDevice]) -> AudioState {
    let mut current = state();
    current.bluetooth = bluetooth.iter().filter(|d| !current.sinks.iter().any(|s| sink_matches_device(&s.id, &d.mac))).cloned().collect();
    current
}

fn connect_bluetooth(mac: &str) -> Result<String, String> {
    if !valid_mac(mac) {
        return Err("Appareil Bluetooth invalide".into());
    }
    let ok = sys::run("bluetoothctl", &["--timeout", "15", "connect", mac], Duration::from_secs(20)).map(|o| o.success()).unwrap_or(false);
    if !ok {
        return Err("Connexion impossible : allume l’appareil et rapproche-le du Raspberry Pi.".into());
    }
    // La sortie audio apparaît quelques secondes après la connexion.
    for _ in 0..20 {
        if let Some(sink) = state().sinks.into_iter().find(|s| sink_matches_device(&s.id, mac)) {
            return Ok(sink.id);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("L’appareil est connecté mais ne propose pas de sortie audio.".into())
}

/// Change la sortie par défaut et y déplace les flux en cours.
pub fn set_output(id: &str) -> Result<AudioState, String> {
    if id.len() > 300 {
        return Err("Sortie audio invalide".into());
    }
    let id = match id.strip_prefix("bt:") {
        Some(mac) => connect_bluetooth(&mac.to_uppercase())?,
        None => id.to_string(),
    };
    if !state().sinks.iter().any(|s| s.id == id) {
        return Err("Sortie audio indisponible".into());
    }
    sys::checked("pactl", &["set-default-sink", &id], Duration::from_secs(3))?;
    for input in sys::quiet("pactl", &["list", "short", "sink-inputs"]).lines().filter_map(|l| l.split_whitespace().next()) {
        let _ = sys::checked("pactl", &["move-sink-input", input, &id], Duration::from_secs(3));
    }
    Ok(state())
}

/// Une ligne de `pactl subscribe` qui justifie de relire l'état.
pub fn subscribe_line_matters(line: &str) -> bool {
    ["on sink ", "on server ", "on card "].iter().any(|k| line.contains(k))
}

/// Un son joue-t-il ? (`pactl list short sink-inputs` non vide)
pub fn something_playing() -> bool {
    !sys::quiet("pactl", &["list", "short", "sink-inputs"]).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sinks_are_classified() {
        let json = r#"[
          {"name":"alsa_output.platform-hdmi","description":"HDMI TV","mute":false,"volume":{"front-left":{"value_percent":"52%"}},"properties":{}},
          {"name":"bluez_output.AA_BB_CC_DD_EE_FF.1","description":"Casque","mute":true,"volume":{"mono":{"value_percent":"130%"}},"properties":{"device.bus":"bluetooth"}},
          {"name":"alsa_output.usb-dac","mute":false,"volume":{},"properties":{"device.bus":"usb","device.form_factor":"headset","node.nick":"DAC"}}
        ]"#;
        let sinks = parse_sinks(json, "bluez_output.AA_BB_CC_DD_EE_FF.1\n");
        assert_eq!((sinks[0].kind.as_str(), sinks[0].volume, sinks[0].active), ("hdmi", 52, false));
        assert_eq!((sinks[1].kind.as_str(), sinks[1].volume, sinks[1].active, sinks[1].muted), ("bluetooth", 100, true, true));
        assert_eq!((sinks[2].kind.as_str(), sinks[2].headset, sinks[2].name.as_str()), ("usb", true, "DAC"));
        assert!(sink_matches_device(&sinks[1].id, "AA:BB:CC:DD:EE:FF"));
    }

    #[test]
    fn bluetooth_listing() {
        let listing = "Device AA:BB:CC:DD:EE:FF Casque Salon\nDevice zz Mauvais\nAutre ligne";
        assert_eq!(parse_paired(listing), vec![("AA:BB:CC:DD:EE:FF".to_string(), "Casque Salon".to_string())]);
        assert!(is_audio_device("\tIcon: audio-headset\n"));
        assert!(!is_audio_device("\tIcon: input-gaming\n"));
        assert!(subscribe_line_matters("Event 'change' on sink #52"));
        assert!(!subscribe_line_matters("Event 'change' on source-output #1"));
    }
}
