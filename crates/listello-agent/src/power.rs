//! The battery, read from `/sys/class/power_supply`.
//!
//! sysfs rather than UPower: the kernel already publishes everything the widget draws (charge,
//! charging, an estimate), it needs no session bus and no new dependency, and the files are
//! plain text a test can fake with a directory. The cost is that nothing tells us when they
//! change, so the agent re-reads on a slow tick and only reports a difference.

use listello_protocol::BatteryState;
use std::fs;
use std::path::Path;

/// The system's battery, or `None` on a machine without one. When there are several the first
/// by name wins: M5's strip draws one bar.
///
/// `charging` means "on external power", not "the kernel says Charging": a MacBook held at
/// its charge limit reports `Discharging` with the adapter plugged in, and a bolt that never
/// appears while the cable is in looks broken.
pub fn read_battery(root: &Path) -> Option<BatteryState> {
    let mut entries: Vec<_> = fs::read_dir(root).ok()?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    let dirs: Vec<_> = entries.iter().map(|e| e.path()).collect();
    let plugged_in = dirs.iter().any(|dir| {
        text(dir, "type").as_deref() == Some("Mains") && number(dir, "online") == Some(1.0)
    });
    let mut battery = dirs
        .iter()
        .find(|dir| text(dir, "type").as_deref() == Some("Battery") && !is_device(dir))
        .and_then(|dir| parse(dir))?;
    battery.charging |= plugged_in;
    Some(battery)
}

/// Wireless mice, keyboards and the like show up as `Battery` too, with `scope = Device`.
fn is_device(dir: &Path) -> bool {
    text(dir, "scope").as_deref() == Some("Device")
}

fn parse(dir: &Path) -> Option<BatteryState> {
    // Relative to what the battery holds when full today, as KDE shows it. The kernel's
    // `capacity` is against the design figure on some machines, so an aged battery that is
    // full reads 76%.
    let percent = relative_charge(dir).or_else(|| number(dir, "capacity"))?;
    if !(0.0..=100.0).contains(&percent) {
        return None;
    }
    let status = text(dir, "status").unwrap_or_default();
    let charging = status == "Charging";
    Some(BatteryState {
        charge: percent / 100.0,
        charging,
        time_remaining_secs: time_remaining(dir, &status),
    })
}

/// Charge as a percentage of the current full capacity, from whichever pair of files the
/// driver has. `None` when it has neither or the numbers make no sense.
fn relative_charge(dir: &Path) -> Option<f64> {
    let (now, full) = match (number(dir, "energy_now"), number(dir, "energy_full")) {
        (Some(now), Some(full)) => (now, full),
        _ => (number(dir, "charge_now")?, number(dir, "charge_full")?),
    };
    (full > 0.0 && now >= 0.0).then(|| (now / full * 100.0).min(100.0))
}

/// Seconds until empty (discharging) or full (charging), from whichever pair of numbers the
/// driver provides: energy and power, or charge and current. `None` when the driver gives
/// neither, when nothing is flowing, or for any other status — an estimate is only worth
/// showing when it is believable.
fn time_remaining(dir: &Path, status: &str) -> Option<u64> {
    let (now, full, rate) = match (number(dir, "energy_now"), number(dir, "power_now")) {
        (Some(now), Some(rate)) => (now, number(dir, "energy_full"), rate),
        _ => (
            number(dir, "charge_now")?,
            number(dir, "charge_full"),
            number(dir, "current_now")?,
        ),
    };
    if rate <= 0.0 {
        return None;
    }
    let hours = match status {
        "Discharging" => now / rate,
        "Charging" => (full? - now).max(0.0) / rate,
        _ => return None,
    };
    let secs = hours * 3600.0;
    // More than a day is a driver reporting a trickle, not an estimate.
    (secs.is_finite() && secs < 24.0 * 3600.0).then_some(secs as u64)
}

fn text(dir: &Path, name: &str) -> Option<String> {
    Some(fs::read_to_string(dir.join(name)).ok()?.trim().to_string())
}

