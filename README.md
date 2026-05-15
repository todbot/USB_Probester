# USBProbester

A cross-platform desktop app for exploring connected USB devices — their
device descriptors, configuration descriptors, interface and endpoint
details, and parsed HID report descriptors. Spiritual successor to Apple's
`USB Prober.app`.

Built with [Tauri v2](https://tauri.app) (Rust backend, React/TypeScript frontend).

---

## Why does this exist?

Apple shipped `USB Prober.app` as part of the **Hardware IO Tools for Xcode**
developer tools package. It was invaluable for USB device development: it showed
the full descriptor tree for every attached device, including the raw bytes of
configuration descriptors and a parsed rendering of HID report descriptors.

Apple quietly dropped it. The last version stopped working on modern macOS
(Apple Silicon / macOS 12+), the download was removed from the developer portal,
and no replacement was provided. 

USBProbester aims to fill that gap with a native, cross-platform tool that
shows the same level of detail USB Prober did — and eventually more.

The reference fixture files in `tests/fixtures/usb-prober-reference*.txt` are
real output captures from the original USB Prober, used as the ground truth for
output format and correctness.

---

## Status

| Platform | Collector | HID Parser | Frontend UI |
|----------|-----------|------------|-------------|
| macOS    | ✓ working | ✓ working  | in progress |
| Linux    | planned   | ✓ shared   | in progress |
| Windows  | planned   | ✓ shared   | in progress |

---

## Workspace layout

```
crates/
  usb-types/            — shared Rust data model (serde + specta types)
  usb-collector-macos/  — macOS USB enumeration via nusb + ioreg HID pass
  hid-parser/           — platform-agnostic HID report descriptor parser
src-tauri/              — Tauri shell and backend commands
src/                    — React/TypeScript frontend (in progress)
tests/fixtures/         — reference output from original USB Prober.app
```

---

## Prerequisites

- **Rust** — [rustup.rs](https://rustup.rs)
- **Node.js** — v18 or later
- **Tauri CLI** — installed via npm (see below)

---

## Building

```bash
# Install JS dependencies
npm install

# Build all Rust crates
cargo build

# Run the Tauri dev server (hot-reload frontend + Rust backend)
npm run tauridev

# Build a release app bundle
npm run tauribuild
```

---

## Running the collectors (macOS, no UI needed)

Two examples let you validate the collector and parser against real hardware
without launching the full app:

```bash
# Structured field dump — shows raw descriptor bytes, endpoint table, HID hex
cargo run -p usb-collector-macos --example dump_one

# USB Prober-style text output — matches the reference fixture format
cargo run -p usb-collector-macos --example prober_fmt
```

---

## Tests

```bash
# Run all tests across the workspace
cargo test

# HID parser golden test only
# Parses the 156-byte Pico 2 HID descriptor and asserts the rendered output
# matches tests/fixtures/usb-prober-reference0.txt lines 199–276 exactly.
cargo test -p hid-parser

# Clean all build artifacts
npm run clean
```

---

## How the macOS collector works

USB enumeration uses two passes:

1. **`nusb` crate** — wraps IOKit directly (no libusb) to enumerate devices
   and retrieve raw descriptor bytes (`GET_DESCRIPTOR(DEVICE)` and
   `GET_DESCRIPTOR(CONFIGURATION)`). `ioreg -a -p IOUSB` only exposes parsed
   field values, not raw bytes, so nusb is necessary for the hex dump display.

2. **`ioreg -a -c IOHIDInterface`** — HID report descriptor bytes live on
   `IOHIDInterface` nodes, not on the USB device node. A separate ioreg pass
   extracts them and correlates them to the nusb device via `LocationID`.

---

## Reference links

### Standards

- [USB 2.0 Specification](https://www.usb.org/document-library/usb-20-specification) — device/config/interface/endpoint descriptor formats
- [HID Usage Tables 1.5 (HUT)](https://usb.org/document-library/hid-usage-tables-15) — usage page and usage name lookup
- [HID Class Specification 1.11](https://www.usb.org/document-library/device-class-definition-hid-111) — report descriptor item stream format

### Rust crates

- [nusb](https://crates.io/crates/nusb) — pure-Rust USB library wrapping IOKit on macOS and WinUSB on Windows
- [Tauri v2](https://tauri.app) — Rust + web frontend desktop app framework
- [specta](https://github.com/oscartbeaumont/specta) + [tauri-specta](https://github.com/oscartbeaumont/tauri-specta) — automatic TypeScript type generation from Rust types
- [plist](https://crates.io/crates/plist) — plist parsing for ioreg XML output

### Reference implementations

- [Apple Hardware IO Tools (archived)](https://developer.apple.com/download/all/?q=hardware%20io%20tools) — the original USB Prober
- [Microsoft USBView](https://github.com/microsoft/Windows-driver-samples/tree/main/usb/usbview) — reference for Windows IOCTL-based USB enumeration
- [Linux `/sys/bus/usb`](https://www.kernel.org/doc/html/latest/driver-api/usb/usb.html) — sysfs USB device tree used by the planned Linux collector


### Related tools

- [lsusb for Linux](https://linux.wiki/docs/commands/system-info/lsusb/) - Linux, no HID report descriptor usually
- [lsusb for Mac OS X](https://github.com/jlhonora/lsusb) - MacOS only and only partial data
- [USBDeview](https://www.nirsoft.net/utils/usb_devices_view.html) - Windows-only
- [USB Device Tree Viewer](https://www.uwe-sieber.de/usbtreeview_e.html) - Windows-only
