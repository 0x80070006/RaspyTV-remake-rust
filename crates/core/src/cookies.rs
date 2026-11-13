//! Import d'une session YouTube exportée d'un navigateur (format cookies.txt « Netscape »).
//!
//! Google refuse souvent la connexion dans un navigateur intégré : on reprend une
//! session ouverte dans le navigateur habituel. Seuls les cookies YouTube et
//! Google sont retenus ; le fichier est effacé après import.

#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub expires: Option<i64>,
    pub name: String,
    pub value: String,
    pub http_only: bool,
}

fn allowed_domain(domain: &str) -> bool {
    let host = domain.trim_start_matches('.').to_lowercase();
    let valid = !host.is_empty() && host.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    valid && ["youtube.com", "google.com"].iter().any(|base| host == *base || host.ends_with(&format!(".{base}")))
}

pub fn parse_cookies_txt(text: &str, now: i64) -> Vec<Cookie> {
    let mut cookies = Vec::new();
    for raw in text.lines() {
        let mut line = raw.trim();
        let mut http_only = false;
        if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            http_only = true;
            line = rest;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }
        let (domain, path, secure, expiry, name) = (fields[0], fields[2], fields[3], fields[4], fields[5]);
        if !allowed_domain(domain) || name.is_empty() {
            continue;
        }
        let expiry: i64 = expiry.parse().unwrap_or(0);
        if expiry > 0 && expiry < now {
            continue; // déjà expiré
        }
        cookies.push(Cookie {
            domain: domain.to_string(),
            path: if path.is_empty() { "/".into() } else { path.to_string() },
            secure: secure.eq_ignore_ascii_case("true"),
            expires: (expiry > 0).then_some(expiry),
            name: name.to_string(),
            value: fields[6..].join("\t"),
            http_only,
        });
    }
    cookies
}

impl Cookie {
    /// Les préfixes `__Host-` et `__Secure-` imposent `secure` (et `/` pour `__Host-`).
    pub fn normalized(&self) -> Cookie {
        let host_only = self.name.starts_with("__Host-");
        let mut cookie = self.clone();
        cookie.secure = self.secure || host_only || self.name.starts_with("__Secure-");
        if host_only {
            cookie.path = "/".into();
            cookie.domain = self.domain.trim_start_matches('.').to_string();
        }
        cookie
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_google_and_youtube() {
        let text = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t4000000000\tSID\tabc\n\
            #HttpOnly_.google.com\tTRUE\t/\tTRUE\t4000000000\t__Secure-1PSID\tv\tw\n\
            .evil.com\tTRUE\t/\tTRUE\t4000000000\tSID\tx\n\
            .youtube.com.evil.com\tTRUE\t/\tTRUE\t4000000000\tSID\tx\n\
            .youtube.com\tTRUE\t/\tTRUE\t10\tOLD\tx\n\
            www.youtube.com\tFALSE\t/a\tFALSE\t0\t__Host-X\ty\n";
        let cookies = parse_cookies_txt(text, 1_000);
        assert_eq!(cookies.len(), 3);
        assert!(cookies[1].http_only && cookies[1].value == "v\tw");
        let host = cookies[2].normalized();
        assert!(host.secure && host.path == "/" && host.expires.is_none());
    }
}
