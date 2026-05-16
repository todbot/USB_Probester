# USBProbester — Architecture Plan

A cross-platform Tauri app that recreates the main functionality of Apple's
old `USB Prober.app`: a hierarchical tree of connected USB devices, with
parsed Device descriptors, Configuration descriptors, and HID Report
descriptors.

Starts on macOS (using `nusb` for device/config descriptors and `ioreg`
for HID report descriptors), with the backend factored so Linux and
Windows can be added without touching the frontend or the data model.

The output format we are matching is in
`tests/fixtures/usb-prober-reference0.txt` — a real USB Prober.app dump
that includes a hub, a composite CDC+HID+Audio device, and a parsed HID
Report Descriptor covering keyboard, mouse, and consumer-control
collections. The HID parser's output must visually match the "Parsed
Report Descriptor" block in that fixture.  
There is a similar output in `usb-prober-referencee1.txt`

---

## Architecture

A single normalized data model in Rust (`crates/usb-types/src/lib.rs`),
with per-platform "collectors" populating it. The frontend never sees
platform-specific shapes. Also derives `specta::Type` for automatic
TypeScript type generation via tauri-specta.

Key types (see crate for full definitions):
- `UsbDevice` — location_id (hex string), bus_number, port_path, speed, device_descriptor, configurations, strings, hid_interfaces, children
- `DeviceDescriptor` — standard USB fields + `raw_bytes: Vec<u8>` (18 bytes)
- `ConfigDescriptor` — standard fields + `raw_bytes: Vec<u8>` (full config blob)
- `InterfaceDescriptor` — class/subclass/protocol/endpoints
- `EndpointDescriptor` — address, attributes, max_packet_size, interval
- `HidInterface` — interface_number, raw_report_descriptor (bytes); `parsed` field TBD in step 3
- `UsbSpeed` — Low/Full/High/Super/SuperPlus/Unknown

A trait with cfg-gated impls:

```rust
pub trait UsbCollector {
    fn enumerate(&self) -> Result<Vec<UsbDevice>, CollectorError>;
}

#[cfg(target_os = "macos")]
pub use macos::MacCollector as PlatformCollector;
#[cfg(target_os = "linux")]
pub use linux::LinuxCollector as PlatformCollector;
#[cfg(target_os = "windows")]
pub use windows::WinCollector as PlatformCollector;
```

Tauri command is then trivial:

```rust
#[tauri::command]
fn enumerate_usb() -> Result<Vec<UsbDevice>, String> {
    PlatformCollector::default()
        .enumerate()
        .map_err(|e| e.to_string())
}
```

---

## macOS collector ✓ DONE

Two-pass approach in `crates/usb-collector-macos/src/lib.rs`:

**Pass 1 — `nusb` crate (primary enumerator):**
`ioreg -a -p IOUSB` does NOT expose raw config descriptor bytes — only
parsed field values. Instead, use the `nusb` crate (v0.2.3), which wraps
IOKit directly and returns raw descriptor bytes from `GetConfigurationDescriptorPtr()`.
No libusb dependency; no exclusive device open required for descriptor reads.

```rust
// nusb returns raw bytes on DeviceInfo without needing open()
for dev_info in nusb::list_devices().wait()? {
    let device = dev_info.open().wait()?;        // needed for device_descriptor()
    let raw_cfg = cfg.as_bytes().to_vec();       // full raw config blob ✓
    let raw_dev = device.device_descriptor().as_bytes().to_vec(); // 18 bytes ✓
}
```

**Pass 2 — ioreg HID pass:**
`IOHIDInterface` nodes do NOT appear in the IOUSB plane. Run a separate
`ioreg -a -c IOHIDInterface -l -r -w 0` and walk the plist for nodes
where `IOObjectClass = "IOHIDInterface"` AND `Transport = "USB"`. Extract
`ReportDescriptor` (Data blob) keyed by `LocationID` (capital L, u32).

Correlation key: `nusb`'s `dev_info.location_id()` == `LocationID` from
the IOHIDInterface node. Both are the same IOKit locationID u32 value
(`(bus << 24) | (port_chain_nibbles...)`).

**Known gaps vs USB Prober output:**
- Interface string descriptors not fetched (only manufacturer/product/serial
  come from `DeviceInfo`; per-interface strings need `device.get_string_descriptor()`)
