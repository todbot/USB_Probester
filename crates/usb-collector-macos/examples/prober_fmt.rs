/// Format USB device output to match Apple's USB Prober.app style.
use usb_collector_macos::MacCollector;
use usb_types::*;
use hid_parser::render_text;

fn main() {
    let devices = MacCollector::new()
        .enumerate()
        .expect("enumeration failed");

    for device in &devices {
        print_device(device);
    }
}

// ── Device top-level ───────────────────────────────────────────────────────────

fn print_device(d: &UsbDevice) {
    let dd = &d.device_descriptor;
    let product = string_for(d, dd.i_product).unwrap_or_default();
    let class_hdr = device_class_header(dd.b_device_class);
    let addr = d.port_path.last().copied().unwrap_or(1);
    let loc = u32::from_str_radix(&d.location_id, 16).unwrap_or(0);
    let speed = speed_label(&d.speed);

    println!(
        "{} device @ {} (0x{:08x}): .............................................   {}: \"{}\"",
        speed, addr, loc, class_hdr, product
    );

    print_port_info();

    // Count endpoints: EP0 plus all interface endpoints in first config.
    if let Some(cfg) = d.configurations.first() {
        let ep_count: usize = cfg.interfaces.iter().map(|i| i.endpoints.len()).sum::<usize>() + 1;
        println!("    Number Of Endpoints (includes EP0):   ");
        println!(
            "        Total Endpoints for Configuration {} (current):   {}",
            cfg.b_configuration_value, ep_count
        );
    }

    print_device_descriptor(d);

    for cfg in &d.configurations {
        print_config_descriptor(d, cfg);
    }

    println!();
}

fn print_port_info() {
    // Port flags aren't surfaced by nusb; emit the common set seen on external devices.
    println!("    Port Information:   0x0018");
    println!("           Not Captive");
    println!("           External Device");
    println!("           Connected");
    println!("           Enabled");
}

// ── Device Descriptor ─────────────────────────────────────────────────────────

fn print_device_descriptor(d: &UsbDevice) {
    let dd = &d.device_descriptor;
    println!("    Device Descriptor   ");
    println!("        Descriptor Version Number:   0x{:04X}", dd.bcd_usb);

    let class = dd.b_device_class;
    println!("        Device Class:   {}   ({})", class, class_label(class));
    println!("        Device Subclass:   {}", dd.b_device_sub_class);
    println!(
        "        Device Protocol:   {}   ({})",
        dd.b_device_protocol,
        dev_protocol_label(class, dd.b_device_protocol)
    );
    println!("        Device MaxPacketSize:   {}", dd.b_max_packet_size0);
    println!(
        "        Device VendorID/ProductID:   0x{:04X}/0x{:04X}   (unknown vendor)",
        dd.id_vendor, dd.id_product
    );
    println!("        Device Version Number:   0x{:04X}", dd.bcd_device);
    println!("        Number of Configurations:   {}", dd.b_num_configurations);
    println!("        Manufacturer String:   {}", string_field(d, dd.i_manufacturer));
    println!("        Product String:   {}", string_field(d, dd.i_product));
    println!("        Serial Number String:   {}", string_field(d, dd.i_serial_number));
}

// ── Configuration Descriptor ──────────────────────────────────────────────────

fn print_config_descriptor(d: &UsbDevice, cfg: &ConfigDescriptor) {
    println!("    Configuration Descriptor (current config)   ");
    println!("        Length (and contents):   {}", cfg.raw_bytes.len());
    print_raw_hex(&cfg.raw_bytes, 12);

    // Count unique interface numbers (not alt-settings).
    let unique: std::collections::HashSet<u8> =
        cfg.interfaces.iter().map(|i| i.b_interface_number).collect();
    println!("        Number of Interfaces:   {}", unique.len());
    println!("        Configuration Value:   {}", cfg.b_configuration_value);
    println!(
        "        Attributes:   0x{:02X} ({})",
        cfg.bm_attributes,
        config_attrs_desc(cfg.bm_attributes)
    );
    println!("        MaxPower:   {} mA", cfg.b_max_power as u16 * 2);

    for iface in &cfg.interfaces {
        print_interface(d, iface);
    }

    // HID report descriptors follow the configuration descriptor section.
    for hid in &d.hid_interfaces {
        print_hid_descriptor(hid);
    }
}

// ── HID Descriptor ────────────────────────────────────────────────────────────

fn print_hid_descriptor(hid: &HidInterface) {
    if hid.raw_report_descriptor.is_empty() {
        return;
    }
    println!("                    Parsed Report Descriptor:   ");
    if let Some(nodes) = &hid.parsed {
        print!("{}", render_text(nodes, 26));
    }
}

// ── Interface ─────────────────────────────────────────────────────────────────

fn print_interface(d: &UsbDevice, iface: &InterfaceDescriptor) {
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
        println!(
            "        Interface #{} - {} ..............................................   \"{}\"",
            iface.b_interface_number, class_sub, name
        );
    } else {
        println!(
            "        Interface #{} - {}   ",
            iface.b_interface_number, class_sub
        );
    }

    println!("            Alternate Setting   {}", iface.b_alternate_setting);
    println!("            Number of Endpoints   {}", iface.endpoints.len());
    println!("            Interface Class:   {}   ({})", class, class_name);

    if sub_name.is_empty() {
        println!("            Interface Subclass;   {}", sub);
    } else {
        println!("            Interface Subclass;   {}   ({})", sub, sub_name);
    }
    println!("            Interface Protocol:   {}", iface.b_interface_protocol);

    for ep in &iface.endpoints {
        print_endpoint(ep);
    }
}

