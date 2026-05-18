//! Windows HID report descriptor collection via SetupDi + HID preparsed data.
//!
//! SetupDi enumerates all HID device interfaces (keyboards, mice, gamepads, …);
//! for each, opens the device, calls `HidD_GetPreparsedData`, then uses the
//! public `HidP_Get*Caps` APIs to enumerate button and value capabilities.
//! A synthetic HID report descriptor is reconstructed from those capabilities.
//!
//! **Limitations of the reconstructed descriptor:**
//! - Not byte-for-byte identical to the device's original descriptor.
//! - Vendor-specific items are absent (not exposed via HidP_ APIs).
//! - Sub-collection nesting is flattened to a single Application collection.
//! - Item ordering may differ from the original.
//! - Despite these differences, the output is valid HID and fully parseable
//!   by the hid-parser crate.
//!
//! Returns a `(vid, pid, serial) → Vec<HidInterface>` map that the caller can
//! correlate with nusb-enumerated devices.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use usb_types::HidInterface;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetPreparsedData, HidD_GetSerialNumberString,
    HidP_GetButtonCaps, HidP_GetCaps, HidP_GetValueCaps, HIDD_ATTRIBUTES, HIDP_BUTTON_CAPS,
    HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, HidP_Feature, HidP_Input, HidP_Output,
    PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

// ── Constants ──────────────────────────────────────────────────────────────────

const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;
const INVALID_HDEVINFO: HDEVINFO = -1isize;

// {4D1E55B2-F16F-11CF-88CB-001111000030}
const GUID_DEVINTERFACE_HID: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0x4D1E55B2,
    data2: 0xF16F,
    data3: 0x11CF,
    data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
};

// ── Public types ───────────────────────────────────────────────────────────────

/// Map key matching `make_location_id()` logic in lib.rs.
pub type HidKey = (u16, u16, Option<String>); // (vid, pid, serial)

// ── Public entry point ─────────────────────────────────────────────────────────

pub fn collect_hid_descriptors() -> HashMap<HidKey, Vec<HidInterface>> {
    let mut map: HashMap<HidKey, Vec<HidInterface>> = HashMap::new();

    let hdev = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_HID,
            core::ptr::null(),
            core::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if hdev == INVALID_HDEVINFO {
        return map;
    }

    let mut index = 0u32;
    loop {
        let mut iface_data: SP_DEVICE_INTERFACE_DATA = unsafe { core::mem::zeroed() };
        iface_data.cbSize = core::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;

        let ok = unsafe {
            SetupDiEnumDeviceInterfaces(
                hdev,
                core::ptr::null_mut(),
                &GUID_DEVINTERFACE_HID,
                index,
                &mut iface_data,
            )
        };
        if ok == 0 {
            break;
        }

        if let Some((key, iface)) = process_interface(hdev, &mut iface_data) {
            map.entry(key).or_default().push(iface);
        }

        index += 1;
    }

    unsafe { SetupDiDestroyDeviceInfoList(hdev) };
    map
}

// ── Per-interface processing ───────────────────────────────────────────────────

fn process_interface(
    hdev: HDEVINFO,
    iface_data: &mut SP_DEVICE_INTERFACE_DATA,
) -> Option<(HidKey, HidInterface)> {
    let path = get_device_path(hdev, iface_data)?;
    let interface_number = parse_interface_number(&path);

    let handle = open_hid(&path);
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    let result = read_hid(handle, interface_number);
    unsafe { CloseHandle(handle) };
    result
}

fn get_device_path(
    hdev: HDEVINFO,
    iface_data: &mut SP_DEVICE_INTERFACE_DATA,
) -> Option<String> {
    // First call: query required buffer size.
    let mut required: u32 = 0;
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            hdev,
            iface_data,
            core::ptr::null_mut(),
            0,
            &mut required,
            core::ptr::null_mut(),
        )
    };
    if required < 5 {
        return None;
    }

    let mut buf: Vec<u8> = vec![0u8; required as usize];
    let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
    unsafe {
        (*detail).cbSize = core::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
    }

    // Second call: fill the detail buffer.
    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            hdev,
            iface_data,
            detail,
            required,
            core::ptr::null_mut(),
            core::ptr::null_mut(), // DeviceInfoData not needed
        )
    };
    if ok == 0 {
        return None;
    }

    // DevicePath is null-terminated UTF-16 starting at byte offset 4.
    let path_ptr = unsafe { buf.as_ptr().add(4) as *const u16 };
    let path_len = (required as usize - 4) / 2;
    let slice = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };
    let end = slice.iter().position(|&c| c == 0).unwrap_or(path_len);
    Some(String::from_utf16_lossy(&slice[..end]))
}

fn parse_interface_number(path: &str) -> u8 {
    // Typical path: \\?\hid#vid_046d&pid_c52b&mi_02#7&...
    let lower = path.to_ascii_lowercase();
    for segment in lower.split(['&', '#', '\\']) {
        if let Some(hex) = segment.strip_prefix("mi_") {
            if let Ok(n) = u8::from_str_radix(hex, 16) {
                return n;
            }
        }
    }
    0
}

