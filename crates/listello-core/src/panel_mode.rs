// New in the lifecycle work (D8, second amendment).
//
// A T2 MacBook's Touch Bar is a USB device (05ac:8302) with two configurations. Configuration 1
// is the T2 chip's own keys (special keys, F1-F12 with Fn); configuration 2 hands the panel to
// the kernel's `appletbdrm`, which is what listello draws on. In configuration 2 with nobody
// drawing, the bar is black. So the renderer selects 2 when it starts and 1 when it ends, and a
// machine with no listello running shows the T2's keys.
//
// systemd runs `listello-core --panel-mode 2` before the renderer and
// `listello-core --panel-mode after-stop` after it (`ExecStartPre=+`, `ExecStopPost=+`: the unit
// is sandboxed and cannot write to /sys). Apple Silicon has no such device and every one of
// these is a no-op there, which is also how the renderer knows it must stay up for good.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

const USB_DEVICES: &str = "/sys/bus/usb/devices";
const T2_VENDOR: &str = "05ac";
const T2_PRODUCT: &str = "8302";
/// How long to wait for the kernel to bring the panel's devices up after selecting 2.
const SETTLE: Duration = Duration::from_secs(15);

/// The renderer exits with this when the user asked for a reload; systemd starts it again, and
/// the panel stays in configuration 2 meanwhile so the T2's keys do not flash.
pub const RELOAD_EXIT: i32 = 75;

/// The Touch Bar's USB device under `usb_devices` (`/sys/bus/usb/devices`), if this is a T2.
fn find_panel(usb_devices: &Path) -> Option<PathBuf> {
    let read = |dir: &Path, name: &str| {
        fs::read_to_string(dir.join(name))
            .ok()
            .map(|s| s.trim().to_string())
    };
    fs::read_dir(usb_devices)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|dir| {
            read(dir, "idVendor").as_deref() == Some(T2_VENDOR)
                && read(dir, "idProduct").as_deref() == Some(T2_PRODUCT)
        })
}

/// Whether this machine has a T2 Touch Bar, i.e. whether the renderer may leave when it has no
/// agent to serve.
pub fn is_t2() -> bool {
    find_panel(Path::new(USB_DEVICES)).is_some()
}

fn current(panel: &Path) -> Option<u8> {
    fs::read_to_string(panel.join("bConfigurationValue"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Select `want` (1 or 2). Going from 1 to 2 passes through 0, as tiny-dfr's udev rule did:
/// the kernel rebinds the drivers on every change.
fn select(panel: &Path, want: u8) -> std::io::Result<()> {
    let file = panel.join("bConfigurationValue");
    match (current(panel), want) {
        (Some(have), want) if have == want => Ok(()),
        (_, 2) => {
            fs::write(&file, "0")?;
            fs::write(&file, "2")
        }
        (_, want) => fs::write(&file, want.to_string()),
    }
}

/// Whether the devices the renderer opens at start are there: the panel's DRM card, its
/// touchpad and its backlight.
fn panel_ready(sys: &Path, proc_devices: &Path) -> bool {
    let drm = fs::read_dir(sys.join("class/drm")).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            fs::read_link(e.path().join("device/driver"))
                .is_ok_and(|d| d.file_name().is_some_and(|n| n == "appletbdrm"))
        })
    });
    let backlight = sys.join("class/backlight/appletb_backlight").exists();
    let touch =
        fs::read_to_string(proc_devices).is_ok_and(|t| t.contains("Touch Bar Display Touchpad"));
    drm && backlight && touch
}

