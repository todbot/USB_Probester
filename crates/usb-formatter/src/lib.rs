use std::fmt::Write;
use hid_parser::render_text;
use usb_types::{self, usb_class, *};

pub fn format_devices(devices: &[UsbDevice]) -> String {
    let mut out = String::new();
    for d in devices {
        format_device(d, &mut out);
    }
    out
}

// ── Device ────────────────────────────────────────────────────────────────────

fn format_device(d: &UsbDevice, out: &mut String) {
    let dd = &d.device_descriptor;
    let product = string_for(d, dd.i_product).unwrap_or_default();
    let class_hdr = device_class_header(dd.b_device_class);
    let addr = d.port_path.last().copied().unwrap_or(1);
    let loc = u32::from_str_radix(&d.location_id, 16).unwrap_or(0);
    let speed = speed_label(&d.speed);

    let _ = writeln!(out,
        "{} device @ {} (0x{:08x}): .............................................   {}: \"{}\"",
        speed, addr, loc, class_hdr, product
    );
    let _ = writeln!(out, "    Port Information:   0x0018");
    let _ = writeln!(out, "           Not Captive");
    let _ = writeln!(out, "           External Device");
    let _ = writeln!(out, "           Connected");
    let _ = writeln!(out, "           Enabled");

    if let Some(cfg) = d.configurations.first() {
        let ep_count: usize = cfg.interfaces.iter()
            .filter(|i| i.b_alternate_setting == 0)
            .map(|i| i.endpoints.len())
            .sum::<usize>() + 1;
        let _ = writeln!(out, "    Number Of Endpoints (includes EP0):   ");
        let _ = writeln!(out,
            "        Total Endpoints for Configuration {} (current):   {}",
            cfg.b_configuration_value, ep_count
        );
    }

    format_device_descriptor(d, out);
    for cfg in &d.configurations {
        format_config_descriptor(d, cfg, out);
    }
    let _ = writeln!(out);
}

// ── Device Descriptor ─────────────────────────────────────────────────────────

fn format_device_descriptor(d: &UsbDevice, out: &mut String) {
    let dd = &d.device_descriptor;
    let class = dd.b_device_class;
    let _ = writeln!(out, "    Device Descriptor   ");
    let _ = writeln!(out, "        Descriptor Version Number:   0x{:04X}", dd.bcd_usb);
    let _ = writeln!(out, "        Device Class:   {}   ({})", class, class_label(class));
    let _ = writeln!(out, "        Device Subclass:   {}", dd.b_device_sub_class);
    let proto_label = dev_protocol_label(class, dd.b_device_protocol);
    if proto_label.is_empty() {
        let _ = writeln!(out, "        Device Protocol:   {}", dd.b_device_protocol);
    } else {
        let _ = writeln!(out, "        Device Protocol:   {}   ({})", dd.b_device_protocol, proto_label);
    }
    let _ = writeln!(out, "        Device MaxPacketSize:   {}", dd.b_max_packet_size0);
    let _ = writeln!(out,
        "        Device VendorID/ProductID:   0x{:04X}/0x{:04X}   (unknown vendor)",
        dd.id_vendor, dd.id_product
    );
    let _ = writeln!(out, "        Device Version Number:   0x{:04X}", dd.bcd_device);
    let _ = writeln!(out, "        Number of Configurations:   {}", dd.b_num_configurations);
    let _ = writeln!(out, "        Manufacturer String:   {}", string_field(d, dd.i_manufacturer));
    let _ = writeln!(out, "        Product String:   {}", string_field(d, dd.i_product));
    let _ = writeln!(out, "        Serial Number String:   {}", string_field(d, dd.i_serial_number));
}

// ── Configuration Descriptor ──────────────────────────────────────────────────

