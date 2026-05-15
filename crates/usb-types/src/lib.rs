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

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HidInterface {
    pub interface_number: u8,
    pub raw_report_descriptor: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<Vec<HidNode>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type")]
pub enum HidNode {
    UsagePage { page_id: u16, name: String },
    Usage { usage_id: u32, name: Option<String> },
    Collection { kind: CollectionKind, children: Vec<HidNode> },
    ReportId { id: u32 },
    LogicalMinimum { value: i32 },
    LogicalMaximum { value: i32 },
    PhysicalMinimum { value: i32 },
    PhysicalMaximum { value: i32 },
    UnitExponent { value: i32 },
    Unit { value: u32 },
    ReportSize { value: u32 },
    ReportCount { value: u32 },
    UsageMinimum { value: u32 },
    UsageMaximum { value: u32 },
    Input { flags: HidIoFlags },
    Output { flags: HidIoFlags },
    Feature { flags: HidIoFlags },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum CollectionKind {
    Physical,
    Application,
    Logical,
    Report,
    NamedArray,
    UsageSwitch,
    UsageModifier,
    Vendor { id: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HidIoFlags {
    pub constant: bool,        // bit 0: false=Data, true=Constant
    pub variable: bool,        // bit 1: false=Array, true=Variable
    pub relative: bool,        // bit 2: false=Absolute, true=Relative
    pub wrap: bool,            // bit 3: false=No Wrap, true=Wrap
    pub non_linear: bool,      // bit 4: false=Linear, true=Non Linear
    pub no_preferred: bool,    // bit 5: false=Preferred State, true=No Preferred
    pub null_state: bool,      // bit 6: false=No Null Position, true=Null State
    pub volatile: bool,        // bit 7: false=Nonvolatile, true=Volatile (Output/Feature)
    pub buffered_bytes: bool,  // bit 8: false=Bitfield, true=Buffered Bytes
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
