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

## Windows collector

The hard one. There's no equivalent shell tool that prints descriptors.
The canonical path is the same one Microsoft's open-source USBView uses:

1. Open each USB hub via `CreateFile(\\.\HCD0)` etc.
2. For each port, send `IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX` to
   discover the device.
3. Send `IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION` to fetch device,
   config, and string descriptors as raw bytes.
4. For HID, `HidD_GetPreparsedData` + `HidP_GetCaps` gives parsed; for
   the raw report descriptor, use `IOCTL_HID_GET_COLLECTION_DESCRIPTOR`.

Microsoft's USBView source is the reference:
`github.com/microsoft/Windows-driver-samples/tree/main/usb/usbview`.
Port the relevant ioctls to Rust using the `windows` crate. This is a
real chunk of work — budget it as the bulk of the Windows port.

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

## Frontend (Tauri side)

Invoke once on load, then on a "Refresh" button or on a USB hotplug
signal (see below). Render with a tree component — `react-arborist` if
React, or build a custom one with a `<details>`/`<summary>` cascade since
the data is hierarchical and virtualization isn't needed unless someone
has 200 USB devices. Right-hand panel switches on selection between
Device Descriptor / Configuration Descriptor(s) / HID Report Descriptor
tabs.

For the HID Report Descriptor view, render the same tree the parser
produces — collapsible Collections, monospace, with the option to also
show the raw hex dump above the parsed tree (USB Prober shows both —
fixture lines 187–197 are the hex, 198–276 the parsed tree).

**Hotplug:** for live updates without polling, the platforms diverge.
- macOS: `IOServiceAddMatchingNotification` (use the `io-kit-sys` crate).
- Linux: open a netlink socket on `NETLINK_KOBJECT_UEVENT` or use `libudev`.
- Windows: `RegisterDeviceNotification` with `DBT_DEVTYP_DEVICEINTERFACE`.

Put each behind a `tokio::sync::mpsc` channel that emits to the Tauri
webview via `app.emit("usb-changed", ...)`. For v1, polling every 2
seconds is fine.

---

## Build order

1. ~~Define the `usb-types` crate.~~ ✓ done
2. ~~macOS collector via nusb + ioreg HID pass.~~ ✓ done
3. ~~`hid-parser` crate.~~ ✓ done
4. ~~Tauri wiring + basic frontend.~~ ✓ done
5. ~~Linux collector via `/sys`.~~ ✓ done
6. Frontend tree + descriptor panels. ← **current focus**
7. Hotplug.
8. Windows via ioctls — the slog.

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
