//! Coffre de la session Jellyfin, chiffré (ChaCha20-Poly1305).
//!
//! La clé est dérivée d'un secret aléatoire (`vault.key`, 0600) et de
//! `/etc/machine-id` : le fichier copié seul, ou sur une autre machine, est
//! illisible. Comme le `safeStorage` d'Electron sans trousseau, cela ne protège
//! pas contre quelqu'un qui dispose déjà du compte.

use std::path::Path;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::jellyfin::Session;

const MAGIC: &[u8; 4] = b"RTV1";

fn machine_id() -> String {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"].iter().find_map(|file| std::fs::read_to_string(file).ok()).unwrap_or_default().trim().to_string()
}

fn key(key_file: &Path) -> Option<[u8; 32]> {
    let secret = match std::fs::read(key_file) {
        Ok(bytes) if bytes.len() == 32 => bytes,
        _ => {
            let mut fresh = vec![0u8; 32];
            rand::thread_rng().fill_bytes(&mut fresh);
            crate::fsutil::atomic_write(key_file, &fresh).ok()?;
            fresh
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(b"raspytv-vault-v1");
    hasher.update(&secret);
    hasher.update(machine_id().as_bytes());
    Some(hasher.finalize().into())
}

pub fn seal(session: &Session, key_file: &Path) -> Option<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key(key_file)?));
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let plain = serde_json::to_vec(session).ok()?;
    let sealed = cipher.encrypt(Nonce::from_slice(&nonce), plain.as_slice()).ok()?;
    Some([MAGIC.as_slice(), &nonce, &sealed].concat())
}

pub fn open(bytes: &[u8], key_file: &Path) -> Option<Session> {
    if bytes.len() < 4 + 12 + 16 || &bytes[..4] != MAGIC {
        return None;
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key(key_file)?));
    let plain = cipher.decrypt(Nonce::from_slice(&bytes[4..16]), &bytes[16..]).ok()?;
    let session: Session = serde_json::from_slice(&plain).ok()?;
    (!session.token.is_empty() && !session.user_id.is_empty()).then_some(session)
}

pub fn load(file: &Path, key_file: &Path) -> Option<Session> {
    open(&std::fs::read(file).ok()?, key_file)
}

pub fn save(session: &Session, file: &Path, key_file: &Path) -> bool {
    match seal(session, key_file) {
        Some(bytes) => crate::fsutil::atomic_write(file, &bytes).is_ok(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_tamper() {
        let dir = std::env::temp_dir().join(format!("raspytv-vault-{}", std::process::id()));
        let key_file = dir.join("vault.key");
        let session = Session { token: "jeton".into(), user_id: "abcd".into(), server_id: "s".into(), server_name: "J".into() };
        let mut sealed = seal(&session, &key_file).unwrap();
        assert!(!String::from_utf8_lossy(&sealed).contains("jeton"));
        assert_eq!(open(&sealed, &key_file), Some(session));
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert_eq!(open(&sealed, &key_file), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