fn format_config_descriptor(d: &UsbDevice, cfg: &ConfigDescriptor, out: &mut String) {
    let _ = writeln!(out, "    Configuration Descriptor (current config)   ");
    let _ = writeln!(out, "        Length (and contents):   {}", cfg.raw_bytes.len());
    format_raw_hex(&cfg.raw_bytes, 12, out);

    let unique: std::collections::HashSet<u8> =
        cfg.interfaces.iter().map(|i| i.b_interface_number).collect();
    let _ = writeln!(out, "        Number of Interfaces:   {}", unique.len());
    let _ = writeln!(out, "        Configuration Value:   {}", cfg.b_configuration_value);
    let _ = writeln!(out,
        "        Attributes:   0x{:02X} ({})",
        cfg.bm_attributes, config_attrs_desc(cfg.bm_attributes)
    );
    let _ = writeln!(out, "        MaxPower:   {} mA", cfg.b_max_power as u16 * 2);

    for iface in &cfg.interfaces {
        if let Some(iad) = cfg.interface_associations.iter()
            .find(|a| a.b_first_interface == iface.b_interface_number)
        {
            let class_name = class_label(iad.b_function_class);
            let name = string_for(d, iad.i_function)
                .map(|s| format!("   \"{}\"", s))
                .unwrap_or_default();
            let _ = writeln!(out, "        Interface Association   {}{}", class_name, name);
        }
        format_interface(d, iface, out);
    }
    for hid in &d.hid_interfaces {
        format_hid_descriptor(hid, out);
    }
}

// ── Interface ─────────────────────────────────────────────────────────────────

fn format_interface(d: &UsbDevice, iface: &InterfaceDescriptor, out: &mut String) {
    let class = iface.b_interface_class;
    let sub = iface.b_interface_sub_class;
    let class_name = class_label(class);
    let sub_name = subclass_label(class, sub);
    let class_sub = if sub_name.is_empty() {
        class_name.to_string()
    } else {
        format!("{}/{}", class_name, sub_name)
    };

    let alt_suffix = if iface.b_alternate_setting > 0 {
        format!(" (#{})", iface.b_alternate_setting)
    } else {
        String::new()
    };
    if let Some(name) = string_for(d, iface.i_interface) {
        let _ = writeln!(out,
            "        Interface #{} - {}{} ..............................................   \"{}\"",
            iface.b_interface_number, class_sub, alt_suffix, name
        );
    } else {
        let _ = writeln!(out,
            "        Interface #{} - {}{}   ", iface.b_interface_number, class_sub, alt_suffix
        );
    }

    let _ = writeln!(out, "            Alternate Setting   {}", iface.b_alternate_setting);
    let _ = writeln!(out, "            Number of Endpoints   {}", iface.endpoints.len());
    let _ = writeln!(out, "            Interface Class:   {}   ({})", class, class_name);
    if sub_name.is_empty() {
        let _ = writeln!(out, "            Interface Subclass;   {}", sub);
    } else {
        let _ = writeln!(out, "            Interface Subclass;   {}   ({})", sub, sub_name);
    }
    let _ = writeln!(out, "            Interface Protocol:   {}", iface.b_interface_protocol);

    for cs in &iface.class_descriptors {
        format_class_specific(class, sub, cs, out);
    }
    for ep in &iface.endpoints {
        format_endpoint(ep, out);
    }
}

// ── Class-specific descriptors ────────────────────────────────────────────────

