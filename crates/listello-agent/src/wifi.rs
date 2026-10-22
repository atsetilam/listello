//! Wifi state, read from `/sys/class/net` and `/proc/net/wireless`.
//!
//! The same trade as the battery (see `power.rs`): no NetworkManager, no session bus, plain
//! files. A symbol only needs "is there a wifi device, is it up, how strong" — the network
//! name would need nl80211 or NetworkManager, and M5's strip does not draw it, so `ssid` is
//! always `None` here.

use listello_protocol::NetworkState;
use std::fs;
use std::path::Path;

/// The wifi state, or `None` on a machine with no wireless interface.
///
/// `net_root` is `/sys/class/net`; `proc_wireless` is the text of `/proc/net/wireless`,
/// passed in so a test can supply it.
pub fn read_network(net_root: &Path, proc_wireless: &str) -> Option<NetworkState> {
    let mut names: Vec<_> = fs::read_dir(net_root)
        .ok()?
        .flatten()
        .filter(|e| e.path().join("wireless").is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let name = names.into_iter().next()?;
    let state = fs::read_to_string(net_root.join(&name).join("operstate")).unwrap_or_default();
    let connected = state.trim() == "up";
    Some(NetworkState {
        connected,
        signal: if connected {
            signal(proc_wireless, &name).unwrap_or(0.0)
        } else {
            0.0
        },
        ssid: None,
    })
}

/// Link quality for `interface` as `0.0..=1.0`. The driver reports it out of 70.
fn signal(proc_wireless: &str, interface: &str) -> Option<f64> {
    proc_wireless.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        if name.trim() != interface {
            return None;
        }
        // status, then link quality, level, noise …; the numbers carry a trailing dot.
        let link: f64 = rest
            .split_whitespace()
            .nth(1)?
            .trim_end_matches('.')
            .parse()
            .ok()?;
        link.is_finite().then(|| (link / 70.0).clamp(0.0, 1.0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::fixture::TempDir;

    const WIRELESS: &str =
        "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n \
         face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n \
         wlan0: 0000   35.  -52.  -256        0      0      0      0      0        0\n";

    fn interface(root: &TempDir, name: &str, operstate: &str, wireless: bool) {
        root.write(&format!("{name}/operstate"), operstate);
        if wireless {
            root.write(&format!("{name}/wireless/status"), "0\n");
        }
    }

    #[test]
    fn a_connected_wifi_reports_its_link_quality_out_of_seventy() {
        let root = TempDir::new("wifi");
        interface(&root, "lo", "unknown\n", false);
        interface(&root, "wlan0", "up\n", true);
        let state = read_network(root.path(), WIRELESS).expect("wifi");
        assert!(state.connected);
        assert_eq!(state.signal, 0.5);
        assert_eq!(state.ssid, None);
    }

    #[test]
    fn a_wifi_that_is_down_is_disconnected_with_no_signal() {
        let root = TempDir::new("down");
        interface(&root, "wlan0", "down\n", true);
        let state = read_network(root.path(), WIRELESS).expect("wifi");
        assert!(!state.connected);
        assert_eq!(state.signal, 0.0);
    }

    #[test]
    fn a_machine_with_only_ethernet_has_no_wifi_state() {
        let root = TempDir::new("eth");
        interface(&root, "enp4s0", "up\n", false);
        assert_eq!(read_network(root.path(), WIRELESS), None);
        assert_eq!(read_network(&root.path().join("missing"), WIRELESS), None);
    }

    #[test]
    fn signal_is_clamped_and_a_missing_or_garbled_line_reads_as_none() {
        let line = |link: &str| format!(" wlan0: 0000  {link}  -52.  -256 0 0 0 0 0 0\n");
        assert_eq!(signal(&line("200."), "wlan0"), Some(1.0));
        assert_eq!(signal(&line("-5."), "wlan0"), Some(0.0));
        assert_eq!(signal(&line("abc"), "wlan0"), None);
        assert_eq!(signal(WIRELESS, "wlan1"), None);
        assert_eq!(signal("", "wlan0"), None);
    }
}
