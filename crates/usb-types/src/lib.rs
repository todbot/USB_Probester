use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UsbDevice {
    pub location_id: String,
    pub bus_number: u8,
    pub port_path: Vec<u8>,
    pub speed: UsbSpeed,
    pub device_descriptor: DeviceDescriptor,
    pub configurations: Vec<ConfigDescriptor>,
    pub strings: Vec<StringDescriptor>,
    pub hid_interfaces: Vec<HidInterface>,
    pub children: Vec<UsbDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DeviceDescriptor {
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ConfigDescriptor {
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
    pub interfaces: Vec<InterfaceDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InterfaceDescriptor {
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
    pub endpoints: Vec<EndpointDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct EndpointDescriptor {
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct StringDescriptor {
    pub index: u8,
    pub value: String,
}

/// Raw HID interface — `parsed` field added when hid-parser crate lands (Step 3).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HidInterface {
    pub interface_number: u8,
    pub raw_report_descriptor: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum UsbSpeed {
    Low,       // 1.5 Mbit/s
    Full,      // 12 Mbit/s
    High,      // 480 Mbit/s
    Super,     // 5 Gbit/s
    SuperPlus, // 10 Gbit/s
    Unknown,
}
