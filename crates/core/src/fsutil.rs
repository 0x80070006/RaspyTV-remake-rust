//! Écritures atomiques et droits restreints (0600 / 0700) sous Unix.

use std::fs;
use std::io;
use std::path::Path;

pub fn ensure_private_dir(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
}

pub fn restrict(file: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(file, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = file;
}

/// Écrit dans un fichier temporaire puis le renomme : un arrêt brutal ne laisse
/// jamais un fichier à moitié écrit.
pub fn atomic_write(file: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = file.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, data)?;
    restrict(&temporary);
    fs::rename(&temporary, file)?;
    restrict(file);
    Ok(())
}

pub fn read_json(file: &Path) -> Option<serde_json::Value> {
    fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn atomic_write_replaces_content() {
        let dir = std::env::temp_dir().join(format!("raspytv-fs-{}", std::process::id()));
        let file = dir.join("a.json");
        super::atomic_write(&file, b"{\"a\":1}").unwrap();
        super::atomic_write(&file, b"{\"a\":2}").unwrap();
        assert_eq!(super::read_json(&file).unwrap()["a"], 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