- `@ N` in the header shows last port in port_chain, not USB device address
- Hub Descriptor, Device Qualifier, Other Speed Config sections omitted
  (need additional class-specific control transfers)
- HID parsed descriptor output blocked on step 3 (hid-parser crate)

**Examples:**
- `examples/dump_one.rs` — structured field dump
- `examples/prober_fmt.rs` — USB Prober-style text output matching `tests/fixtures/usb-prober-reference0.txt`

---

## Linux collector ✓ DONE

All data read from sysfs — **no device open, no elevated privileges required**:

```
/sys/bus/usb/devices/<bus>-<port>/
    descriptors             ← raw binary: device descriptor + all config blobs
    <bus>-<port>:<cfg>.<iface>/
        0003:<VID>:<PID>.<N>/
            report_descriptor  ← raw HID bytes (if HID interface)
```

Implementation in `crates/usb-collector-linux/`:
- `src/lib.rs` — `LinuxCollector::enumerate()` + `device_from_descriptor_bytes()` helper
- `src/descriptor.rs` — parses raw device/config/interface/endpoint descriptors
- `src/hid.rs` — walks sysfs looking for `0003:` subdirs and reads `report_descriptor`

`nusb::list_devices()` supplies metadata (busnum, port_chain, string caches, speed).
The `descriptors` sysfs file provides all raw descriptor bytes without opening the
device. `location_id` is set to the sysfs basename (e.g. `"2-4"`, `"2-2.3"`).

**Examples:**
- `examples/dump_one.rs` — live USB enumeration with HID parsing
- `examples/from_sysfs_file.rs` — parse a stored sysfs `descriptors` binary;
  falls back to a hardcoded blink(1) fixture when no path is given (no hardware needed)

---

## Windows collector ✓ basic done — full descriptors TODO

`crates/usb-collector-windows/src/lib.rs` — follows the macOS pattern using nusb.

**What works:**
- `nusb::list_devices()` enumerates all USB devices
- Devices with WinUSB driver loaded: full device descriptor + config descriptors via `dev_info.open()`
- All devices: VID/PID, speed, manufacturer/product/serial strings
- `location_id` constructed from `{vid:04x}:{pid:04x}:{serial_or_port_chain}`

**What's missing — the hard parts:**

*Full descriptors for class-driver devices:* HID keyboards/mice/webcams etc. are owned
by HID.sys and can't be opened via nusb/WinUSB. The canonical path (used by Microsoft's
USBView) is to go through the hub driver:
1. Enumerate hubs via `CreateFile(\\.\HCD0)` etc.
2. Per-port: `IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX`
3. Per-device: `IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION` → raw bytes

*HID report descriptors:* use `HidD_GetReportDescriptor` (hid.dll) via `windows-sys`.
Match devices to HID paths by VID/PID/serial or instance ID, similar to how macOS
uses ioreg LocationID correlation.

Microsoft's USBView source is the reference:
`github.com/microsoft/Windows-driver-samples/tree/main/usb/usbview`.
Add `windows-sys` with `Win32_Devices_HumanInterfaceDevice` + `Win32_Storage_FileSystem`
features. This is a real chunk of work — budget it as the bulk of completing the Windows port.

---

## HID report descriptor parser ✓ DONE

Custom item-stream walker in `crates/hid-parser/src/lib.rs`.

- `parse(&[u8]) -> Result<Vec<HidNode>, ParseError>` — reads byte stream,
  builds typed tree (Collection nodes contain their children)
- `render_text(&[HidNode], base_indent) -> String` — USB Prober-style text output
- `HidNode` and supporting types (`CollectionKind`, `HidIoFlags`) live in
  `crates/usb-types` so they serialize to JSON automatically via serde + specta
- Golden test in `crates/hid-parser/tests/golden_pico2_parsed.txt` — 156-byte
  Pico 2 descriptor rendered output matches reference fixture lines 199–276 exactly

Key implementation details:
- Logical/Physical Min/Max sign-extended from raw bytes (e.g. `0x81` → -127)
- Input with Variable (bit 1): show 8 flags; Input with Array: show 3 flags
- Output/Feature: always show 9 flags including Volatile (bit 7)
- Usage table is a minimal static match covering common pages; unknown usages
  fall back to `Usage N (0xN)` matching USB Prober behaviour

