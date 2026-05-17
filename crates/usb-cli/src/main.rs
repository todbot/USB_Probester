use clap::{Parser, ValueEnum};
use usb_types::{usb_class, *};

#[derive(Parser)]
#[command(name = "usb-probester-cli", about = "Dump USB device information")]
struct Cli {
    /// Output format
    #[arg(short, long, value_enum, default_value = "tree")]
    format: Format,
    /// Omit USB hubs from output
    #[arg(long)]
    hide_hubs: bool,
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
    match cli.format {
        Format::Tree => print!("{}", usb_formatter::format_devices(&devices)),
        Format::Json => println!("{}", serde_json::to_string_pretty(&devices).expect("serialize failed")),
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

