//! USB device collector for macOS.
//!
//! Enumerates all connected USB devices and returns their full descriptor trees
//! using two complementary data sources:
//!
//! - **[`nusb`]** — wraps IOKit directly to enumerate devices and retrieve raw
//!   Device and Configuration descriptor bytes via `GET_DESCRIPTOR`. This gives
//!   us the complete descriptor hierarchy including endpoints and class-specific
//!   descriptors.
//!
//! - **`ioreg -a -c IOHIDInterface`** — HID report descriptor bytes live on
//!   `IOHIDInterface` nodes, not on the USB device node. A separate ioreg pass
//!   extracts them and correlates them to the nusb device via `LocationID`.
//!
//! The public API is [`MacCollector::enumerate`], which returns a
//! `Vec<`[`UsbDevice`]`>` or a [`CollectorError`].

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::process::Command;

use nusb::MaybeFuture;
use plist::Value;
use usb_types::*;

// ── Public API ─────────────────────────────────────────────────────────────────

pub struct MacCollector;

impl MacCollector {
    pub fn new() -> Self {
        MacCollector
    }

    pub fn enumerate(&self) -> Result<Vec<UsbDevice>, CollectorError> {
        // Pass 2 first: build location_id → HID report descriptors map from ioreg.
        let hid_map = collect_hid_descriptors()?;

        // Pass 1: nusb enumerates all devices, giving us topology + raw descriptor bytes.
        let mut result = Vec::new();
        for dev_info in nusb::list_devices().wait()? {
            let location_id = dev_info.location_id();
            let hid_interfaces = hid_map.get(&location_id).cloned().unwrap_or_default();

            match build_device(&dev_info, hid_interfaces) {
                Ok(device) => result.push(device),
                Err(e) => eprintln!(
                    "warning: skipped {:04x}:{:04x} @ {:#010x}: {}",
                    dev_info.vendor_id(),
                    dev_info.product_id(),
                    location_id,
                    e
                ),
            }
        }

        Ok(result)
    }
}

impl Default for MacCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ── Error type ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CollectorError {
    Io(std::io::Error),
    Plist(plist::Error),
    Nusb(nusb::Error),
    Other(String),
}

impl std::fmt::Display for CollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollectorError::Io(e) => write!(f, "io: {e}"),
            CollectorError::Plist(e) => write!(f, "plist: {e}"),
            CollectorError::Nusb(e) => write!(f, "nusb: {e}"),
            CollectorError::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for CollectorError {}

impl From<std::io::Error> for CollectorError {
    fn from(e: std::io::Error) -> Self {
        CollectorError::Io(e)
    }
}

impl From<plist::Error> for CollectorError {
    fn from(e: plist::Error) -> Self {
        CollectorError::Plist(e)
    }
}

impl From<nusb::Error> for CollectorError {
    fn from(e: nusb::Error) -> Self {
        CollectorError::Nusb(e)
    }
}

// ── Device builder ─────────────────────────────────────────────────────────────