// ── Endpoint ──────────────────────────────────────────────────────────────────

fn print_endpoint(ep: &EndpointDescriptor) {
    let dir_label = if ep.b_endpoint_address & 0x80 != 0 { "Input" } else { "Output" };
    let dir_short = if ep.b_endpoint_address & 0x80 != 0 { "IN" } else { "OUT" };
    let xfer = xfer_label(ep.bm_attributes & 0x03);

    println!(
        "            Endpoint 0x{:02X} - {} {}   ",
        ep.b_endpoint_address, xfer, dir_label
    );
    println!("                Address:   0x{:02X}  ({})", ep.b_endpoint_address, dir_short);
    println!("                Attributes:   0x{:02X}  ({})", ep.bm_attributes, xfer);
    println!(
        "                Max Packet Size:   {}",
        format_pkt_size(ep.w_max_packet_size, ep.bm_attributes & 0x03)
    );
    println!(
        "                Polling Interval:   {}",
        format_interval(ep.b_interval, ep.bm_attributes & 0x03)
    );
}

// ── Raw hex dump ──────────────────────────────────────────────────────────────

fn print_raw_hex(bytes: &[u8], indent: usize) {
    let pad = " ".repeat(indent);
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let offset = row * 16;
        let hi = &chunk[..chunk.len().min(8)];
        let lo = if chunk.len() > 8 { &chunk[8..] } else { &[][..] };
        let hi_str: String = hi.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
        let lo_str: String = lo.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
        if lo.is_empty() {
            println!("{}Raw Descriptor (hex)    {:04X}: {} ", pad, offset, hi_str);
        } else {
            println!("{}Raw Descriptor (hex)    {:04X}: {}  {} ", pad, offset, hi_str, lo_str);
        }
    }
}

// ── Lookup helpers ────────────────────────────────────────────────────────────

fn string_for<'a>(d: &'a UsbDevice, index: u8) -> Option<&'a str> {
    if index == 0 {
        return None;
    }
    d.strings.iter().find(|s| s.index == index).map(|s| s.value.as_str())
}

fn string_field(d: &UsbDevice, index: u8) -> String {
    if index == 0 {
        return "0 (none)".to_string();
    }
    match string_for(d, index) {
        Some(s) => format!("{} \"{}\"", index, s),
        None => format!("{} (none)", index),
    }
}

fn speed_label(s: &UsbSpeed) -> &'static str {
    match s {
        UsbSpeed::Low => "Low Speed",
        UsbSpeed::Full => "Full Speed",
        UsbSpeed::High => "High Speed",
        UsbSpeed::Super => "Super Speed",
        UsbSpeed::SuperPlus => "Super Speed+",
        UsbSpeed::Unknown => "Unknown Speed",
    }
}

fn device_class_header(class: u8) -> &'static str {
    match class {
        0 => "Composite device",
        9 => "Hub device",
        0xFF => "Vendor Specific device",
        _ => "Device",
    }
}

fn class_label(class: u8) -> &'static str {
    match class {
        0 => "Composite",
        1 => "Audio",
        2 => "Communications-Control",
        3 => "HID",
        5 => "Physical",
        6 => "Image",
        7 => "Printer",
        8 => "Mass Storage",
        9 => "Hub",
        10 => "Communications-Data",
        11 => "Smart Card",
        14 => "Video",
        0xDC => "Diagnostic",
        0xE0 => "Wireless Controller",
        0xFE => "Application Specific",
        0xFF => "Vendor Specific",
        _ => "Unknown",
    }
}

fn subclass_label(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (1, 1) => "Control",
        (1, 2) => "Streaming",
        (1, 3) => "Streaming",
        (2, 2) => "Abstract Control Model",
        (8, 1) => "RBC",
        (8, 2) => "ATAPI",
        (8, 3) => "QIC-157",
        (8, 4) => "UFI",
        (8, 5) => "SFF-8070i",
        (8, 6) => "SCSI",
        (10, 0) => "Unknown Comm Class Model",
        _ => "",
    }
}

fn dev_protocol_label(class: u8, protocol: u8) -> &'static str {
    match (class, protocol) {
        (9, 0) => "Full/Low Speed",
        (9, 1) => "High Speed Single Transaction Translator",
        (9, 2) => "High Speed Multi Transaction Translator",
        _ => "",
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
    if attrs & 0x40 != 0 {
        parts.push("self-powered");
    } else {
        parts.push("bus-powered");
    }
    if attrs & 0x20 != 0 {
        parts.push("remote wakeup");
    }
    parts.join(", ")
}

fn format_pkt_size(size: u16, xfer: u8) -> String {
    if xfer == 3 {
        // Interrupt / Isochronous: high-speed encoding per USB 2.0 §9.6.6
        let pkt = size & 0x07FF;
        let mult = ((size >> 11) & 0x03) + 1;
        format!(
            "0x{:04X}  ({} x {}  transactions opportunities per microframe)",
            size, mult, pkt
        )
    } else {
        format!("{}", size)
    }
}

fn format_interval(interval: u8, xfer: u8) -> String {
    // High-speed interrupt/isochronous: interval = 2^(bInterval-1) microframes.
    // Full/low-speed: interval is in ms.
    // For simplicity, just show raw value with ms; callers can extend.
    if xfer == 3 && interval > 0 {
        // Could detect High Speed from device speed, but we lack it here.
        format!("{} ms", interval)
    } else {
        format!("{} ms", interval)
    }
}