---

## Frontend (Tauri side) ✓ DONE

Two view modes toggled in the header:

**Tree view** — single-pane collapsible tree matching Mac USB Prober's layout.
Each device is a top-level `TreeNode`; children are Device Descriptor,
Configuration Descriptor (with interfaces/endpoints/HID inline), and
Number of Endpoints. Uses React context (`DepthCtx`) to track nesting depth
and compute label-area pixel width so the value column lands at a constant
absolute x-position at every nesting level.

**Split view** — left panel lists devices; right panel shows Device Descriptor /
Configuration / HID tabs for the selected device. Tab is preserved when
switching devices (unless the new device has no HID and "HID" was active).

UI features:
- **Refresh** button re-enumerates live devices
- **Save Output** button — calls `format_as_text` Tauri command (Mac USB Prober-style
  text, same formatter as the CLI), shows native macOS save dialog, writes `.txt`
- **Save JSON** button — serialises the already-loaded device list via
  `JSON.stringify`, shows native save dialog, writes `.json`
- Dark mode via CSS custom properties; all colors defined in `:root`, overridden
  once in `@media (prefers-color-scheme: dark)`
- Window: 1000 × 600 px; font: SF Mono 13px weight 500

**Tauri commands:**
- `enumerate_usb() -> Result<Vec<UsbDevice>, String>` — platform-dispatched
- `format_as_text(devices: Vec<UsbDevice>) -> String` — Mac USB Prober text format
- `write_text_file(path: String, content: String) -> Result<(), String>` — file write

Text formatter lives in `src-tauri/src/formatter.rs`; same logic duplicated in
`crates/usb-cli/src/main.rs` for the standalone CLI.

**Hotplug:** for live updates without polling, the platforms diverge.
- macOS: `IOServiceAddMatchingNotification` (use the `io-kit-sys` crate).
- Linux: open a netlink socket on `NETLINK_KOBJECT_UEVENT` or use `libudev`.
- Windows: `RegisterDeviceNotification` with `DBT_DEVTYP_DEVICEINTERFACE`.

Put each behind a `tokio::sync::mpsc` channel that emits to the Tauri
webview via `app.emit("usb-changed", ...)`. For v1, polling every 2
seconds is fine.

---

## CLI binary ✓ DONE

`crates/usb-cli` — standalone `usb-probester-cli` binary.

```bash
usb-probester-cli                  # Mac USB Prober-style text tree (default)
usb-probester-cli --format json    # pretty-printed JSON
cargo build --release -p usb-cli   # → target/release/usb-probester-cli
```

Depends on the same collector and parser crates as the Tauri app.
Platform dispatch mirrors `src-tauri/src/lib.rs`.

---

## Build order

1. ~~Define the `usb-types` crate.~~ ✓ done
2. ~~macOS collector via nusb + ioreg HID pass.~~ ✓ done
3. ~~`hid-parser` crate.~~ ✓ done
4. ~~Tauri wiring + basic frontend.~~ ✓ done
5. ~~Linux collector via `/sys`.~~ ✓ done
6. ~~Frontend tree + descriptor panels.~~ ✓ done
7. Hotplug. ← **next**
8. ~~Windows basic enumeration (nusb).~~ ✓ done — hub IOCTLs + HID still TODO

The design holds up because the report descriptor bytes are the same
bytes regardless of how they were obtained, AND the parser is fully
device-agnostic. The OS backends only need to produce `Vec<u8>` for each
descriptor type plus topology metadata.

---

## Practical tips

- **Pipe real `ioreg` output into the repo as test fixtures.**
  `ioreg -a -p IOUSB -l -w 0 > tests/fixtures/macos-mymachine.plist`.
  Same later for Linux (`cp -r /sys/bus/usb/devices tests/fixtures/linux-sysfs/`,
  carefully) and for Windows ioctl response captures.

- **Expand the HUT usage table as needed.** Currently a minimal static
  match covering pages used by the Pico 2 fixture. Unknown usages fall back
  to `Usage N (0xN)`. Grow the table when real devices expose gaps.
