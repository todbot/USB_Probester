use std::fmt::Write;
use hid_parser::render_text;
use usb_types::*;

pub fn format_devices(devices: &[UsbDevice]) -> String {
    let mut out = String::new();
    for d in devices {
        format_device(d, &mut out);
    }
    out
}

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
        let ep_count: usize = cfg.interfaces.iter().map(|i| i.endpoints.len()).sum::<usize>() + 1;
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

fn format_device_descriptor(d: &UsbDevice, out: &mut String) {
    let dd = &d.device_descriptor;
    let class = dd.b_device_class;
    let _ = writeln!(out, "    Device Descriptor   ");
    let _ = writeln!(out, "        Descriptor Version Number:   0x{:04X}", dd.bcd_usb);
    let _ = writeln!(out, "        Device Class:   {}   ({})", class, class_label(class));
    let _ = writeln!(out, "        Device Subclass:   {}", dd.b_device_sub_class);
    let _ = writeln!(out,
        "        Device Protocol:   {}   ({})",
        dd.b_device_protocol, dev_protocol_label(class, dd.b_device_protocol)
    );
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
        format_interface(d, iface, out);
    }
    for hid in &d.hid_interfaces {
        format_hid_descriptor(hid, out);
    }
}

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

    if let Some(name) = string_for(d, iface.i_interface) {
        let _ = writeln!(out,
            "        Interface #{} - {} ..............................................   \"{}\"",
            iface.b_interface_number, class_sub, name
        );
    } else {
        let _ = writeln!(out,
            "        Interface #{} - {}   ", iface.b_interface_number, class_sub
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

    for ep in &iface.endpoints {
        format_endpoint(ep, out);
    }
}

fn format_endpoint(ep: &EndpointDescriptor, out: &mut String) {
    let dir_label = if ep.b_endpoint_address & 0x80 != 0 { "Input" } else { "Output" };
    let dir_short = if ep.b_endpoint_address & 0x80 != 0 { "IN" } else { "OUT" };
    let xfer = xfer_label(ep.bm_attributes & 0x03);
    let _ = writeln!(out,
        "            Endpoint 0x{:02X} - {} {}   ", ep.b_endpoint_address, xfer, dir_label
    );
    let _ = writeln!(out, "                Address:   0x{:02X}  ({})", ep.b_endpoint_address, dir_short);
    let _ = writeln!(out, "                Attributes:   0x{:02X}  ({})", ep.bm_attributes, xfer);
    let _ = writeln!(out, "                Max Packet Size:   {}", ep.w_max_packet_size);
    let _ = writeln!(out, "                Polling Interval:   {} ms", ep.b_interval);
}

fn format_hid_descriptor(hid: &HidInterface, out: &mut String) {
    if hid.raw_report_descriptor.is_empty() { return; }
    let _ = writeln!(out, "                    Parsed Report Descriptor:   ");
    if let Some(nodes) = &hid.parsed {
        let _ = write!(out, "{}", render_text(nodes, 26));
    }
}

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
        0    => "Composite device",
        9    => "Hub device",
        0xFF => "Vendor Specific device",
        _    => "Device",
    }
}

fn class_label(class: u8) -> &'static str {
    match class {
        0    => "Composite",        1  => "Audio",
        2    => "Communications-Control",
        3    => "HID",              5  => "Physical",
        6    => "Image",            7  => "Printer",
        8    => "Mass Storage",     9  => "Hub",
        10   => "Communications-Data",
        11   => "Smart Card",       14 => "Video",
        0xDC => "Diagnostic",       0xE0 => "Wireless Controller",
        0xFE => "Application Specific",
        0xFF => "Vendor Specific",
        _    => "Unknown",
    }
}

fn subclass_label(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (1, 1)  => "Control",       (1, 2) => "Streaming",
        (1, 3)  => "MIDI Streaming",
        (2, 2)  => "Abstract Control Model",
        (8, 1)  => "RBC",           (8, 2) => "ATAPI",
        (8, 3)  => "QIC-157",       (8, 4) => "UFI",
        (8, 5)  => "SFF-8070i",     (8, 6) => "SCSI",
        (10, 0) => "Unknown Comm Class Model",
        _       => "",
    }
}

fn dev_protocol_label(class: u8, protocol: u8) -> &'static str {
    match (class, protocol) {
        (9, 0) => "Full/Low Speed",
        (9, 1) => "High Speed Single Transaction Translator",
        (9, 2) => "High Speed Multi Transaction Translator",
        _      => "",
    }
}

fn xfer_label(xfer: u8) -> &'static str {
    match xfer { 0 => "Control", 1 => "Isochronous", 2 => "Bulk", 3 => "Interrupt", _ => "Unknown" }
}

fn config_attrs_desc(attrs: u8) -> String {
    let mut parts = Vec::new();
    if attrs & 0x40 != 0 { parts.push("self-powered"); } else { parts.push("bus-powered"); }
    if attrs & 0x20 != 0 { parts.push("remote wakeup"); }
    parts.join(", ")
}