fn build_device(
    dev_info: &nusb::DeviceInfo,
    hid_interfaces: Vec<HidInterface>,
) -> Result<UsbDevice, CollectorError> {
    let device = dev_info
        .open()
        .wait()
        .map_err(|e| CollectorError::Other(format!("open: {e}")))?;

    // Raw device descriptor (18 bytes) — build our type directly from nusb's parsed fields.
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

    // Configuration descriptors — raw bytes + parsed structure.
    let configurations: Vec<ConfigDescriptor> = device
        .configurations()
        .map(|cfg| {
            let raw_bytes = cfg.as_bytes().to_vec();
            let class_map = usb_types::extract_class_specific(&raw_bytes);
            let interface_associations = usb_types::extract_iads(&raw_bytes);
            let interfaces: Vec<InterfaceDescriptor> = cfg
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

    // Collect extra string indices (interface/config names) not available from OS cache.
    let mut extra_indices: Vec<u8> = Vec::new();
    for cfg in &configurations {
        if cfg.i_configuration > 0 { extra_indices.push(cfg.i_configuration); }
        for iface in &cfg.interfaces {
            if iface.i_interface > 0 { extra_indices.push(iface.i_interface); }
        }
    }
    let strings = build_strings(dev_info, &device, &device_descriptor, &extra_indices);

    let location_id = dev_info.location_id();

    Ok(UsbDevice {
        location_id: format!("{:08X}", location_id),
        bus_number: (location_id >> 24) as u8,
        port_path: dev_info.port_chain().to_vec(),
        speed: map_speed(dev_info.speed()),
        device_descriptor,
        configurations,
        strings,
        hid_interfaces,
        children: vec![],
    })
}

fn build_strings(
    dev_info: &nusb::DeviceInfo,
    device: &nusb::Device,
    dd: &DeviceDescriptor,
    extra_indices: &[u8],
) -> Vec<StringDescriptor> {
    use std::num::NonZeroU8;
    use std::time::Duration;

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

    // Fetch interface/config name strings via control transfer (English, 0x0409).
    let known: std::collections::HashSet<u8> = strings.iter().map(|s| s.index).collect();
    let mut deduped = extra_indices.to_vec();
    deduped.sort_unstable();
    deduped.dedup();
    let timeout = Duration::from_millis(200);
    for idx in deduped {
        if known.contains(&idx) { continue; }
        if let Some(nz) = NonZeroU8::new(idx) {
            if let Ok(s) = device.get_string_descriptor(nz, 0x0409, timeout).wait() {
                strings.push(StringDescriptor { index: idx, value: s });
            }
        }
    }

    strings
}

fn map_speed(speed: Option<nusb::Speed>) -> UsbSpeed {
    match speed {
        Some(nusb::Speed::Low) => UsbSpeed::Low,
        Some(nusb::Speed::Full) => UsbSpeed::Full,
        Some(nusb::Speed::High) => UsbSpeed::High,
        Some(nusb::Speed::Super) => UsbSpeed::Super,
        Some(nusb::Speed::SuperPlus) => UsbSpeed::SuperPlus,
        _ => UsbSpeed::Unknown,
    }
}

// ── ioreg HID pass ─────────────────────────────────────────────────────────────
// Returns: location_id → HID interfaces on that device.
// IOHIDInterface exposes LocationID (u32) matching the parent USB device's locationID.

fn collect_hid_descriptors() -> Result<HashMap<u32, Vec<HidInterface>>, CollectorError> {
    let out = Command::new("ioreg")
        .args(["-a", "-c", "IOHIDInterface", "-l", "-r", "-w", "0"])
        .output()?;

    let root: Value = plist::from_bytes(&out.stdout)?;
    let mut map: HashMap<u32, Vec<HidInterface>> = HashMap::new();
    walk_hid(&root, &mut map);
    Ok(map)
}

fn walk_hid(value: &Value, map: &mut HashMap<u32, Vec<HidInterface>>) {
    match value {
        Value::Dictionary(dict) => {
            let is_hid_interface = dict
                .get("IOObjectClass")
                .and_then(|v| v.as_string())
                == Some("IOHIDInterface");

            let is_usb = dict
                .get("Transport")
                .and_then(|v| v.as_string())
                == Some("USB");

            if is_hid_interface && is_usb {
                if let Some(location_id) = dict
                    .get("LocationID")
                    .and_then(|v| v.as_unsigned_integer())
                    .map(|n| n as u32)
                {
                    if let Some(Value::Data(bytes)) = dict.get("ReportDescriptor") {
                        let parsed = hid_parser::parse(bytes).ok();
                        map.entry(location_id)
                            .or_default()
                            .push(HidInterface {
                                interface_number: 0,
                                raw_report_descriptor: bytes.clone(),
                                parsed,
                            });
                    }
                }
            }

            if let Some(Value::Array(children)) = dict.get("IORegistryEntryChildren") {
                for child in children {
                    walk_hid(child, map);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                walk_hid(item, map);
            }
        }
        _ => {}
    }
}
