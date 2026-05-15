# Project: USBProbester

Cross-platform Tauri app recreating Apple's old `USB Prober.app`:
hierarchical USB device tree with parsed Device descriptors,
Configuration descriptors, and HID Report descriptors.

See `PLAN.md` for the full architecture.

## Conventions

- Rust backend, [TODO: pick frontend framework — React / Svelte / Vue]
- Workspace layout:
  - `crates/usb-types` — shared data model
  - `crates/usb-collector-macos` — nusb + ioreg HID pass collector (cfg-gated)
  - `crates/usb-collector-linux` — `/sys`-based collector (cfg-gated)
  - `crates/usb-collector-windows` — ioctl-based collector (cfg-gated)
  - `crates/hid-parser` — platform-agnostic HID report descriptor parser
  - `src-tauri` — Tauri shell
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
6. Frontend tree + descriptor panels  ← **current focus**
7. Hotplug
8. Windows ioctls

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

## Useful commands

```bash
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

# Clean all build artifacts
npm run clean
```

## Current focus

Step 6: Frontend tree + descriptor panels. Steps 1–5 are all done.
