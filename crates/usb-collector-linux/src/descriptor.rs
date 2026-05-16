//! Parse raw USB descriptor bytes as returned by `/sys/…/descriptors`.
//!
//! The sysfs `descriptors` file contains the device descriptor (18 bytes)
//! followed by the full binary blob for each configuration descriptor, exactly
//! as the device returns them on the bus.

use usb_types::*;

const TYPE_CONFIG: u8 = 0x02;
const TYPE_INTERFACE: u8 = 0x04;
const TYPE_ENDPOINT: u8 = 0x05;

pub fn parse_device_descriptor(raw: &[u8]) -> Option<DeviceDescriptor> {
    if raw.len() < 18 || raw[0] < 18 || raw[1] != 0x01 {
        return None;
    }
    Some(DeviceDescriptor {
        bcd_usb:            u16::from_le_bytes([raw[2],  raw[3]]),
        b_device_class:     raw[4],
        b_device_sub_class: raw[5],
        b_device_protocol:  raw[6],
        b_max_packet_size0: raw[7],
        id_vendor:          u16::from_le_bytes([raw[8],  raw[9]]),
        id_product:         u16::from_le_bytes([raw[10], raw[11]]),
        bcd_device:         u16::from_le_bytes([raw[12], raw[13]]),
        i_manufacturer:     raw[14],
        i_product:          raw[15],
        i_serial_number:    raw[16],
        b_num_configurations: raw[17],
        raw_bytes: raw[..18].to_vec(),
    })
}

/// Parse the configuration descriptors that follow the device descriptor.
pub fn parse_config_descriptors(raw: &[u8]) -> Vec<ConfigDescriptor> {
    let mut configs = Vec::new();
    let mut pos = 0;

    while pos + 4 <= raw.len() {
        let b_len  = raw[pos] as usize;
        let b_type = raw[pos + 1];

        if b_len < 2 || pos + b_len > raw.len() {
            break;
        }

        if b_type != TYPE_CONFIG || b_len < 9 {
            pos += b_len;
            continue;
        }

        let total = u16::from_le_bytes([raw[pos + 2], raw[pos + 3]]) as usize;
        let end   = (pos + total).min(raw.len());

        let raw_bytes = raw[pos..end].to_vec();
        let class_map = usb_types::extract_class_specific(&raw_bytes);
        let interfaces = parse_interface_descriptors(&raw[pos + 9..end], &class_map);

        configs.push(ConfigDescriptor {
            b_configuration_value: raw[pos + 5],
            i_configuration:       raw[pos + 6],
            bm_attributes:         raw[pos + 7],
            b_max_power:           raw[pos + 8],
            interfaces,
            raw_bytes,
        });

        pos += total.max(b_len);
    }

    configs
}

fn parse_interface_descriptors(
    raw: &[u8],
    class_map: &std::collections::HashMap<(u8, u8), Vec<usb_types::ClassSpecificDescriptor>>,
) -> Vec<InterfaceDescriptor> {
    let mut interfaces = Vec::new();
    let mut current: Option<InterfaceDescriptor> = None;
    let mut pos = 0;

    while pos + 2 <= raw.len() {
        let b_len  = raw[pos] as usize;
        let b_type = raw[pos + 1];

        if b_len < 2 || pos + b_len > raw.len() {
            break;
        }

        match b_type {
            TYPE_INTERFACE if b_len >= 9 => {
                if let Some(iface) = current.take() {
                    interfaces.push(iface);
                }
                let iface_num = raw[pos + 2];
                let alt_set   = raw[pos + 3];
                current = Some(InterfaceDescriptor {
                    b_interface_number:    iface_num,
                    b_alternate_setting:   alt_set,
                    b_interface_class:     raw[pos + 5],
                    b_interface_sub_class: raw[pos + 6],
                    b_interface_protocol:  raw[pos + 7],
                    i_interface:           raw[pos + 8],
                    endpoints: Vec::new(),
                    class_descriptors: class_map.get(&(iface_num, alt_set)).cloned().unwrap_or_default(),
                });
            }
            TYPE_ENDPOINT if b_len >= 7 => {
                if let Some(ref mut iface) = current {
                    iface.endpoints.push(EndpointDescriptor {
                        b_endpoint_address: raw[pos + 2],
                        bm_attributes:      raw[pos + 3],
                        w_max_packet_size:  u16::from_le_bytes([raw[pos + 4], raw[pos + 5]]),
                        b_interval:         raw[pos + 6],
                    });
                }
            }
            _ => {}
        }

        pos += b_len;
    }

    if let Some(iface) = current {
        interfaces.push(iface);
    }

    interfaces
}
