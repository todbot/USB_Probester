use clap::{Parser, ValueEnum};
use usb_types::{usb_class, *};

#[derive(Parser)]
#[command(name = "usb-probester-cli", about = "List USB devices (lsusb-style by default)")]
struct Cli {
    /// Show full USB Prober-style descriptor dump
    #[arg(long)]
    dump: bool,
    /// Output format (overrides --dump if specified)
    #[arg(short, long, value_enum)]
    format: Option<Format>,
    /// Omit USB hubs from output
    #[arg(long)]
    hide_hubs: bool,
    /// Filter by vendor ID (hex, e.g. 27b8 or 0x27b8)
    #[arg(long, value_parser = parse_hex_u16)]
    vid: Option<u16>,
    /// Filter by product ID (hex, e.g. 4444 or 0x4444)
    #[arg(long, value_parser = parse_hex_u16)]
    pid: Option<u16>,
}

fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u16::from_str_radix(s, 16).map_err(|_| format!("invalid hex value: '{s}'"))
}

#[derive(Clone, ValueEnum)]
enum Format {
    /// Mac USB Prober-style text tree
    Tree,
    /// Pretty-printed JSON
    Json,
}

fn main() {
    let cli = Cli::parse();
    let mut devices = enumerate();
    if cli.hide_hubs {
        devices.retain(|d| d.device_descriptor.b_device_class != usb_class::HUB);
    }
    if let Some(vid) = cli.vid {
        devices.retain(|d| d.device_descriptor.id_vendor == vid);
    }
    if let Some(pid) = cli.pid {
        devices.retain(|d| d.device_descriptor.id_product == pid);
    }
    match (cli.dump, &cli.format) {
        (_, Some(Format::Json)) => println!("{}", serde_json::to_string_pretty(&devices).expect("serialize failed")),
        (true, _) | (_, Some(Format::Tree)) => print!("{}", usb_formatter::format_devices(&devices)),
        (false, None) => print_lsusb(&devices),
    }
}

fn get_string(device: &UsbDevice, index: u8) -> &str {
    if index == 0 { return ""; }
    device.strings.iter()
        .find(|s| s.index == index)
        .map(|s| s.value.as_str())
        .unwrap_or("")
}

fn print_lsusb(devices: &[UsbDevice]) {
    for device in devices {
        let dd = &device.device_descriptor;
        let mfg = get_string(device, dd.i_manufacturer);
        let product = get_string(device, dd.i_product);
        let name = match (mfg.is_empty(), product.is_empty()) {
            (false, false) => format!("{mfg} {product}"),
            (false, true)  => mfg.to_string(),
            (true,  false) => product.to_string(),
            (true,  true)  => String::new(),
        };
        println!(
            "Bus {:03} Dev {:15}: ID {:04x}:{:04x}  {}",
            device.bus_number,
            device.location_id,
            dd.id_vendor,
            dd.id_product,
            name,
        );
        print_lsusb(&device.children);
    }
}

fn enumerate() -> Vec<UsbDevice> {
    #[cfg(target_os = "macos")]
    {
        usb_collector_macos::MacCollector::new()
            .enumerate()
            .unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); })
    }
    #[cfg(target_os = "linux")]
    {
        usb_collector_linux::LinuxCollector::new()
            .enumerate()
            .unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); })
    }
    #[cfg(target_os = "windows")]
    {
        usb_collector_windows::WindowsCollector::new()
            .enumerate()
            .unwrap_or_else(|e| { eprintln!("error: {e}"); std::process::exit(1); })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        eprintln!("USB enumeration not supported on this platform");
        vec![]
    }
}