fn open_hid(path: &str) -> HANDLE {
    let wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if h != INVALID_HANDLE_VALUE {
        return h;
    }
    // Some devices require read+write access.
    unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    }
}

fn read_hid(handle: HANDLE, interface_number: u8) -> Option<(HidKey, HidInterface)> {
    let mut attrs: HIDD_ATTRIBUTES = unsafe { core::mem::zeroed() };
    attrs.Size = core::mem::size_of::<HIDD_ATTRIBUTES>() as u32;
    if unsafe { HidD_GetAttributes(handle, &mut attrs) } == 0 {
        return None;
    }

    let vid = attrs.VendorID;
    let pid = attrs.ProductID;
    let serial = get_serial(handle);

    let raw = reconstruct_descriptor(handle).unwrap_or_default();
    let parsed = if raw.is_empty() { None } else { hid_parser::parse(&raw).ok() };

    Some((
        (vid, pid, serial),
        HidInterface { interface_number, raw_report_descriptor: raw, parsed },
    ))
}

fn get_serial(handle: HANDLE) -> Option<String> {
    let mut buf = [0u16; 256];
    let ok = unsafe {
        HidD_GetSerialNumberString(
            handle,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            (buf.len() * 2) as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..end]);
    if s.is_empty() { None } else { Some(s) }
}

// ── Descriptor reconstruction via preparsed data ────────────────────────────────

/// Calls HidD_GetPreparsedData and reconstructs a synthetic descriptor from
/// the public HidP_ capability APIs.
fn reconstruct_descriptor(handle: HANDLE) -> Option<Vec<u8>> {
    let mut pp_data: PHIDP_PREPARSED_DATA = 0;
    if unsafe { HidD_GetPreparsedData(handle, &mut pp_data) } == 0 || pp_data == 0 {
        return None;
    }
    let result = build_from_caps(pp_data);
    unsafe { HidD_FreePreparsedData(pp_data) };
    result
}

fn build_from_caps(pp: PHIDP_PREPARSED_DATA) -> Option<Vec<u8>> {
    let mut caps: HIDP_CAPS = unsafe { core::mem::zeroed() };
    if unsafe { HidP_GetCaps(pp, &mut caps) } != HIDP_STATUS_SUCCESS {
        return None;
    }

    let mut buf = Vec::<u8>::new();

    // Device-level usage wrapper
    push_usage_page(&mut buf, caps.UsagePage);
    push_usage(&mut buf, caps.Usage);
    buf.extend_from_slice(&[0xA1, 0x01]); // Collection (Application)

    for &report_type in &[HidP_Input, HidP_Output, HidP_Feature] {
        let (n_btn, n_val) = match report_type {
            t if t == HidP_Input  => (caps.NumberInputButtonCaps,   caps.NumberInputValueCaps),
            t if t == HidP_Output => (caps.NumberOutputButtonCaps,  caps.NumberOutputValueCaps),
            _                     => (caps.NumberFeatureButtonCaps, caps.NumberFeatureValueCaps),
        };

        // HID main-item prefix byte: tag=Input(8)/Output(9)/Feature(11), type=Main(0), size=1byte
        let main_prefix: u8 = match report_type {
            t if t == HidP_Input  => 0x81,
            t if t == HidP_Output => 0x91,
            _                     => 0xB1,
        };

        let btn_caps = fetch_button_caps(pp, report_type, n_btn);
        let val_caps = fetch_value_caps(pp, report_type, n_val);

        let mut last_rid: Option<u8> = None;

        for bc in &btn_caps {
            if bc.ReportID != 0 && Some(bc.ReportID) != last_rid {
                buf.extend_from_slice(&[0x85, bc.ReportID]); // Report ID
                last_rid = Some(bc.ReportID);
            }
            push_usage_page(&mut buf, bc.UsagePage);

            // Per MSDN, for IsRange=TRUE the count is UsageMax-UsageMin+1.
            // For IsRange=FALSE, ReportCount is 1 (or >1 only for a usage array).
            let count: u16 = if bc.IsRange != 0 {
                let r = unsafe { bc.Anonymous.Range };
                push_usage_min(&mut buf, r.UsageMin);
                push_usage_max(&mut buf, r.UsageMax);
                r.UsageMax.saturating_sub(r.UsageMin).saturating_add(1)
            } else {
                let nr = unsafe { bc.Anonymous.NotRange };
                push_usage(&mut buf, nr.Usage);
                bc.ReportCount.max(1)
            };

            buf.extend_from_slice(&[0x15, 0x00]); // Logical Minimum (0)
            buf.extend_from_slice(&[0x25, 0x01]); // Logical Maximum (1)
            buf.extend_from_slice(&[0x75, 0x01]); // Report Size (1 bit per button)
            push_count(&mut buf, count);
            buf.extend_from_slice(&[main_prefix, (bc.BitField & 0xFF) as u8]);
        }

        for vc in &val_caps {
            if vc.ReportID != 0 && Some(vc.ReportID) != last_rid {
                buf.extend_from_slice(&[0x85, vc.ReportID]); // Report ID
                last_rid = Some(vc.ReportID);
            }
            push_usage_page(&mut buf, vc.UsagePage);

            if vc.IsRange != 0 {
                let r = unsafe { vc.Anonymous.Range };
                push_usage_min(&mut buf, r.UsageMin);
                push_usage_max(&mut buf, r.UsageMax);
            } else {
                let nr = unsafe { vc.Anonymous.NotRange };
                push_usage(&mut buf, nr.Usage);
            }

            push_logical_min(&mut buf, vc.LogicalMin);
            push_logical_max(&mut buf, vc.LogicalMax);
            push_size(&mut buf, vc.BitSize);
            push_count(&mut buf, vc.ReportCount);
            buf.extend_from_slice(&[main_prefix, (vc.BitField & 0xFF) as u8]);
        }
    }

    buf.push(0xC0); // End Collection

    // 7 = minimum wrapper with no caps: UsagePage(2)+Usage(2)+Collection(2)+EndCollection(1)
    if buf.len() <= 7 { None } else { Some(buf) }
}

