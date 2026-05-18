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

## Windows collector ✓ done

`crates/usb-collector-windows/src/lib.rs` — follows the macOS pattern using nusb.

**What works:**
- `nusb::list_devices()` enumerates all USB devices
- nusb reads device/config descriptors via Windows USB device interface (hub-level);
  works for all devices regardless of driver (WinUSB, HID.sys, usbstor, usbaudio, etc.)
- All devices: VID/PID, speed, manufacturer/product/serial strings, config descriptors
- `location_id` constructed from `{vid:04x}:{pid:04x}:{serial_or_port_chain}`
- HID report descriptors via `src/hid.rs`:
  - SetupDi enumerates all HID device interfaces (`GUID_DEVINTERFACE_HID`)
  - Opens each with `CreateFile` → `HidD_GetAttributes` for VID/PID
  - `HidD_GetSerialNumberString` for serial (used as map key)
  - `HidD_GetPreparsedData` → `HidP_GetCaps` / `HidP_GetButtonCaps` /
    `HidP_GetValueCaps` to enumerate all input/output/feature capabilities
  - Reconstructs a synthetic but valid HID report descriptor from those capabilities
  - Interface number parsed from device path (`MI_xx` segment)

**Approach rationale:**

The Windows kernel unconditionally overwrites the `bmRequest` field in
`IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION` to `0x80` (standard device
request), making it impossible to retrieve HID class descriptors (types 0x21,
0x22) via hub IOCTLs. `HidD_GetReportDescriptor` is kernel-mode only and not
exported from user-mode `hid.dll`. The preparsed-data approach (`hidapi` style)
is the correct user-mode path and works for all HID devices regardless of driver.

**Synthetic descriptor limitations:**
- Not byte-identical to the device's original descriptor
- Vendor-specific items are absent (not exposed via HidP_ APIs)
- Sub-collection nesting is flattened to a single Application collection
- Item ordering may differ from the original
- Despite these differences the output is valid HID and fully parseable

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

**HID output hierarchy** (matches USB Prober reference fixture):
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
                  ...
    Endpoint 0x84 - Interrupt Input
```
The formatter (`crates/usb-formatter`) and the GUI tree view both produce this
hierarchy. The `HID Report Descriptor` is nested inside `HID Descriptor` →
`Descriptor N`, not rendered as a separate top-level sibling.

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
- **Refresh** button + **Auto** toggle — manual re-enumeration or live hotplug via
  `nusb::watch_devices()` emitting a `usb-changed` Tauri event
- **Save Output** button (`Cmd+S`) — calls `format_as_text` Tauri command (Mac USB Prober-style
  text, same formatter as the CLI), shows native save dialog, writes `.txt`
- **Save JSON** button (`Cmd+Shift+S`) — serialises the loaded device list, writes `.json`
- **Row selection** — click or click-drag selects rows line-by-line (no character-level
  text selection); shift+click extends the range; Cmd+C copies selected rows as
  formatter-matched indented text; double-click or click the ▾/▸ arrow to expand/collapse
- Dark mode via CSS custom properties; all colors defined in `:root`, overridden
  once in `@media (prefers-color-scheme: dark)`
- Window: 1000 × 600 px; font: SF Mono 13px weight 500

**Tauri commands:**
- `enumerate_usb() -> Result<Vec<UsbDevice>, String>` — platform-dispatched
- `format_as_text(devices: Vec<UsbDevice>) -> String` — Mac USB Prober text format
- `write_text_file(path: String, content: String) -> Result<(), String>` — file write

Text formatter lives in `src-tauri/src/formatter.rs`; same logic duplicated in
`crates/usb-cli/src/main.rs` for the standalone CLI.

**Hotplug:** implemented via `nusb::watch_devices()` in a background thread that
emits `app.emit("usb-changed", ())` on every attach/detach event. The frontend
listens with `listen("usb-changed", ...)` when the Auto toggle is on.

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
7. ~~Hotplug (nusb `watch_devices` + Tauri `usb-changed` event + auto-refresh toggle).~~ ✓ done
8. ~~Windows basic enumeration (nusb).~~ ✓ done — hub IOCTLs + HID still TODO
9. ~~Class-specific descriptors (CS_INTERFACE/HID/IAD, CDC, Audio, MIDI).~~ ✓ done
10. ~~Row selection — click/drag line-by-line; Cmd+C copies formatter-matched text.~~ ✓ done

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
