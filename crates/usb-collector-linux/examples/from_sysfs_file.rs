/// Parse a stored sysfs `descriptors` binary and display the resulting UsbDevice.
/// Validates the descriptor parser independently from live USB hardware.
///
/// Usage:
///   # Use the built-in blink(1) fixture:
///   cargo run --example from_sysfs_file
///
///   # Load a real sysfs descriptors file:
///   cargo run --example from_sysfs_file -- /sys/bus/usb/devices/2-4/descriptors
///
///   # Save a snapshot and replay it later:
///   cp /sys/bus/usb/devices/2-2.3/descriptors blink1.bin
///   cargo run --example from_sysfs_file -- blink1.bin
use std::env;
use std::fs;
use usb_collector_linux::device_from_descriptor_bytes;
use usb_types::*;

// ── Hardcoded fixture: ThingM blink(1) mk2 (27B8:01ED) ───────────────────────
//
// Captured from /sys/bus/usb/devices/2-2.3/descriptors on a Linux machine.
// Device descriptor + 1 configuration (1 HID interface, 2 interrupt endpoints).
const BLINK1_DESCRIPTORS: &[u8] = &[
    // Device descriptor (18 bytes)
    0x12, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x08,
    0xb8, 0x27, 0xed, 0x01, 0x02, 0x00, 0x01, 0x02,
    0x03, 0x01,
    // Configuration descriptor (wTotalLength=41)
    0x09, 0x02, 0x29, 0x00, 0x01, 0x01, 0x00, 0x80, 0x3c,
    // Interface descriptor #0 — HID
    0x09, 0x04, 0x00, 0x00, 0x02, 0x03, 0x00, 0x00, 0x00,
    // HID class descriptor (skipped by parser, present in raw)
    0x09, 0x21, 0x01, 0x01, 0x00, 0x01, 0x22, 0x18, 0x00,
    // Endpoint 0x81 — Interrupt IN
    0x07, 0x05, 0x81, 0x03, 0x08, 0x00, 0x01,
    // Endpoint 0x01 — Interrupt OUT
    0x07, 0x05, 0x01, 0x03, 0x08, 0x00, 0x01,
];

// HID report descriptor captured from
// /sys/bus/usb/devices/2-2.3/2-2.3:1.0/0003:27B8:01ED.*/report_descriptor
const BLINK1_HID_REPORT: &[u8] = &[
    0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x15,
    0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x85, 0x01,
    0x95, 0x08, 0x09, 0x00, 0xb2, 0x02, 0x01, 0xc0,
];

fn main() {
    let args: Vec<String> = env::args().collect();

    let (raw, label) = if args.len() > 1 {
        let path = &args[1];
        let bytes = fs::read(path).unwrap_or_else(|e| {
            eprintln!("error reading {path}: {e}");
            std::process::exit(1);
        });
        (bytes, path.clone())
    } else {
        println!("No file argument — using built-in blink(1) fixture.\n");
        (BLINK1_DESCRIPTORS.to_vec(), "blink(1) fixture".to_string())
    };

    // When loading from fixture, inject the known HID report descriptor.
    // When loading from a real file, HID data would need to be read separately
    // from the interface subdirectory; here we include it for the fixture.
    let hid_interfaces = if args.len() <= 1 {
        let parsed = hid_parser::parse(BLINK1_HID_REPORT).ok();
        vec![HidInterface {
            interface_number: 0,
            raw_report_descriptor: BLINK1_HID_REPORT.to_vec(),
            parsed,
        }]
    } else {
        vec![]
    };

    let strings = if args.len() <= 1 {
        // Strings as they would appear from nusb/kernel cache for blink(1)
        vec![
            StringDescriptor { index: 1, value: "ThingM".to_string() },
            StringDescriptor { index: 2, value: "blink(1) mk2".to_string() },
            StringDescriptor { index: 3, value: "2-2.3".to_string() },
        ]
    } else {
        vec![]
    };

    match device_from_descriptor_bytes(
        &raw,
        &label,
        /*bus_number=*/ 0,
        /*port_path=*/  vec![],
        UsbSpeed::Full,
        strings,
        hid_interfaces,
    ) {
        Some(device) => print_device(&device),
        None => {
            eprintln!("Failed to parse descriptor bytes from '{label}'.");
            std::process::exit(1);
        }
    }
}

fn print_device(d: &UsbDevice) {
    let dd = &d.device_descriptor;

    let product = d.strings.iter().find(|s| s.index == dd.i_product).map(|s| s.value.as_str()).unwrap_or("?");
    let mfr     = d.strings.iter().find(|s| s.index == dd.i_manufacturer).map(|s| s.value.as_str()).unwrap_or("?");

    println!("=== {} ===", d.location_id);
    println!("  {:?}  {:04X}:{:04X}  \"{}\" / \"{}\"",
        d.speed, dd.id_vendor, dd.id_product, mfr, product);
    println!("  bcdUSB={:04X}  class={:#04x}  sub={:#04x}  proto={:#04x}  maxpkt0={}",
        dd.bcd_usb, dd.b_device_class, dd.b_device_sub_class, dd.b_device_protocol, dd.b_max_packet_size0);
    println!("  bcdDevice={:04X}  iMfr={}  iProd={}  iSerial={}  numConfigs={}",
        dd.bcd_device, dd.i_manufacturer, dd.i_product, dd.i_serial_number, dd.b_num_configurations);
    println!("  raw device descriptor ({} bytes): {}", dd.raw_bytes.len(), hex_line(&dd.raw_bytes));

    for (ci, cfg) in d.configurations.iter().enumerate() {
        println!();
        println!("  Config {} ({} raw bytes):", ci + 1, cfg.raw_bytes.len());
        println!("    bConfigValue={}  bmAttrs={:#04x}  maxPower={}mA",
            cfg.b_configuration_value, cfg.bm_attributes, cfg.b_max_power as u16 * 2);
        println!("    raw config blob: {}", hex_line(&cfg.raw_bytes));

        for iface in &cfg.interfaces {
            println!("    Interface #{}.{}  class={:#04x} sub={:#04x} proto={:#04x}  {} EP(s)",
                iface.b_interface_number, iface.b_alternate_setting,
                iface.b_interface_class, iface.b_interface_sub_class,
                iface.b_interface_protocol, iface.endpoints.len());
            for ep in &iface.endpoints {
                let dir = if ep.b_endpoint_address & 0x80 != 0 { "IN" } else { "OUT" };
                let xfer = match ep.bm_attributes & 0x03 {
                    0 => "Control", 1 => "Iso", 2 => "Bulk", _ => "Interrupt",
                };
                println!("      EP {:#04x} {dir} {xfer}  maxpkt={}  interval={}ms",
                    ep.b_endpoint_address, ep.w_max_packet_size, ep.b_interval);
            }
        }
    }

    for hid in &d.hid_interfaces {
        println!();
        println!("  HID iface #{} report descriptor ({} bytes):",
            hid.interface_number, hid.raw_report_descriptor.len());
        for (i, chunk) in hid.raw_report_descriptor.chunks(16).enumerate() {
            println!("    {:04X}: {}", i * 16, hex_line(chunk));
        }
        if let Some(nodes) = &hid.parsed {
            println!("  Parsed HID:");
            print!("{}", hid_parser::render_text(nodes, 4));
        }
    }

    println!();
    println!("  Strings:");
    for s in &d.strings {
        println!("    [{}] {:?}", s.index, s.value);
    }
}

fn hex_line(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}
