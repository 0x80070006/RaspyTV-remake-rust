//! Lancement de commandes externes avec délai maximal.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;

#[derive(Debug, Clone)]
pub struct Output {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }
    /// Message d'erreur lisible : stderr, sinon le code de sortie.
    pub fn error(&self) -> String {
        let text = self.stderr.trim();
        if text.is_empty() {
            format!("code {:?}", self.status)
        } else {
            text.to_string()
        }
    }
}

/// Lance `program args…`, attend au plus `timeout`, tue le processus au-delà.
pub fn run_env(program: &str, args: &[&str], timeout: Duration, env: &[(&str, &str)]) -> Result<Output, String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().map_err(|e| format!("{program}: {e}"))?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    // Lectures dans des fils séparés : une sortie volumineuse ne bloque pas le processus.
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.take(8 * 1024 * 1024).read_to_string(&mut text);
        }
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.take(1024 * 1024).read_to_string(&mut text);
        }
        text
    });
    let status = match child.wait_timeout(timeout).map_err(|e| e.to_string())? {
        Some(status) => status.code(),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program}: délai dépassé"));
        }
    };
    Ok(Output { status, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
}

pub fn run(program: &str, args: &[&str], timeout: Duration) -> Result<Output, String> {
    run_env(program, args, timeout, &[])
}

/// Sortie standard si la commande réussit, sinon une erreur.
pub fn checked(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let output = run(program, args, timeout)?;
    if output.success() {
        Ok(output.stdout.trim().to_string())
    } else {
        Err(output.error())
    }
}

/// Sortie standard, même vide en cas d'échec (équivalent de `exec` de main.js).
pub fn quiet(program: &str, args: &[&str]) -> String {
    run(program, args, Duration::from_secs(3)).map(|o| o.stdout.trim().to_string()).unwrap_or_default()
}

/// Redémarrer / éteindre via une règle sudo explicite (`sudo -n`).
pub fn power(action: &str) -> Result<(), String> {
    if action != "reboot" && action != "poweroff" {
        return Err("Action invalide".into());
    }
    checked("sudo", &["-n", "systemctl", action], Duration::from_secs(5)).map(|_| ()).map_err(|error| {
        crate::rlog!("system power {action} failure {error}");
        let lower = error.to_lowercase();
        if ["password", "mot de passe", "not allowed", "sudoers"].iter().any(|w| lower.contains(w)) {
            "Droits administrateur requis : configure sudo pour cette action.".to_string()
        } else {
            "Action système impossible.".to_string()
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_program_is_an_error() {
        assert!(super::run("raspytv-programme-absent", &[], std::time::Duration::from_secs(1)).is_err());
        assert_eq!(super::quiet("raspytv-programme-absent", &[]), "");
    }

    #[test]
    fn power_rejects_unknown_actions() {
        assert!(super::power("halt").is_err());
    }
}
