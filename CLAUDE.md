# Project: USBProbester

Cross-platform Tauri app recreating Apple's old `USB Prober.app`:
hierarchical USB device tree with parsed Device descriptors,
Configuration descriptors, and HID Report descriptors.

See `PLAN.md` for the full architecture.

## Conventions

- Rust backend, React/TypeScript frontend (Tauri v2)
- Workspace layout:
  - `crates/usb-types` — shared data model (serde + specta types)
  - `crates/usb-collector-macos` — nusb + ioreg HID pass collector (cfg-gated)
  - `crates/usb-collector-linux` — `/sys`-based collector (cfg-gated)
  - `crates/usb-collector-windows` — nusb-based collector (cfg-gated); partial descriptor support
  - `crates/hid-parser` — platform-agnostic HID report descriptor parser
  - `crates/usb-cli` — standalone CLI binary (`usb-probester-cli`)
  - `src-tauri` — Tauri shell, backend commands, text formatter
- Platform code behind `cfg` gates; frontend never sees platform-specific shapes.
- Prefer parsing raw descriptor bytes over scraping pretty-printed tool output.
- Capture real OS output as test fixtures in `tests/fixtures/` so unit tests
  don't depend on attached hardware.

## Build order (from PLAN.md)

1. ~~`usb-types` crate~~ ✓ done
2. ~~macOS collector (nusb + ioreg HID pass)~~ ✓ done
3. ~~`hid-parser` crate~~ ✓ done
4. ~~Tauri wiring + basic frontend~~ ✓ done
5. ~~Linux collector (`/sys`)~~ ✓ done
6. ~~Frontend tree + descriptor panels~~ ✓ done
7. ~~Hotplug (nusb watch_devices + auto-refresh toggle)~~ ✓ done
8. ~~Windows basic enumeration (nusb)~~ ✓ done — HID descriptors + full config access via hub IOCTLs still TODO
9. ~~Class-specific descriptors (CS_INTERFACE/HID/IAD)~~ ✓ done — CDC, Audio, MIDI decoded; generic hex fallback for unknowns
10. ~~Row selection~~ ✓ done — click/drag selects rows line-by-line; Cmd+C copies formatter-matched text

## Platform gating

Both collector crates are unconditional workspace members. They use a
top-level `#![cfg(target_os = "…")]` to become no-ops on the wrong OS,
avoiding the need for conditional workspace membership. `src-tauri/Cargo.toml`
declares each under `[target.'cfg(…)'.dependencies]`.

## Linux collector notes

Reads everything from sysfs — no device open, no elevated privileges:
- `nusb::list_devices()` for metadata (busnum, port_chain, strings, speed)
- `descriptors` sysfs file for raw USB descriptor bytes; parsed in `src/descriptor.rs`
- HID report descriptors from `<dev>/<dev>:<cfg>.<iface>/0003:<VID>:<PID>.<N>/report_descriptor`
- `location_id` is the sysfs basename (e.g. `"2-4"`, `"2-2.3"`)

## Text formatter

`src-tauri/src/formatter.rs` contains the Mac USB Prober-style text renderer,
shared by both the Tauri "Save Output" command and the CLI binary.
The same logic also lives in `crates/usb-cli/src/main.rs` (standalone copy
for the CLI; these should be kept in sync if the format changes).

## Useful commands

```bash
# CLI — USB Prober-style text dump
cargo run -p usb-cli

# CLI — JSON dump
cargo run -p usb-cli -- --format json

# CLI — standalone release binary
cargo build --release -p usb-cli
# binary at target/release/usb-probester-cli

# Linux — live USB enumeration
cargo run -p usb-collector-linux --example dump_one

# Linux — parse stored sysfs descriptors binary (built-in blink(1) fixture)
cargo run -p usb-collector-linux --example from_sysfs_file

# Linux — parse a real sysfs descriptors file
cargo run -p usb-collector-linux --example from_sysfs_file -- /sys/bus/usb/devices/2-4/descriptors

# macOS — structured dump
cargo run -p usb-collector-macos --example dump_one

# macOS — USB Prober-style text output
cargo run -p usb-collector-macos --example prober_fmt

# Build everything
cargo build

# Run Tauri dev server
npm run tauridev

# Build release app bundle
npm run tauribuild

# Clean all build artifacts
npm run clean
```

## Windows collector notes

`crates/usb-collector-windows/src/lib.rs` — nusb-based, cfg-gated with `#![cfg(target_os = "windows")]`.

- `nusb::list_devices()` for metadata (port_chain, strings, speed)
- Tries `dev_info.open()` for full descriptor access (works for WinUSB devices)
- Falls back to partial info (VID/PID/strings/speed only) for devices owned by HID.sys,
  usbstor, CDC, etc. — their class driver blocks nusb from opening them
- `location_id` is `"{vid:04x}:{pid:04x}:{serial}"` or `"…:{port.chain}"` if no serial
- `bus_number` is always 0 (Windows doesn't expose it the same way)
- HID report descriptors collected via SetupDi + `HidD_GetReportDescriptor` in `src/hid.rs`;
  two-pass strategy mirrors the macOS collector (nusb pass + HID pass keyed by vid/pid/serial)
- Full config descriptor access for non-WinUSB devices needs `IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION`

## Current focus

All planned steps done. Remaining work:
- Windows full config descriptor access for non-WinUSB devices (hub IOCTLs)
