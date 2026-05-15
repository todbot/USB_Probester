/// Enumerate USB devices via the live LinuxCollector and dump device info.
/// Validates nusb enumeration + sysfs descriptor parsing + HID collection.
///
/// Usage: cargo run --example dump_one
use usb_collector_linux::LinuxCollector;
use usb_types::*;

fn main() {
    let devices = LinuxCollector::new()
        .enumerate()
        .expect("enumeration failed");

    if devices.is_empty() {
        println!("No USB devices found.");
        return;
    }

    println!("Found {} USB device(s).\n", devices.len());

    for device in &devices {
        print_device(device, 0);
    }
}

fn print_device(d: &UsbDevice, depth: usize) {
    let indent = "  ".repeat(depth);
    let dd = &d.device_descriptor;

    let product = d.strings.iter()
        .find(|s| s.index == dd.i_product)
        .map(|s| s.value.as_str())
        .unwrap_or("<no product string>");
    let vendor = d.strings.iter()
        .find(|s| s.index == dd.i_manufacturer)
        .map(|s| s.value.as_str())
        .unwrap_or("");
    let serial = d.strings.iter()
        .find(|s| s.index == dd.i_serial_number)
        .map(|s| s.value.as_str())
        .unwrap_or("");

    println!("{indent}{:?} @ {} ({})  \"{}\"",
        d.speed,
        d.location_id,
        format!("{:04X}:{:04X}", dd.id_vendor, dd.id_product),
        product);

    println!("{indent}  Device Descriptor ({} raw bytes):", dd.raw_bytes.len());
    println!("{indent}    bcdUSB:           {:x}.{:02x}", dd.bcd_usb >> 8, dd.bcd_usb & 0xff);
    println!("{indent}    bDeviceClass:     {:#04x}", dd.b_device_class);
    println!("{indent}    bDeviceSubClass:  {:#04x}", dd.b_device_sub_class);
    println!("{indent}    bDeviceProtocol:  {:#04x}", dd.b_device_protocol);
    println!("{indent}    bMaxPacketSize0:  {}", dd.b_max_packet_size0);
    println!("{indent}    idVendor/Product: {:04X}h / {:04X}h  ({} / {})",
        dd.id_vendor, dd.id_product, vendor, product);
    println!("{indent}    bcdDevice:        {:x}.{:02x}", dd.bcd_device >> 8, dd.bcd_device & 0xff);
    println!("{indent}    iManufacturer:    {} {:?}", dd.i_manufacturer,
        d.strings.iter().find(|s| s.index == dd.i_manufacturer).map(|s| &s.value));
    println!("{indent}    iProduct:         {} {:?}", dd.i_product,
        d.strings.iter().find(|s| s.index == dd.i_product).map(|s| &s.value));
    println!("{indent}    iSerialNumber:    {} {:?}", dd.i_serial_number,
        if serial.is_empty() { None } else { Some(serial) });
    println!("{indent}    bNumConfigs:      {}", dd.b_num_configurations);

    if let Some(raw) = dd.raw_bytes.get(..18) {
        println!("{indent}    raw (18 bytes):   {}", hex_line(raw));
    }

    for (i, cfg) in d.configurations.iter().enumerate() {
        println!("{indent}  Configuration {} ({} raw bytes, {} interfaces):",
            i + 1, cfg.raw_bytes.len(), cfg.interfaces.len());
        if !cfg.raw_bytes.is_empty() {
            println!("{indent}    raw[0..min16]:    {}",
                hex_line(&cfg.raw_bytes[..cfg.raw_bytes.len().min(16)]));
        }
        for iface in &cfg.interfaces {
            println!("{indent}    Interface #{}.{}  class={:#04x} sub={:#04x} proto={:#04x}  {} endpoint(s)",
                iface.b_interface_number,
                iface.b_alternate_setting,
                iface.b_interface_class,
                iface.b_interface_sub_class,
                iface.b_interface_protocol,
                iface.endpoints.len());
            for ep in &iface.endpoints {
                let dir = if ep.b_endpoint_address & 0x80 != 0 { "IN" } else { "OUT" };
                println!("{indent}      EP {:#04x} ({dir})  attr={:#04x}  maxpkt={}  interval={}",
                    ep.b_endpoint_address, ep.bm_attributes,
                    ep.w_max_packet_size, ep.b_interval);
            }
        }
    }

    for hid in &d.hid_interfaces {
        println!("{indent}  HID iface #{} report descriptor ({} bytes):",
            hid.interface_number, hid.raw_report_descriptor.len());
        for (i, chunk) in hid.raw_report_descriptor.chunks(16).enumerate() {
            println!("{indent}    {:04X}: {}", i * 16, hex_line(chunk));
        }
        if let Some(nodes) = &hid.parsed {
            println!("{indent}  HID parsed ({} top-level node(s)):", nodes.len());
            print!("{}", hid_parser::render_text(nodes, indent.len() + 4));
        }
    }

    println!();

    for child in &d.children {
        print_device(child, depth + 1);
    }
}

fn hex_line(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}
