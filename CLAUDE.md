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
8. ~~Windows basic enumeration (nusb)~~ ✓ done — HID descriptors via HidD_GetPreparsedData reconstruction
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

# Windows — cross-compile x86_64 binary from ARM Windows build machine
rustup target add x86_64-pc-windows-msvc
cargo build --release -p usb-cli --target x86_64-pc-windows-msvc
# binary at target\x86_64-pc-windows-msvc\release\usb-probester-cli.exe

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
- nusb opens devices via `GUID_DEVINTERFACE_USB_DEVICE` (hub-level access); gives
  descriptor reads for ALL devices, not just WinUSB ones
- `claim_interface(n)` requires WinUSB on interface n; class-driver interfaces
  (HID.sys, usbstor, usbaudio, usbser) return "incompatible driver" and cannot be claimed
- `location_id` is `"{vid:04x}:{pid:04x}:{serial}"` or `"…:{port.chain}"` if no serial
- `bus_number` is always 0 (Windows doesn't expose it the same way)
- HID report descriptors via `src/hid.rs` — preparsed-data approach (hidapi style):
  - SetupDi enumerates all HID device interfaces (`GUID_DEVINTERFACE_HID`)
  - Opens each with `CreateFile` → `HidD_GetAttributes` for VID/PID/version
  - `HidD_GetSerialNumberString` for serial (map key: `(vid, pid, serial)`)
  - `parse_interface_number` extracts `MI_xx` from the device path for `interface_number`
  - `HidD_GetPreparsedData` → `HidP_GetCaps` / `HidP_GetButtonCaps` /
    `HidP_GetValueCaps` to enumerate capabilities for Input, Output, Feature report types
  - Reconstructs a synthetic but valid and parseable HID report descriptor
  - Works for all HID devices regardless of driver (HID.sys, WinUSB, etc.)

### Why not hub IOCTLs or control transfers

The Windows kernel unconditionally overwrites `bmRequest=0x80` in
`IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION`, making HID class descriptor
requests (types 0x21, 0x22) impossible. `HidD_GetReportDescriptor` is kernel-mode
only and not exported from user-mode `hid.dll`. The preparsed-data path is the
correct user-mode approach and is what `hidapi` uses.

### Synthetic descriptor limitations

- Not byte-identical to the device's original descriptor
- Vendor-specific items absent (not exposed via `HidP_` APIs)
- Sub-collection nesting flattened to a single Application collection
- Item ordering may differ from the original
- Output is valid HID and fully parseable by the `hid-parser` crate

### Cross-compilation from ARM Windows

```
rustup target add x86_64-pc-windows-msvc
cargo build --release -p usb-cli --target x86_64-pc-windows-msvc
```

## Text formatter and HID output hierarchy

`crates/usb-formatter/src/lib.rs` — the Mac USB Prober-style text renderer used by
both the Tauri "Save Output" command and the CLI binary.

HID report descriptors are nested inside the `HID Descriptor` block, matching the
USB Prober reference fixture:
```
Interface #N - HID
    HID Descriptor
        Descriptor Version Number:   0x0111
        Country Code:   0
        Descriptor Count:   1
        Descriptor 1
            Type:   0x22  (Report Descriptor)
            Length (and contents):   156
                Raw Descriptor (hex)    0000: ...
            Parsed Report Descriptor:
                  Usage Page    (Generic Desktop)
    Endpoint ...
```
The GUI tree view uses the same hierarchy via `ClassSpecificNode` in `App.tsx`.

## Current focus

All planned steps done. No blocking code issues remain.