/// `--panel-mode 2`: give the panel to the renderer and wait until its devices exist.
/// Always exits 0 so that a slow kernel shows up as the renderer's own error, not as a unit
/// that never started.
pub fn start() {
    let Some(panel) = find_panel(Path::new(USB_DEVICES)) else {
        return;
    };
    if let Err(e) = select(&panel, 2) {
        eprintln!("listello: could not select the panel's configuration 2: {e}");
        return;
    }
    let begun = Instant::now();
    while begun.elapsed() < SETTLE {
        if panel_ready(Path::new("/sys"), Path::new("/proc/bus/input/devices")) {
            // The seat rules must have run on them before the renderer looks.
            let _ = Command::new("udevadm")
                .args(["settle", "--timeout=5"])
                .status();
            return;
        }
        sleep(Duration::from_millis(100));
    }
    eprintln!("listello: the panel's devices did not appear within {SETTLE:?}");
}

/// `--panel-mode after-stop`: hand the panel back to the T2, unless the renderer is only
/// restarting for a reload (systemd tells us how it ended).
pub fn after_stop() {
    let exited = std::env::var("EXIT_CODE").is_ok_and(|v| v == "exited");
    let reload = exited && std::env::var("EXIT_STATUS").is_ok_and(|v| v == RELOAD_EXIT.to_string());
    if reload {
        return;
    }
    let Some(panel) = find_panel(Path::new(USB_DEVICES)) else {
        return;
    };
    if let Err(e) = select(&panel, 1) {
        eprintln!("listello: could not hand the panel back to the T2: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_usb(root: &Path, name: &str, vendor: &str, product: &str, config: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("idVendor"), format!("{vendor}\n")).unwrap();
        fs::write(dir.join("idProduct"), format!("{product}\n")).unwrap();
        fs::write(dir.join("bConfigurationValue"), format!("{config}\n")).unwrap();
        dir
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("listello-panel-mode-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_panel_is_found_by_its_usb_id_and_other_devices_are_not() {
        let root = scratch("find");
        fake_usb(&root, "7-5", "05ac", "8600", "1");
        assert!(find_panel(&root).is_none());
        let panel = fake_usb(&root, "7-6", "05ac", "8302", "1");
        assert_eq!(find_panel(&root), Some(panel));
    }

    #[test]
    fn a_machine_without_the_t2_panel_has_none() {
        let root = scratch("none");
        fake_usb(&root, "1-1", "1d6b", "0002", "1");
        assert!(find_panel(&root).is_none());
        assert!(find_panel(&root.join("missing")).is_none());
    }

    #[test]
    fn selecting_a_configuration_writes_it_and_leaves_a_matching_one_alone() {
        let root = scratch("select");
        let panel = fake_usb(&root, "7-6", "05ac", "8302", "1");
        select(&panel, 2).unwrap();
        assert_eq!(current(&panel), Some(2));
        select(&panel, 1).unwrap();
        assert_eq!(current(&panel), Some(1));
        // Already there: nothing to do, and the file is untouched.
        fs::write(panel.join("bConfigurationValue"), "2\n").unwrap();
        select(&panel, 2).unwrap();
        assert_eq!(
            fs::read_to_string(panel.join("bConfigurationValue")).unwrap(),
            "2\n"
        );
    }

    #[test]
    fn the_panel_is_ready_only_when_display_touchpad_and_backlight_are_all_there() {
        let sys = scratch("ready");
        let devices = sys.join("devices");
        fs::write(
            &devices,
            "N: Name=\"Apple Inc. Touch Bar Display Touchpad\"\n",
        )
        .unwrap();
        fs::create_dir_all(sys.join("class/backlight/appletb_backlight")).unwrap();
        assert!(!panel_ready(&sys, &devices), "no display yet");

        let driver = sys.join("drivers/appletbdrm");
        fs::create_dir_all(&driver).unwrap();
        let card = sys.join("class/drm/card2/device");
        fs::create_dir_all(&card).unwrap();
        std::os::unix::fs::symlink(&driver, card.join("driver")).unwrap();
        assert!(panel_ready(&sys, &devices));

        fs::write(&devices, "N: Name=\"something else\"\n").unwrap();
        assert!(!panel_ready(&sys, &devices), "no touchpad");
    }
}
