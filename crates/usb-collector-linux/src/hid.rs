//! Collect HID report descriptors from sysfs.
//!
//! On Linux, each USB interface lives at:
//!   `/sys/bus/usb/devices/<dev>/<dev>:<config>.<iface>/`
//!
//! If that interface is bound to the HID driver, a sub-directory named
//! `0003:<VID>:<PID>.<N>/` (HID bus = 0003) will be present, and inside it
//! a readable binary file called `report_descriptor`.

use std::fs;
use std::path::Path;
use usb_types::HidInterface;

pub fn collect(dev_sysfs: &Path) -> Vec<HidInterface> {
    let mut result = Vec::new();

    let dev_name = match dev_sysfs.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None    => return result,
    };
    let iface_prefix = format!("{}:", dev_name);

    let Ok(dir) = fs::read_dir(dev_sysfs) else { return result };

    for entry in dir.flatten() {
        let fname = entry.file_name().to_string_lossy().into_owned();

        // Interface dirs: "<dev_name>:<config>.<iface_num>"
        if !fname.starts_with(&iface_prefix) {
            continue;
        }

        // iface number is the part after the last '.'
        let iface_num: u8 = fname
            .rsplit('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // HID sub-directories have the form "0003:<VID>:<PID>.<N>"
        let iface_path = entry.path();
        let Ok(subd) = fs::read_dir(&iface_path) else { continue };

        for sub in subd.flatten() {
            if !sub.file_name().to_string_lossy().starts_with("0003:") {
                continue;
            }
            let rd_path = sub.path().join("report_descriptor");
            if let Ok(bytes) = fs::read(&rd_path) {
                let parsed = hid_parser::parse(&bytes).ok();
                result.push(HidInterface {
                    interface_number: iface_num,
                    raw_report_descriptor: bytes,
                    parsed,
                });
            }
        }
    }

    result
}
