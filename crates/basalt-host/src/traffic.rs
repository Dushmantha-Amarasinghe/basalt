//! What each device has moved.
//!
//! Cumulative counters only. The host deliberately does **not** compute rates:
//! a rate is a measurement over an interval, and whoever reads these counters
//! knows how long it has been since they last looked, while this module does
//! not. Working one out here and shipping it as a number would be guessing at
//! the interval — which is exactly the mistake that once had the client
//! reporting 35 MB/s over a 22.7 MB/s link.

use std::collections::HashMap;
use std::sync::Mutex;

/// Everything one device has moved since the host started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceTraffic {
    pub sent: u64,
    pub received: u64,
    /// Connections this device currently has open.
    pub connections: u32,
}

impl DeviceTraffic {
    pub fn total(&self) -> u64 {
        self.sent + self.received
    }
}

/// Per-device counters, keyed by the device's token hash.
///
/// Keyed by the hash rather than the name because two machines can share a
/// name, and because the hash is what identifies a device everywhere else.
#[derive(Debug, Default)]
pub struct Traffic {
    devices: Mutex<HashMap<String, DeviceTraffic>>,
}

impl Traffic {
    /// Bytes the host sent to a device.
    pub fn sent(&self, device: &str, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.devices
            .lock()
            .expect("traffic lock")
            .entry(device.to_string())
            .or_default()
            .sent += bytes;
    }

    /// Bytes the host received from a device.
    pub fn received(&self, device: &str, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.devices
            .lock()
            .expect("traffic lock")
            .entry(device.to_string())
            .or_default()
            .received += bytes;
    }

    pub fn connected(&self, device: &str) {
        self.devices
            .lock()
            .expect("traffic lock")
            .entry(device.to_string())
            .or_default()
            .connections += 1;
    }

    pub fn disconnected(&self, device: &str) {
        if let Some(entry) = self.devices.lock().expect("traffic lock").get_mut(device) {
            // Saturating, so a disconnect the host somehow sees twice cannot
            // wrap the count to four billion open connections.
            entry.connections = entry.connections.saturating_sub(1);
        }
    }

    pub fn of(&self, device: &str) -> DeviceTraffic {
        self.devices
            .lock()
            .expect("traffic lock")
            .get(device)
            .copied()
            .unwrap_or_default()
    }

    pub fn snapshot(&self) -> HashMap<String, DeviceTraffic> {
        self.devices.lock().expect("traffic lock").clone()
    }

    /// Drops a device's counters, for when it is removed.
    pub fn forget(&self, device: &str) {
        self.devices.lock().expect("traffic lock").remove(device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_nobody_has_heard_of_has_moved_nothing() {
        let traffic = Traffic::default();
        assert_eq!(traffic.of("unknown"), DeviceTraffic::default());
        assert_eq!(traffic.of("unknown").total(), 0);
    }

    #[test]
    fn bytes_accumulate_in_both_directions() {
        let traffic = Traffic::default();
        traffic.sent("aa", 1000);
        traffic.sent("aa", 500);
        traffic.received("aa", 250);

        let counts = traffic.of("aa");
        assert_eq!(counts.sent, 1500);
        assert_eq!(counts.received, 250);
        assert_eq!(counts.total(), 1750);
    }

    #[test]
    fn devices_are_counted_separately() {
        let traffic = Traffic::default();
        traffic.sent("aa", 100);
        traffic.sent("bb", 900);

        assert_eq!(traffic.of("aa").sent, 100);
        assert_eq!(traffic.of("bb").sent, 900);
        assert_eq!(traffic.snapshot().len(), 2);
    }

    #[test]
    fn zero_byte_reports_do_not_create_an_entry() {
        // Every ping and every empty response would otherwise put a device in
        // the list before it had done anything.
        let traffic = Traffic::default();
        traffic.sent("aa", 0);
        traffic.received("aa", 0);
        assert!(traffic.snapshot().is_empty());
    }

    #[test]
    fn connections_are_counted_up_and_down() {
        let traffic = Traffic::default();
        traffic.connected("aa");
        traffic.connected("aa");
        assert_eq!(traffic.of("aa").connections, 2);

        traffic.disconnected("aa");
        assert_eq!(traffic.of("aa").connections, 1);
    }

    // The client keeps a pool, so connections open and close constantly. One
    // miscounted close must not read as four billion open.
    #[test]
    fn disconnecting_more_than_connecting_cannot_wrap() {
        let traffic = Traffic::default();
        traffic.connected("aa");
        traffic.disconnected("aa");
        traffic.disconnected("aa");
        traffic.disconnected("aa");
        assert_eq!(traffic.of("aa").connections, 0);
    }

    #[test]
    fn disconnecting_something_unknown_is_harmless() {
        let traffic = Traffic::default();
        traffic.disconnected("never-seen");
        assert!(traffic.snapshot().is_empty());
    }

    #[test]
    fn a_removed_device_takes_its_counters_with_it() {
        let traffic = Traffic::default();
        traffic.sent("aa", 5000);
        traffic.forget("aa");
        assert_eq!(traffic.of("aa").total(), 0);
        assert!(traffic.snapshot().is_empty());
    }

    #[test]
    fn the_snapshot_is_a_copy_rather_than_a_view() {
        let traffic = Traffic::default();
        traffic.sent("aa", 100);
        let before = traffic.snapshot();
        traffic.sent("aa", 900);

        assert_eq!(before["aa"].sent, 100, "a snapshot must not change later");
        assert_eq!(traffic.of("aa").sent, 1000);
    }
}