fn format_class_specific(iface_class: u8, iface_sub_class: u8, cs: &usb_types::ClassSpecificDescriptor, out: &mut String) {
    let typ = cs.descriptor_type;
    let sub = cs.descriptor_subtype;
    let data = &cs.data;
    let hex_data = |d: &[u8]| d.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
    let jack_type = |t: u8| match t { 0x01 => "Embedded", 0x02 => "External", _ => "Unknown" };

    if typ == 0x21 {
        let bcd_hid = if data.len() >= 2 { u16::from_le_bytes([data[0], data[1]]) } else { 0 };
        let _ = writeln!(out, "            HID Descriptor");
        let _ = writeln!(out, "                bcdHID:   0x{:04X}", bcd_hid);
        if data.len() >= 4 {
            let _ = writeln!(out, "                bCountryCode:   {}", data[2]);
            let _ = writeln!(out, "                bNumDescriptors:   {}", data[3]);
        }
        if data.len() >= 7 {
            let desc_type_name = match data[4] { 0x22 => "Report", 0x23 => "Physical", _ => "Unknown" };
            let desc_len = u16::from_le_bytes([data[5], data[6]]);
            let _ = writeln!(out, "                bDescriptorType:   {}", desc_type_name);
            let _ = writeln!(out, "                wDescriptorLength:   {}", desc_len);
        }
        return;
    }

    if typ == 0x24 && iface_class == 0x02 {
        let label = match sub {
            0x00 => "Comm Class Header Functional Descriptor",
            0x01 => "Comm Class Call Management Functional Descriptor",
            0x02 => "Comm Class Abstract Control Management Functional Descriptor",
            0x06 => "Comm Class Union Functional Descriptor",
            _    => "Comm Class-Specific Descriptor",
        };
        let _ = writeln!(out, "            {}", label);
        match sub {
            0x00 if data.len() >= 2 => {
                let bcd = u16::from_le_bytes([data[0], data[1]]);
                let _ = writeln!(out, "                bcdCDC:   0x{:04X}", bcd);
            }
            0x01 if data.len() >= 2 => {
                let _ = writeln!(out, "                bmCapabilities:   0x{:02X}", data[0]);
                let _ = writeln!(out, "                bDataInterface:   {}", data[1]);
            }
            0x02 if !data.is_empty() => {
                let _ = writeln!(out, "                bmCapabilities:   0x{:02X}", data[0]);
            }
            0x06 if data.len() >= 2 => {
                let _ = writeln!(out, "                bMasterInterface:   {}", data[0]);
                for (i, &s) in data[1..].iter().enumerate() {
                    let _ = writeln!(out, "                bSlaveInterface{}:   {}", i, s);
                }
            }
            _ => { let _ = writeln!(out, "                data:   {}", hex_data(data)); }
        }
        return;
    }

    if typ == 0x24 && iface_class == 0x01 {
        match (iface_sub_class, sub) {
            (1, 0x01) if data.len() >= 4 => {
                let bcd = u16::from_le_bytes([data[0], data[1]]);
                let total = u16::from_le_bytes([data[2], data[3]]);
                let n = data.get(4).copied().unwrap_or(0) as usize;
                let _ = writeln!(out, "            Audio Control Interface Header Descriptor");
                let _ = writeln!(out, "                bcdADC:   0x{:04X}", bcd);
                let _ = writeln!(out, "                wTotalLength:   {}", total);
                let _ = writeln!(out, "                bInCollection:   {}", n);
                for (i, &v) in data[5..].iter().take(n).enumerate() {
                    let _ = writeln!(out, "                baInterfaceNr[{}]:   {}", i, v);
                }
            }
            (3, 0x01) if data.len() >= 4 => {
                let bcd = u16::from_le_bytes([data[0], data[1]]);
                let total = u16::from_le_bytes([data[2], data[3]]);
                let _ = writeln!(out, "            MIDI Streaming Interface Header Descriptor");
                let _ = writeln!(out, "                bcdMSC:   0x{:04X}", bcd);
                let _ = writeln!(out, "                wTotalLength:   {}", total);
            }
            (3, 0x02) if data.len() >= 3 => {
                let _ = writeln!(out, "            MIDI IN Jack Descriptor");
                let _ = writeln!(out, "                bJackType:   {} (0x{:02X})", jack_type(data[0]), data[0]);
                let _ = writeln!(out, "                bJackID:   {}", data[1]);
                let _ = writeln!(out, "                iJack:   {}", data[2]);
            }
            (3, 0x03) if data.len() >= 3 => {
                let n_pins = data[2] as usize;
                let i_jack = data.get(3 + n_pins * 2).copied().unwrap_or(0);
                let _ = writeln!(out, "            MIDI OUT Jack Descriptor");
                let _ = writeln!(out, "                bJackType:   {} (0x{:02X})", jack_type(data[0]), data[0]);
                let _ = writeln!(out, "                bJackID:   {}", data[1]);
                let _ = writeln!(out, "                bNrInputPins:   {}", n_pins);
                for i in 0..n_pins {
                    let _ = writeln!(out, "                baSourceID[{}]:   {}", i, data.get(3 + i * 2).copied().unwrap_or(0));
                    let _ = writeln!(out, "                baSourcePin[{}]:   {}", i, data.get(4 + i * 2).copied().unwrap_or(0));
                }
                let _ = writeln!(out, "                iJack:   {}", i_jack);
            }
            _ => {
                let _ = writeln!(out, "            Audio Class-Specific Descriptor (subtype 0x{:02X})", sub);
                let _ = writeln!(out, "                data:   {}", hex_data(data));
            }
        }
        return;
    }

    if typ == 0x25 && iface_class == 0x01 && iface_sub_class == 0x03 && sub == 0x01 && !data.is_empty() {
        let n = data[0] as usize;
        let _ = writeln!(out, "            MIDI Streaming Data Endpoint Descriptor");
        let _ = writeln!(out, "                bNumEmbMIDIJack:   {}", n);
        for (i, &j) in data[1..].iter().take(n).enumerate() {
            let _ = writeln!(out, "                baAssocJackID[{}]:   {}", i, j);
        }
        return;
    }

    // Generic fallback
    let type_label = match typ {
        0x24 => format!("CS_INTERFACE (subtype 0x{:02X})", sub),
        0x25 => format!("CS_ENDPOINT (subtype 0x{:02X})", sub),
        _    => format!("Class-Specific (type 0x{:02X})", typ),
    };
    let _ = writeln!(out, "            {}", type_label);
    let _ = writeln!(out, "                data:   {}", hex_data(data));
}

