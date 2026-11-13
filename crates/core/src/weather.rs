//! Météo Open-Meteo, avec la dernière valeur connue en cas de coupure.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use crate::site::Site;

pub fn url(site: &Site) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min&timezone={}",
        site.weather_latitude,
        site.weather_longitude,
        url::form_urlencoded::byte_serialize(site.timezone.as_bytes()).collect::<String>()
    )
}

pub fn parse(data: &Value) -> Option<Value> {
    let round = |v: Option<&Value>| v.and_then(Value::as_f64).map(|x| x.round() as i64);
    Some(json!({
        "temperature": round(data.pointer("/current/temperature_2m"))?,
        "code": data.pointer("/current/weather_code").and_then(Value::as_i64).unwrap_or(0),
        "min": round(data.pointer("/daily/temperature_2m_min/0"))?,
        "max": round(data.pointer("/daily/temperature_2m_max/0"))?,
        "offline": false,
    }))
}

pub fn fetch(site: &Site, cache: &Path) -> Value {
    let agent = crate::http::agent(Duration::from_secs(8));
    if let Some(result) = crate::http::get_json(&agent, &url(site), &[]).as_ref().and_then(parse) {
        let _ = crate::fsutil::atomic_write(cache, result.to_string().as_bytes());
        return result;
    }
    match crate::fsutil::read_json(cache) {
        Some(mut cached) => {
            cached["offline"] = json!(true);
            cached
        }
        None => json!({"unavailable": true, "offline": true}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_meteo() {
        let data = json!({"current": {"temperature_2m": 11.6, "weather_code": 3}, "daily": {"temperature_2m_min": [4.4], "temperature_2m_max": [13.5]}});
        assert_eq!(parse(&data).unwrap(), json!({"temperature": 12, "code": 3, "min": 4, "max": 14, "offline": false}));
        assert!(parse(&json!({})).is_none());
        assert!(url(&Site::default()).contains("timezone=Europe%2FParis"));
    }
}