fn number(dir: &Path, name: &str) -> Option<f64> {
    text(dir, name)?.parse().ok()
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh empty directory under the system temp dir, removed on drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(label: &str) -> TempDir {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "listello-agent-test-{}-{label}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }

        pub fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::TempDir;
    use super::*;

    fn supply(root: &TempDir, name: &str, files: &[(&str, &str)]) {
        for (file, contents) in files {
            root.write(&format!("{name}/{file}"), contents);
        }
    }

    #[test]
    fn a_discharging_laptop_battery_reports_charge_and_an_estimate() {
        let root = TempDir::new("bat");
        supply(&root, "ADP1", &[("type", "Mains\n")]);
        supply(
            &root,
            "BAT0",
            &[
                ("type", "Battery\n"),
                ("capacity", "62\n"),
                ("status", "Discharging\n"),
                ("energy_now", "31000000\n"),
                ("energy_full", "50000000\n"),
                ("power_now", "10000000\n"),
            ],
        );
        let battery = read_battery(root.path()).expect("a battery");
        assert_eq!(battery.charge, 0.62);
        assert!(!battery.charging);
        // 31 Wh at 10 W.
        assert_eq!(battery.time_remaining_secs, Some(11160));
    }

    #[test]
    fn charging_estimates_the_time_to_full_from_charge_and_current() {
        let root = TempDir::new("chg");
        supply(
            &root,
            "BAT0",
            &[
                ("type", "Battery\n"),
                ("capacity", "50\n"),
                ("status", "Charging\n"),
                ("charge_now", "2000000\n"),
                ("charge_full", "4000000\n"),
                ("current_now", "1000000\n"),
            ],
        );
        let battery = read_battery(root.path()).expect("a battery");
        assert!(battery.charging);
        assert_eq!(battery.time_remaining_secs, Some(2 * 3600));
    }

    #[test]
    fn no_flow_or_a_full_battery_has_no_estimate() {
        let root = TempDir::new("full");
        supply(
            &root,
            "BAT0",
            &[
                ("type", "Battery\n"),
                ("capacity", "100\n"),
                ("status", "Full\n"),
                ("energy_now", "50000000\n"),
                ("energy_full", "50000000\n"),
                ("power_now", "0\n"),
            ],
        );
        let battery = read_battery(root.path()).expect("a battery");
        assert_eq!(battery.charge, 1.0);
        assert!(!battery.charging);
        assert_eq!(battery.time_remaining_secs, None);
    }

    #[test]
    fn a_desktop_has_no_battery() {
        let root = TempDir::new("desktop");
        supply(&root, "AC", &[("type", "Mains\n")]);
        assert_eq!(read_battery(root.path()), None);
        assert_eq!(read_battery(&root.path().join("missing")), None);
    }

    #[test]
    fn a_wireless_peripheral_is_not_the_system_battery() {
        let root = TempDir::new("mouse");
        supply(
            &root,
            "hidpp_battery_0",
            &[
                ("type", "Battery\n"),
                ("scope", "Device\n"),
                ("capacity", "30\n"),
            ],
        );
        assert_eq!(read_battery(root.path()), None);
    }

    #[test]
    fn nonsense_in_the_files_is_a_missing_battery_not_a_panic() {
        let root = TempDir::new("junk");
        supply(
            &root,
            "BAT0",
            &[("type", "Battery\n"), ("capacity", "lots\n")],
        );
        assert_eq!(read_battery(root.path()), None);
        supply(&root, "BAT0", &[("capacity", "400\n")]);
        assert_eq!(read_battery(root.path()), None);
    }

    #[test]
    fn charge_is_relative_to_the_full_capacity_not_the_design_figure() {
        let root = TempDir::new("aged");
        supply(
            &root,
            "BAT0",
            &[
                ("type", "Battery\n"),
                ("capacity", "76\n"),
                ("status", "Discharging\n"),
                ("charge_now", "7500000\n"),
                ("charge_full", "7800000\n"),
            ],
        );
        let charge = read_battery(root.path()).expect("a battery").charge;
        assert!((charge - 7500.0 / 7800.0).abs() < 1e-9, "{charge}");
    }

    #[test]
    fn a_battery_held_at_its_limit_with_the_adapter_in_counts_as_plugged_in() {
        let root = TempDir::new("held");
        supply(&root, "ADP1", &[("type", "Mains\n"), ("online", "1\n")]);
        supply(
            &root,
            "BAT0",
            &[
                ("type", "Battery\n"),
                ("capacity", "80\n"),
                ("status", "Discharging\n"),
            ],
        );
        assert!(read_battery(root.path()).expect("a battery").charging);
        supply(&root, "ADP1", &[("online", "0\n")]);
        assert!(!read_battery(root.path()).expect("a battery").charging);
    }
}