// ── Endpoint ──────────────────────────────────────────────────────────────────

fn format_endpoint(ep: &EndpointDescriptor, out: &mut String) {
    let dir_label = if ep.b_endpoint_address & 0x80 != 0 { "Input" } else { "Output" };
    let dir_short = if ep.b_endpoint_address & 0x80 != 0 { "IN" } else { "OUT" };
    let xfer = xfer_label(ep.bm_attributes & 0x03);
    let _ = writeln!(out,
        "            Endpoint 0x{:02X} - {} {}   ", ep.b_endpoint_address, xfer, dir_label
    );
    let _ = writeln!(out, "                Address:   0x{:02X}  ({})", ep.b_endpoint_address, dir_short);
    let _ = writeln!(out, "                Attributes:   0x{:02X}  ({})", ep.bm_attributes, xfer);
    let _ = writeln!(out, "                Max Packet Size:   {}",
        format_pkt_size(ep.w_max_packet_size, ep.bm_attributes & 0x03));
    let _ = writeln!(out, "                Polling Interval:   {} ms", ep.b_interval);
}

// ── HID Report Descriptor ─────────────────────────────────────────────────────

fn format_hid_descriptor(hid: &HidInterface, out: &mut String) {
    if hid.raw_report_descriptor.is_empty() { return; }
    let _ = writeln!(out, "                    Parsed Report Descriptor:   ");
    if let Some(nodes) = &hid.parsed {
        let _ = write!(out, "{}", render_text(nodes, 26));
    }
}

// ── Raw hex ───────────────────────────────────────────────────────────────────

fn format_raw_hex(bytes: &[u8], indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let offset = row * 16;
        let hi = &chunk[..chunk.len().min(8)];
        let lo = if chunk.len() > 8 { &chunk[8..] } else { &[][..] };
        let hi_str: String = hi.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
        let lo_str: String = lo.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
        if lo.is_empty() {
            let _ = writeln!(out, "{}Raw Descriptor (hex)    {:04X}: {}", pad, offset, hi_str);
        } else {
            let _ = writeln!(out, "{}Raw Descriptor (hex)    {:04X}: {}  {}", pad, offset, hi_str, lo_str);
        }
    }
}

// ── Lookup helpers ────────────────────────────────────────────────────────────

