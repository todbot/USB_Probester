//! USB device collector for Linux.
//!
//! Enumerates all connected USB devices and returns their full descriptor trees
//! entirely from sysfs — no device open and no elevated privileges required:
//!
//! - **[`nusb`]** — provides device metadata: bus number, port chain, cached
//!   string descriptors (manufacturer, product, serial), and speed.
//!
//! - **`/sys/bus/usb/devices/<dev>/descriptors`** — world-readable binary file
//!   containing the raw Device Descriptor followed by the full Configuration
//!   Descriptor blob, parsed by the internal `descriptor` module.
//!
//! - **HID report descriptors** — located at
//!   `/sys/bus/usb/devices/<dev>/<dev>:<cfg>.<iface>/0003:<VID>:<PID>.<N>/report_descriptor`,
//!   collected by the internal `hid` module.
//!
//! The public API is [`LinuxCollector::enumerate`], which returns a
//! `Vec<`[`UsbDevice`]`>` or a [`CollectorError`].

#![cfg(target_os = "linux")]

mod descriptor;
mod hid;

use nusb::MaybeFuture;
use usb_types::*;

// ── Public API ──────────────────────────────────────────────────────────────────

pub struct LinuxCollector;

impl LinuxCollector {
    pub fn new() -> Self {
        LinuxCollector
    }

    pub fn enumerate(&self) -> Result<Vec<UsbDevice>, CollectorError> {
        let mut result = Vec::new();
        for dev_info in nusb::list_devices().wait()? {
            match build_device(&dev_info) {
                Ok(device) => result.push(device),
                Err(e) => eprintln!(
                    "warning: skipped {:04x}:{:04x}: {}",
                    dev_info.vendor_id(),
                    dev_info.product_id(),
                    e
                ),
            }
        }
        Ok(result)
    }
}

impl Default for LinuxCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ── Error type ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CollectorError {
    Io(std::io::Error),
    Nusb(nusb::Error),
    Other(String),
}

impl std::fmt::Display for CollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollectorError::Io(e)    => write!(f, "io: {e}"),
            CollectorError::Nusb(e)  => write!(f, "nusb: {e}"),
            CollectorError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for CollectorError {}

impl From<std::io::Error> for CollectorError {
    fn from(e: std::io::Error) -> Self { CollectorError::Io(e) }
}

impl From<nusb::Error> for CollectorError {
    fn from(e: nusb::Error) -> Self { CollectorError::Nusb(e) }
}

// ── Device builder ──────────────────────────────────────────────────────────────

fn build_device(dev_info: &nusb::DeviceInfo) -> Result<UsbDevice, CollectorError> {
    let sysfs = dev_info.sysfs_path();
    let busnum = dev_info.busnum();

    // Location ID: sysfs basename, e.g. "2-4" or "2-2.3"
    let location_id = sysfs
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| busnum.to_string());

    // Raw descriptor bytes from sysfs — no open() required.
    let raw = std::fs::read(sysfs.join("descriptors"))?;
    if raw.len() < 18 {
        return Err(CollectorError::Other("descriptor blob too short".into()));
    }

    let device_descriptor = descriptor::parse_device_descriptor(&raw)
        .ok_or_else(|| CollectorError::Other("invalid device descriptor".into()))?;

    let configurations  = descriptor::parse_config_descriptors(&raw[18..]);
    let strings         = build_strings(dev_info, &device_descriptor);
    let hid_interfaces  = hid::collect(sysfs);

    Ok(UsbDevice {
        location_id,
        bus_number: busnum,
        port_path: dev_info.port_chain().to_vec(),
        speed: map_speed(dev_info.speed()),
        device_descriptor,
        configurations,
        strings,
        hid_interfaces,
        children: vec![],
    })
}

fn build_strings(dev_info: &nusb::DeviceInfo, dd: &DeviceDescriptor) -> Vec<StringDescriptor> {
    let mut strings = Vec::new();
    if dd.i_manufacturer > 0 {
        if let Some(s) = dev_info.manufacturer_string() {
            strings.push(StringDescriptor { index: dd.i_manufacturer, value: s.to_string() });
        }
    }
    if dd.i_product > 0 {
        if let Some(s) = dev_info.product_string() {
            strings.push(StringDescriptor { index: dd.i_product, value: s.to_string() });
        }
    }
    if dd.i_serial_number > 0 {
        if let Some(s) = dev_info.serial_number() {
            strings.push(StringDescriptor { index: dd.i_serial_number, value: s.to_string() });
        }
    }
    strings
}

fn map_speed(speed: Option<nusb::Speed>) -> UsbSpeed {
    match speed {
        Some(nusb::Speed::Low)       => UsbSpeed::Low,
        Some(nusb::Speed::Full)      => UsbSpeed::Full,
        Some(nusb::Speed::High)      => UsbSpeed::High,
        Some(nusb::Speed::Super)     => UsbSpeed::Super,
        Some(nusb::Speed::SuperPlus) => UsbSpeed::SuperPlus,
        _                            => UsbSpeed::Unknown,
    }
}

// ── Public helpers (used by examples) ──────────────────────────────────────────

/// Parse a `UsbDevice` from raw sysfs descriptor bytes plus optional strings.
/// Useful for loading stored sysfs data without a live bus.
pub fn device_from_descriptor_bytes(
    raw: &[u8],
    location_id: &str,
    bus_number: u8,
    port_path: Vec<u8>,
    speed: UsbSpeed,
    strings: Vec<StringDescriptor>,
    hid_interfaces: Vec<HidInterface>,
) -> Option<UsbDevice> {
    if raw.len() < 18 { return None; }
    let device_descriptor = descriptor::parse_device_descriptor(raw)?;
    let configurations    = descriptor::parse_config_descriptors(&raw[18..]);
    Some(UsbDevice {
        location_id: location_id.to_string(),
        bus_number,
        port_path,
        speed,
        device_descriptor,
        configurations,
        strings,
        hid_interfaces,
        children: vec![],
    })
}