fn fetch_button_caps(pp: PHIDP_PREPARSED_DATA, report_type: i32, n: u16) -> Vec<HIDP_BUTTON_CAPS> {
    if n == 0 {
        return Vec::new();
    }
    let mut caps: Vec<HIDP_BUTTON_CAPS> = vec![unsafe { core::mem::zeroed() }; n as usize];
    let mut len = n;
    unsafe { HidP_GetButtonCaps(report_type, caps.as_mut_ptr(), &mut len, pp) };
    caps.truncate(len as usize);
    caps
}

fn fetch_value_caps(pp: PHIDP_PREPARSED_DATA, report_type: i32, n: u16) -> Vec<HIDP_VALUE_CAPS> {
    if n == 0 {
        return Vec::new();
    }
    let mut caps: Vec<HIDP_VALUE_CAPS> = vec![unsafe { core::mem::zeroed() }; n as usize];
    let mut len = n;
    unsafe { HidP_GetValueCaps(report_type, caps.as_mut_ptr(), &mut len, pp) };
    caps.truncate(len as usize);
    caps
}

// ── HID short-item encoding helpers ───────────────────────────────────────────

fn push_usage_page(buf: &mut Vec<u8>, page: u16) {
    if page <= 0xFF {
        buf.extend_from_slice(&[0x05, page as u8]);
    } else {
        buf.push(0x06);
        buf.extend_from_slice(&page.to_le_bytes());
    }
}

fn push_usage(buf: &mut Vec<u8>, usage: u16) {
    if usage <= 0xFF {
        buf.extend_from_slice(&[0x09, usage as u8]);
    } else {
        buf.push(0x0A);
        buf.extend_from_slice(&usage.to_le_bytes());
    }
}

fn push_usage_min(buf: &mut Vec<u8>, usage: u16) {
    if usage <= 0xFF {
        buf.extend_from_slice(&[0x19, usage as u8]);
    } else {
        buf.push(0x1A);
        buf.extend_from_slice(&usage.to_le_bytes());
    }
}

fn push_usage_max(buf: &mut Vec<u8>, usage: u16) {
    if usage <= 0xFF {
        buf.extend_from_slice(&[0x29, usage as u8]);
    } else {
        buf.push(0x2A);
        buf.extend_from_slice(&usage.to_le_bytes());
    }
}

fn push_logical_min(buf: &mut Vec<u8>, val: i32) {
    if val >= -128 && val <= 127 {
        buf.extend_from_slice(&[0x15, val as i8 as u8]);
    } else if val >= -32768 && val <= 32767 {
        buf.push(0x16);
        buf.extend_from_slice(&(val as i16).to_le_bytes());
    } else {
        buf.push(0x17);
        buf.extend_from_slice(&val.to_le_bytes());
    }
}

fn push_logical_max(buf: &mut Vec<u8>, val: i32) {
    if val >= -128 && val <= 127 {
        buf.extend_from_slice(&[0x25, val as i8 as u8]);
    } else if val >= -32768 && val <= 32767 {
        buf.push(0x26);
        buf.extend_from_slice(&(val as i16).to_le_bytes());
    } else {
        buf.push(0x27);
        buf.extend_from_slice(&val.to_le_bytes());
    }
}

fn push_size(buf: &mut Vec<u8>, size: u16) {
    if size <= 0xFF {
        buf.extend_from_slice(&[0x75, size as u8]);
    } else {
        buf.push(0x76);
        buf.extend_from_slice(&size.to_le_bytes());
    }
}

fn push_count(buf: &mut Vec<u8>, count: u16) {
    if count <= 0xFF {
        buf.extend_from_slice(&[0x95, count as u8]);
    } else {
        buf.push(0x96);
        buf.extend_from_slice(&count.to_le_bytes());
    }
}