fn string_for<'a>(d: &'a UsbDevice, index: u8) -> Option<&'a str> {
    if index == 0 { return None; }
    d.strings.iter().find(|s| s.index == index).map(|s| s.value.as_str())
}

fn string_field(d: &UsbDevice, index: u8) -> String {
    if index == 0 { return "0 (none)".to_string(); }
    match string_for(d, index) {
        Some(s) => format!("{} \"{}\"", index, s),
        None    => format!("{} (none)", index),
    }
}

fn speed_label(s: &UsbSpeed) -> &'static str {
    match s {
        UsbSpeed::Low       => "Low Speed",
        UsbSpeed::Full      => "Full Speed",
        UsbSpeed::High      => "High Speed",
        UsbSpeed::Super     => "Super Speed",
        UsbSpeed::SuperPlus => "Super Speed+",
        UsbSpeed::Unknown   => "Unknown Speed",
    }
}

fn device_class_header(class: u8) -> &'static str {
    match class {
        usb_class::COMPOSITE     => "Composite device",
        usb_class::HUB           => "Hub device",
        usb_class::VENDOR_SPECIFIC => "Vendor Specific device",
        _                        => "Device",
    }
}

fn class_label(class: u8) -> &'static str {
    match class {
        usb_class::COMPOSITE     => "Composite",
        usb_class::AUDIO         => "Audio",
        usb_class::COMMUNICATIONS => "Communications-Control",
        usb_class::HID           => "HID",
        usb_class::PHYSICAL      => "Physical",
        usb_class::IMAGE         => "Image",
        usb_class::PRINTER       => "Printer",
        usb_class::MASS_STORAGE  => "Mass Storage",
        usb_class::HUB           => "Hub",
        usb_class::CDC_DATA      => "Communications-Data",
        usb_class::SMART_CARD    => "Smart Card",
        usb_class::VIDEO         => "Video",
        usb_class::DIAGNOSTIC    => "Diagnostic",
        usb_class::WIRELESS      => "Wireless Controller",
        usb_class::APPLICATION   => "Application Specific",
        usb_class::VENDOR_SPECIFIC => "Vendor Specific",
        _                        => "Unknown",
    }
}

fn subclass_label(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (1, 1)  => "Control",
        (1, 2)  => "Streaming",
        (1, 3)  => "MIDI Streaming",
        (2, 2)  => "Abstract Control Model",
        (8, 1)  => "RBC",
        (8, 2)  => "ATAPI",
        (8, 3)  => "QIC-157",
        (8, 4)  => "UFI",
        (8, 5)  => "SFF-8070i",
        (8, 6)  => "SCSI",
        (10, 0) => "Unknown Comm Class Model",
        _       => "",
    }
}

fn dev_protocol_label(class: u8, protocol: u8) -> &'static str {
    match (class, protocol) {
        (usb_class::HUB, 0) => "Full/Low Speed",
        (usb_class::HUB, 1) => "High Speed Single Transaction Translator",
        (usb_class::HUB, 2) => "High Speed Multi Transaction Translator",
        _                   => "",
    }
}

fn xfer_label(xfer: u8) -> &'static str {
    match xfer {
        0 => "Control",
        1 => "Isochronous",
        2 => "Bulk",
        3 => "Interrupt",
        _ => "Unknown",
    }
}

fn config_attrs_desc(attrs: u8) -> String {
    let mut parts = Vec::new();
    if attrs & 0x40 != 0 { parts.push("self-powered"); } else { parts.push("bus-powered"); }
    if attrs & 0x20 != 0 { parts.push("remote wakeup"); }
    parts.join(", ")
}

fn format_pkt_size(size: u16, xfer: u8) -> String {
    if xfer == 1 || xfer == 3 {
        let pkt = size & 0x07FF;
        let mult = ((size >> 11) & 0x03) + 1;
        format!("0x{:04X}  ({} x {} transactions per microframe)", size, mult, pkt)
    } else {
        format!("{}", size)
    }
}
