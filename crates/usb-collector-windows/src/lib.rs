//! USB device collector for Windows.
//!
//! Enumerates connected USB devices via [`nusb`] and attempts to retrieve full
//! descriptor information for each:
//!
//! - Devices using the **WinUSB** driver can be opened directly; full Device and
//!   Configuration descriptor bytes are read and parsed.
//!
//! - Devices using Windows class drivers (**HID.sys**, **usbstor**, **CDC**,
//!   etc.) cannot be opened via nusb. These are returned with VID/PID, speed,
//!   and string descriptors only — interface and endpoint descriptors are absent.
//!
//! Known gaps (planned):
//! - Full descriptor access for class-driver devices requires
//!   `IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION` via hub handles.
//!
//! The public API is [`WindowsCollector::enumerate`], which returns a
//! `Vec<`[`UsbDevice`]`>` or a [`CollectorError`].

#![cfg(target_os = "windows")]

mod hid;

use std::collections::HashMap;
use nusb::MaybeFuture;
use usb_types::*;

// ── Public API ─────────────────────────────────────────────────────────────────

pub struct WindowsCollector;

impl WindowsCollector {
    pub fn new() -> Self {
        WindowsCollector
    }

    pub fn enumerate(&self) -> Result<Vec<UsbDevice>, CollectorError> {
        let hid_map = hid::collect_hid_descriptors();
        let mut result = Vec::new();
        for dev_info in nusb::list_devices().wait()? {
            match build_device(&dev_info, &hid_map) {
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

impl Default for WindowsCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ── Error type ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CollectorError {
    Nusb(nusb::Error),
    Other(String),
}

impl std::fmt::Display for CollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollectorError::Nusb(e) => write!(f, "nusb: {e}"),
            CollectorError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for CollectorError {}

impl From<nusb::Error> for CollectorError {
    fn from(e: nusb::Error) -> Self {
        CollectorError::Nusb(e)
    }
}

// ── Device builder ─────────────────────────────────────────────────────────────

fn build_device(
    dev_info: &nusb::DeviceInfo,
    hid_map: &HashMap<hid::HidKey, Vec<HidInterface>>,
) -> Result<UsbDevice, CollectorError> {
    let loc = make_location_id(dev_info);
    let key = (
        dev_info.vendor_id(),
        dev_info.product_id(),
        dev_info.serial_number().filter(|s| !s.is_empty()).map(str::to_owned),
    );
    let hid_interfaces = hid_map.get(&key).cloned().unwrap_or_default();
    eprintln!(
        "note: build_device {:04x}:{:04x} serial={:?} → hid_map hit={} hid_count={}",
        key.0, key.1, key.2,
        hid_map.contains_key(&key),
        hid_interfaces.len()
    );

    // Try to open the device for full descriptor access.
    // On Windows, only devices using WinUSB (not HID.sys, usbstor, etc.) can be opened
    // this way.  For everything else we fall back to the info available from enumeration.
    match dev_info.open().wait() {
        Ok(device) => build_from_open(dev_info, &device, loc, hid_interfaces),
        Err(e) => {
            eprintln!(
                "note: cannot open {:04x}:{:04x} ({e}), using partial info",
                dev_info.vendor_id(),
                dev_info.product_id()
            );
            Ok(build_from_info(dev_info, loc, hid_interfaces))
        }
    }
}

fn build_from_open(
    dev_info: &nusb::DeviceInfo,
    device: &nusb::Device,
    loc: String,
    hid_interfaces: Vec<HidInterface>,
) -> Result<UsbDevice, CollectorError> {
    let ndd = device.device_descriptor();

    let device_descriptor = DeviceDescriptor {
        bcd_usb: ndd.usb_version(),
        b_device_class: ndd.class(),
        b_device_sub_class: ndd.subclass(),
        b_device_protocol: ndd.protocol(),
        b_max_packet_size0: ndd.max_packet_size_0(),
        id_vendor: ndd.vendor_id(),
        id_product: ndd.product_id(),
        bcd_device: ndd.device_version(),
        i_manufacturer: ndd.manufacturer_string_index().map(|n| n.get()).unwrap_or(0),
        i_product: ndd.product_string_index().map(|n| n.get()).unwrap_or(0),
        i_serial_number: ndd.serial_number_string_index().map(|n| n.get()).unwrap_or(0),
        b_num_configurations: ndd.num_configurations(),
        raw_bytes: ndd.as_bytes().to_vec(),
    };

    let strings = build_strings(dev_info, &device_descriptor);

    let configurations: Vec<ConfigDescriptor> = device
        .configurations()
        .map(|cfg| {
            let raw_bytes = cfg.as_bytes().to_vec();
            let class_map = usb_types::extract_class_specific(&raw_bytes);
            let interface_associations = usb_types::extract_iads(&raw_bytes);
            let interfaces = cfg
                .interfaces()
                .flat_map(|igroup| {
                    igroup.alt_settings().map(|iface| {
                        let key = (iface.interface_number(), iface.alternate_setting());
                        InterfaceDescriptor {
                            b_interface_number: iface.interface_number(),
                            b_alternate_setting: iface.alternate_setting(),
                            b_interface_class: iface.class(),
                            b_interface_sub_class: iface.subclass(),
                            b_interface_protocol: iface.protocol(),
                            i_interface: iface.string_index().map(|n| n.get()).unwrap_or(0),
                            endpoints: iface
                                .endpoints()
                                .map(|ep| EndpointDescriptor {
                                    b_endpoint_address: ep.address(),
                                    bm_attributes: ep.attributes(),
                                    w_max_packet_size: ep.max_packet_size_raw(),
                                    b_interval: ep.as_bytes().get(6).copied().unwrap_or(0),
                                })
                                .collect(),
                            class_descriptors: class_map.get(&key).cloned().unwrap_or_default(),
                        }
                    })
                    .collect::<Vec<_>>()
                })
                .collect();

            ConfigDescriptor {
                b_configuration_value: cfg.configuration_value(),
                i_configuration: cfg.string_index().map(|n| n.get()).unwrap_or(0),
                bm_attributes: cfg.attributes(),
                b_max_power: cfg.max_power(),
                interfaces,
                interface_associations,
                raw_bytes,
            }
        })
        .collect();

    // For devices that nusb can open (WinUSB / virtual), fetch HID report descriptors
    // via control transfer — more reliable than hub IOCTL for these devices.
    // The hub-IOCTL pass (hid.rs) already handled real HID.sys devices; only use
    // this path to fill in interfaces where that pass returned empty bytes.
    let hid_interfaces = fill_hid_from_device(device, &configurations, hid_interfaces);

    Ok(UsbDevice {
        location_id: loc,
        bus_number: 0,
        port_path: dev_info.port_chain().to_vec(),
        speed: map_speed(dev_info.speed()),
        device_descriptor,
        configurations,
        strings,
        hid_interfaces,
        children: vec![],
    })
}

/// For devices that nusb can open, supplement the hid_map entries (which may have
/// empty `raw_report_descriptor` if the hub IOCTL path failed) by fetching the
/// report descriptor via a standard control transfer on the open device handle.
/// Entries already populated by hid.rs (non-empty bytes) are kept as-is.
fn fill_hid_from_device(
    device: &nusb::Device,
    configurations: &[ConfigDescriptor],
    mut existing: Vec<HidInterface>,
) -> Vec<HidInterface> {
    use nusb::transfer::{ControlIn, ControlType, Recipient};
    use std::time::Duration;

    // Build set of interface numbers that already have descriptor bytes.
    let have: std::collections::HashSet<u8> = existing
        .iter()
        .filter(|h| !h.raw_report_descriptor.is_empty())
        .map(|h| h.interface_number)
        .collect();

    let timeout = Duration::from_millis(500);

    for cfg in configurations {
        for iface in &cfg.interfaces {
            if iface.b_interface_class != 3 {
                continue; // not HID
            }
            let inum = iface.b_interface_number;
            if have.contains(&inum) {
                continue;
            }

            // data[5..7] = wDescriptorLength for HID class descriptor (type 0x21).
            let report_len = iface
                .class_descriptors
                .iter()
                .find(|d| d.descriptor_type == 0x21)
                .filter(|d| d.data.len() >= 7)
                .map(|d| u16::from_le_bytes([d.data[5], d.data[6]]))
                .unwrap_or(0);
            if report_len == 0 {
                continue;
            }

            // Claim the interface to issue control transfers on Windows (WinUSB requirement).
            // This fails if another driver (HID.sys) owns the interface — skip silently.
            let iface_handle = match device.claim_interface(inum).wait() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("note: fill_hid: claim_interface({inum}) failed: {e}");
                    continue;
                }
            };

            // Standard GET_DESCRIPTOR, recipient=Interface.
            // On Windows, index must equal the interface number (WinUSB limitation).
            let raw = iface_handle
                .control_in(
                    ControlIn {
                        control_type: ControlType::Standard,
                        recipient: Recipient::Interface,
                        request: 0x06,
                        value: 0x2200,
                        index: inum as u16,
                        length: report_len,
                    },
                    timeout,
                )
                .wait()
                .unwrap_or_default();

            let parsed = if raw.is_empty() { None } else { hid_parser::parse(&raw).ok() };

            // Update an existing (empty) entry or push a new one.
            if let Some(entry) = existing.iter_mut().find(|h| h.interface_number == inum) {
                entry.raw_report_descriptor = raw;
                entry.parsed = parsed;
            } else {
                existing.push(HidInterface { interface_number: inum, raw_report_descriptor: raw, parsed });
            }
        }
    }

    existing
}

// Fallback for devices whose class driver blocks nusb from opening them (HID,
// mass storage, CDC, etc.).  We can still show VID/PID, speed, and strings.
fn build_from_info(dev_info: &nusb::DeviceInfo, loc: String, hid_interfaces: Vec<HidInterface>) -> UsbDevice {
    // Use synthetic string indices 1/2/3 so the strings vec lines up correctly.
    let i_manufacturer = if dev_info.manufacturer_string().is_some() { 1 } else { 0 };
    let i_product      = if dev_info.product_string().is_some()      { 2 } else { 0 };
    let i_serial       = if dev_info.serial_number().is_some()       { 3 } else { 0 };

    let device_descriptor = DeviceDescriptor {
        bcd_usb:             0x0200,
        b_device_class:      0,
        b_device_sub_class:  0,
        b_device_protocol:   0,
        b_max_packet_size0:  64,
        id_vendor:           dev_info.vendor_id(),
        id_product:          dev_info.product_id(),
        bcd_device:          0,
        i_manufacturer,
        i_product,
        i_serial_number:     i_serial,
        b_num_configurations: 0,
        raw_bytes:           vec![],
    };

    let strings = build_strings(dev_info, &device_descriptor);

    UsbDevice {
        location_id: loc,
        bus_number: 0,
        port_path: dev_info.port_chain().to_vec(),
        speed: map_speed(dev_info.speed()),
        device_descriptor,
        configurations: vec![],
        strings,
        hid_interfaces,
        children: vec![],
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn make_location_id(dev_info: &nusb::DeviceInfo) -> String {
    let vid = dev_info.vendor_id();
    let pid = dev_info.product_id();
    // Prefer serial number; fall back to port chain for uniqueness.
    if let Some(serial) = dev_info.serial_number().filter(|s| !s.is_empty()) {
        return format!("{:04x}:{:04x}:{}", vid, pid, serial);
    }
    let port: Vec<String> = dev_info.port_chain().iter().map(|p| p.to_string()).collect();
    format!("{:04x}:{:04x}:{}", vid, pid, port.join("."))
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
