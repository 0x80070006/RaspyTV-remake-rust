//! Informations système pour l'onglet « Système ».

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub hostname: String,
    pub uptime: u64,
    pub cpu: String,
    pub ram_used: u64,
    pub ram_total: u64,
    pub temperature: Option<f64>,
    pub disk_free: Option<String>,
    pub disk_used_percent: Option<u8>,
    pub resident_apps: usize,
    pub resident_limit: u8,
    pub version: String,
    pub engine: String,
}

/// Lit `/sys/class/thermal/thermal_zone0/temp` (millidegrés).
pub fn parse_temperature(raw: &str) -> Option<f64> {
    raw.trim().parse::<f64>().ok().filter(|v| *v != 0.0).map(|v| v / 1000.0)
}

pub fn human_bytes(bytes: u64) -> String {
    let units = ["o", "Ko", "Mo", "Go", "To"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit >= 3 {
        format!("{value:.1} {}", units[unit])
    } else {
        format!("{value:.0} {}", units[unit])
    }
}

pub fn collect(resident_apps: usize, resident_limit: u8, engine: &str) -> Stats {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    system.refresh_cpu_all();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let root = disks.list().iter().find(|d| d.mount_point() == std::path::Path::new("/")).or_else(|| disks.list().first());
    let cpu = system.cpus().first().map(|c| c.brand().trim().to_string()).filter(|b| !b.is_empty());
    let cpu = cpu.or_else(|| std::fs::read_to_string("/proc/device-tree/model").ok().map(|m| m.trim_end_matches('\0').trim().to_string()));
    Stats {
        hostname: sysinfo::System::host_name().unwrap_or_default(),
        uptime: sysinfo::System::uptime(),
        cpu: cpu.unwrap_or_else(|| "Raspberry Pi".into()),
        ram_used: system.used_memory(),
        ram_total: system.total_memory(),
        temperature: std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").ok().as_deref().and_then(parse_temperature),
        disk_free: root.map(|d| human_bytes(d.available_space())),
        disk_used_percent: root.filter(|d| d.total_space() > 0).map(|d| (100 - d.available_space() * 100 / d.total_space()) as u8),
        resident_apps,
        resident_limit,
        version: crate::VERSION.into(),
        engine: engine.into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn temperature_and_sizes() {
        assert_eq!(super::parse_temperature("48312\n"), Some(48.312));
        assert_eq!(super::parse_temperature("0"), None);
        assert_eq!(super::human_bytes(41 * 1024 * 1024 * 1024), "41.0 Go");
        assert_eq!(super::human_bytes(512), "512 o");
    }
}
