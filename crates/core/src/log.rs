//! Journal `logs/raspy-tv.log`, tourné à 2 Mo (un fichier `.1` conservé).

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
const MAX_SIZE: u64 = 2 * 1024 * 1024;

pub fn init(dir: PathBuf) {
    crate::fsutil::ensure_private_dir(&dir);
    if let Ok(mut file) = LOG_FILE.lock() {
        *file = Some(dir.join("raspy-tv.log"));
    }
}

pub fn log(message: impl AsRef<str>) {
    let message: String = message.as_ref().chars().take(4000).collect();
    let line = format!("{} {}\n", jiff::Timestamp::now(), message);
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    let Ok(guard) = LOG_FILE.lock() else { return };
    let Some(file) = guard.as_ref() else { return };
    if std::fs::metadata(file).map(|m| m.len() > MAX_SIZE).unwrap_or(false) {
        let rotated = file.with_extension("log.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(file, rotated);
    }
    if let Ok(mut handle) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = handle.write_all(line.as_bytes());
        crate::fsutil::restrict(file);
    }
}

/// `rlog!("format {}", x)` : raccourci vers [`log`].
#[macro_export]
macro_rules! rlog {
    ($($arg:tt)*) => { $crate::log::log(format!($($arg)*)) };
}
