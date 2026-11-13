//! Boucle kiosque : relance `raspytv` après un plantage, s'arrête sur une sortie propre.
//!
//! Équivalent de `start-kiosk.sh` : un arrêt demandé (code 0) termine la boucle,
//! tout autre code relance l'interface deux secondes plus tard.

use std::process::Command;
use std::time::Duration;

fn main() {
    let paths = raspytv_core::paths::Paths::from_env();
    paths.ensure();
    raspytv_core::log::init(paths.logs());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(if cfg!(windows) { "raspytv.exe" } else { "raspytv" })))
        .unwrap_or_else(|| "raspytv".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    loop {
        let status = Command::new(&exe).args(&args).status();
        match status {
            Ok(status) if status.success() => std::process::exit(0),
            Ok(status) => raspytv_core::rlog!("RaspyTV crashed with status {status}; restarting in 2s"),
            Err(error) => raspytv_core::rlog!("RaspyTV launch failure {error}; retrying in 2s"),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
