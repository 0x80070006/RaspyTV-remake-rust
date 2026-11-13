//! Client HTTP commun (ureq, rustls).

use std::time::Duration;

use ureq::Agent;

/// Agent avec délai global ; les codes 4xx/5xx restent des réponses (pas des erreurs).
pub fn agent(timeout: Duration) -> Agent {
    Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).user_agent(concat!("RaspyTV/", env!("CARGO_PKG_VERSION"))).build().into()
}

/// GET qui renvoie du JSON si le statut est 2xx.
pub fn get_json(agent: &Agent, url: &str, headers: &[(&str, &str)]) -> Option<serde_json::Value> {
    let mut request = agent.get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request.call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.body_mut().read_json().ok()
}

/// GET binaire limité en taille ; renvoie (type MIME, octets).
pub fn get_bytes(agent: &Agent, url: &str, headers: &[(&str, &str)], limit: u64) -> Option<(String, Vec<u8>)> {
    let mut request = agent.get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request.call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mime = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = response.body_mut().with_config().limit(limit).read_to_vec().ok()?;
    Some((mime, bytes))
}
