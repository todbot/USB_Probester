//! Windows HID report descriptor collection via SetupDi + HID API.
//!
//! Enumerates all HID device interfaces using SetupDi, opens each one, reads
//! VID/PID/serial/interface-number, and fetches the report descriptor via
//! `DeviceIoControl(IOCTL_HID_GET_REPORT_DESCRIPTOR)`.  Returns a map keyed by `(vid, pid, serial)`
//! so the caller can correlate entries with nusb-enumerated devices.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use usb_types::HidInterface;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_GetAttributes, HidD_GetSerialNumberString, HIDD_ATTRIBUTES,
};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

// GENERIC_READ/WRITE are not re-exported by Win32::Storage::FileSystem in this
// version of windows-sys — define them directly from the SDK headers.
const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;

// HidD_GetReportDescriptor does NOT exist in the user-mode hid.dll.
// The correct user-mode path is DeviceIoControl with IOCTL_HID_GET_REPORT_DESCRIPTOR.
// CTL_CODE(FILE_DEVICE_KEYBOARD=0xb, func=0, METHOD_NEITHER=3, FILE_ANY_ACCESS=0)
// = (0xb << 16) | (0 << 14) | (0 << 2) | 3 = 0x000b0003
const IOCTL_HID_GET_REPORT_DESCRIPTOR: u32 = 0x000b0003;

// {4D1E55B2-F16F-11CF-88CB-001111000030}
const GUID_DEVINTERFACE_HID: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0x4D1E55B2,
    data2: 0xF16F,
    data3: 0x11CF,
    data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
};

// HDEVINFO is isize in windows-sys; INVALID_HANDLE_VALUE is *mut c_void (-1).
// The sentinel value for a failed SetupDiGetClassDevsW is the same bit-pattern.
const INVALID_HDEVINFO: HDEVINFO = -1isize;

/// Map key matching `make_location_id()` logic in lib.rs.
pub type HidKey = (u16, u16, Option<String>); // (vid, pid, serial)

/// Walk all present HID device interfaces, open each, read VID/PID/serial and
/// the report descriptor, then return a `(vid, pid, serial) → interfaces` map.
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
            break; // ERROR_NO_MORE_ITEMS
        }

        if let Some((key, iface)) = process_interface(hdev, &mut iface_data) {
            map.entry(key).or_default().push(iface);
        }

        index += 1;
    }

    unsafe { SetupDiDestroyDeviceInfoList(hdev) };
    map
}

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

fn get_device_path(hdev: HDEVINFO, iface_data: &mut SP_DEVICE_INTERFACE_DATA) -> Option<String> {
    // First call with null detail just to get the required buffer size.
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

    // Allocate buffer, set cbSize at the start.
    let mut buf: Vec<u8> = vec![0u8; required as usize];
    let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
    unsafe {
        (*detail).cbSize = core::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
    }

    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            hdev,
            iface_data,
            detail,
            required,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }

    // DevicePath is a null-terminated UTF-16 string starting at byte offset 4 (after cbSize).
    let path_ptr = unsafe { buf.as_ptr().add(4) as *const u16 };
    let path_len = (required as usize - 4) / 2;
    let slice = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };
    let end = slice.iter().position(|&c| c == 0).unwrap_or(path_len);
    Some(String::from_utf16_lossy(&slice[..end]))
}

fn parse_interface_number(path: &str) -> u8 {
    // Typical path: \\?\hid#vid_046d&pid_c52b&mi_02#7&...
    // Split on common delimiters and look for the MI_ segment.
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
    // Some devices (e.g. gamepads) require read+write access.
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

    // Descriptor fetch may fail (see get_report_descriptor); still emit the
    // HidInterface so the device appears in the tree even without descriptor bytes.
    let raw = get_report_descriptor(handle).unwrap_or_default();
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

fn get_report_descriptor(handle: HANDLE) -> Option<Vec<u8>> {
    // IOCTL_HID_GET_REPORT_DESCRIPTOR (0x000b0003) is technically a kernel-to-
    // minidriver IOCTL, but hidclass.sys may respond to it from user mode too.
    // If it fails we log the Windows error code and return None; the caller
    // falls back to an empty descriptor so the device still appears in the tree.
    let mut buf = vec![0u8; 4096];
    let mut bytes_returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_HID_GET_REPORT_DESCRIPTOR,
            core::ptr::null(),
            0,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            buf.len() as u32,
            &mut bytes_returned,
            core::ptr::null_mut(),
        )
    };
    if ok != 0 && bytes_returned > 0 {
        buf.truncate(bytes_returned as usize);
        Some(buf)
    } else {
        let err = unsafe { GetLastError() };
        eprintln!("note: HID report descriptor IOCTL failed (ok={ok}, bytes={bytes_returned}, err={err:#010x})");
        None
    }
}
