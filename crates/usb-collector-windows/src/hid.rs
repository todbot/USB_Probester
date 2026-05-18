//! Windows HID report descriptor collection via SetupDi + USB hub IOCTL.
//!
//! Two-phase approach:
//! 1. SetupDi enumerates all HID device interfaces (keyboards, mice, gamepads, …),
//!    reads VID/PID/serial/interface-number, and obtains the CM DevInst.
//! 2. For each HID device, walks the device tree (CM_Get_Parent) to find the parent
//!    USB hub and port number, then sends
//!    `IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION` to the hub to retrieve the raw
//!    HID report descriptor bytes.  Falls back to an empty descriptor on any failure
//!    so the device still appears in the tree.
//!
//! Returns a `(vid, pid, serial) → Vec<HidInterface>` map so the caller can correlate
//! entries with nusb-enumerated devices.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use usb_types::HidInterface;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
    SP_DEVINFO_DATA, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_GetAttributes, HidD_GetReportDescriptor, HidD_GetSerialNumberString, HIDD_ATTRIBUTES,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

// ── Constants ──────────────────────────────────────────────────────────────────

const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;

// HDEVINFO is isize; the failure sentinel for SetupDiGetClassDevsW is -1.
const INVALID_HDEVINFO: HDEVINFO = -1isize;

// {4D1E55B2-F16F-11CF-88CB-001111000030}
const GUID_DEVINTERFACE_HID: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0x4D1E55B2,
    data2: 0xF16F,
    data3: 0x11CF,
    data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
};

// {F18A0E88-C30C-11D0-8815-00A0C906BED8}
const GUID_DEVINTERFACE_USB_HUB: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0xF18A0E88,
    data2: 0xC30C,
    data3: 0x11D0,
    data4: [0x88, 0x15, 0x00, 0xA0, 0xC9, 0x06, 0xBE, 0xD8],
};

// CTL_CODE(FILE_DEVICE_USB=0x22, 260, METHOD_BUFFERED=0, FILE_ANY_ACCESS=0)
// = (0x22 << 16) | (260 << 2) | 0 = 0x00220410
const IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION: u32 = 0x00220410;

// CM_DRP_ADDRESS: port number of the device on its parent hub (cfgmgr32.h, 0x1D).
const CM_DRP_ADDRESS: u32 = 0x1D;
const CR_SUCCESS: u32 = 0;
const CM_GET_DEVICE_INTERFACE_LIST_PRESENT: u32 = 0;

// Fixed header size of USB_DESCRIPTOR_REQUEST (ConnectionIndex + SetupPacket).
const USB_REQ_HDR: usize = 12;

// ── CM function declarations ────────────────────────────────────────────────────
// cfgmgr32.dll is always present on Windows; declare manually to avoid needing
// an additional windows-sys feature.

#[link(name = "cfgmgr32", kind = "raw-dylib")]
extern "system" {
    fn CM_Get_Parent(pdnDevInst: *mut u32, dnDevInst: u32, ulFlags: u32) -> u32;

    fn CM_Get_DevNode_Registry_PropertyW(
        dnDevInst: u32,
        ulProperty: u32,
        pulRegDataType: *mut u32,
        Buffer: *mut core::ffi::c_void,
        pulLength: *mut u32,
        ulFlags: u32,
    ) -> u32;

    fn CM_Get_Device_IDW(dnDevInst: u32, Buffer: *mut u16, BufferLen: u32, ulFlags: u32) -> u32;

    fn CM_Get_Device_Interface_List_SizeW(
        pulLen: *mut u32,
        InterfaceClassGuid: *const windows_sys::core::GUID,
        pDeviceID: *const u16,
        ulFlags: u32,
    ) -> u32;

    fn CM_Get_Device_Interface_ListW(
        InterfaceClassGuid: *const windows_sys::core::GUID,
        pDeviceID: *const u16,
        Buffer: *mut u16,
        BufferLen: u32,
        ulFlags: u32,
    ) -> u32;
}

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

// ── Per-interface processing ───────────────────────────────────────────────────

fn process_interface(
    hdev: HDEVINFO,
    iface_data: &mut SP_DEVICE_INTERFACE_DATA,
) -> Option<(HidKey, HidInterface)> {
    let (path, dev_inst) = get_device_path_and_instance(hdev, iface_data)?;
    let interface_number = parse_interface_number(&path);

    let handle = open_hid(&path);
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    let result = read_hid(handle, interface_number, dev_inst);
    unsafe { CloseHandle(handle) };
    result
}

fn get_device_path_and_instance(
    hdev: HDEVINFO,
    iface_data: &mut SP_DEVICE_INTERFACE_DATA,
) -> Option<(String, u32)> {
    // First call: get required buffer size.
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

    // Second call: fill detail AND devinfo (for DevInst).
    let mut devinfo: SP_DEVINFO_DATA = unsafe { core::mem::zeroed() };
    devinfo.cbSize = core::mem::size_of::<SP_DEVINFO_DATA>() as u32;

    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            hdev,
            iface_data,
            detail,
            required,
            core::ptr::null_mut(),
            &mut devinfo,
        )
    };
    if ok == 0 {
        return None;
    }

    let dev_inst = devinfo.DevInst;

    // DevicePath is null-terminated UTF-16 starting at byte offset 4.
    let path_ptr = unsafe { buf.as_ptr().add(4) as *const u16 };
    let path_len = (required as usize - 4) / 2;
    let slice = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };
    let end = slice.iter().position(|&c| c == 0).unwrap_or(path_len);
    Some((String::from_utf16_lossy(&slice[..end]), dev_inst))
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
    // Some devices require read+write.
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

fn read_hid(handle: HANDLE, interface_number: u8, dev_inst: u32) -> Option<(HidKey, HidInterface)> {
    let mut attrs: HIDD_ATTRIBUTES = unsafe { core::mem::zeroed() };
    attrs.Size = core::mem::size_of::<HIDD_ATTRIBUTES>() as u32;
    if unsafe { HidD_GetAttributes(handle, &mut attrs) } == 0 {
        return None;
    }

    let vid = attrs.VendorID;
    let pid = attrs.ProductID;
    let serial = get_serial(handle);

    // Try hub IOCTL (exact length, works on real hardware hubs).
    // Fall back to HidD_GetReportDescriptor (works even behind KVM / virtual hubs,
    // since it talks directly to HID.sys rather than through the USB hub).
    let raw = get_report_descriptor_via_hub(dev_inst, interface_number)
        .or_else(|| get_report_descriptor_via_hid_api(handle))
        .unwrap_or_default();
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

// ── HidD_GetReportDescriptor fallback ─────────────────────────────────────────

/// Retrieve the HID report descriptor bytes directly via HID.sys.
///
/// `HidD_GetReportDescriptor` succeeds when the buffer is >= the actual descriptor
/// length and writes the descriptor bytes into the buffer (trailing bytes remain
/// zero).  We probe ascending sizes and take the first success, then trim trailing
/// zeros; HID descriptors always end in End Collection (0xC0), so the last real
/// byte is non-zero.
fn get_report_descriptor_via_hid_api(handle: HANDLE) -> Option<Vec<u8>> {
    const PROBE_SIZES: &[u32] = &[32, 64, 128, 256, 512, 1024, 2048, 4096];
    for &size in PROBE_SIZES {
        let mut buf = vec![0u8; size as usize];
        let ok = unsafe {
            HidD_GetReportDescriptor(handle, buf.as_mut_ptr() as *mut core::ffi::c_void, size)
        };
        if ok != 0 {
            let actual = buf.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            if actual == 0 {
                return None;
            }
            buf.truncate(actual);
            return Some(buf);
        }
    }
    None
}

// ── Hub IOCTL path ─────────────────────────────────────────────────────────────

fn get_report_descriptor_via_hub(hid_dev_inst: u32, interface_num: u8) -> Option<Vec<u8>> {
    let (hub_path, port) = find_hub_and_port(hid_dev_inst)?;

    // Step 1: get HID class descriptor (type 0x21, 9 bytes) to read wDescriptorLength.
    let hid_cls = hub_request(&hub_path, port, 0x21, interface_num as u16, 9)?;
    if hid_cls.len() < 9 {
        return None;
    }
    // HID class descriptor layout (0-indexed):
    //   [0] bLength  [1] bDescriptorType=0x21  [2..3] bcdHID
    //   [4] bCountryCode  [5] bNumDescriptors
    //   [6] bDescriptorType (for report = 0x22)  [7..8] wDescriptorLength LE
    let report_len = u16::from_le_bytes([hid_cls[7], hid_cls[8]]);
    if report_len == 0 {
        return None;
    }

    // Step 2: get the HID report descriptor (type 0x22).
    hub_request(&hub_path, port, 0x22, interface_num as u16, report_len)
}

/// Walk up the device tree from the HID node to find the USB device's port
/// number on its parent hub, then return the hub's device interface path.
///
/// Strategy: at each level, try `probe_hub_interface_path` on the *parent*.
/// When it returns Some, the parent is a USB hub and the current node is the
/// USB device sitting on it; CM_DRP_ADDRESS on current gives the port number.
fn find_hub_and_port(hid_dev_inst: u32) -> Option<(String, u32)> {
    let mut current = hid_dev_inst;

    for _ in 0..6 {
        let mut parent: u32 = 0;
        if unsafe { CM_Get_Parent(&mut parent, current, 0) } != CR_SUCCESS {
            return None;
        }

        if let Some(hub_path) = probe_hub_interface_path(parent) {
            // parent is the hub; get port from current's CM_DRP_ADDRESS.
            let port = cm_address(current)?;
            return Some((hub_path, port));
        }

        current = parent;
    }
    None
}

fn cm_address(inst: u32) -> Option<u32> {
    let mut addr: u32 = 0;
    let mut size: u32 = 4;
    let cr = unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            inst,
            CM_DRP_ADDRESS,
            core::ptr::null_mut(),
            &mut addr as *mut u32 as *mut core::ffi::c_void,
            &mut size,
            0,
        )
    };
    if cr == CR_SUCCESS && addr > 0 { Some(addr) } else { None }
}

/// Try to get the device interface path for a node as a USB hub.
/// Returns None silently if the node is not a hub — used speculatively during tree walk.
fn probe_hub_interface_path(inst: u32) -> Option<String> {
    let mut id_buf = [0u16; 512];
    let cr = unsafe {
        CM_Get_Device_IDW(inst, id_buf.as_mut_ptr(), id_buf.len() as u32, 0)
    };
    if cr != CR_SUCCESS {
        return None;
    }

    let id_end = id_buf.iter().position(|&c| c == 0).unwrap_or(id_buf.len());
    let mut id_wide: Vec<u16> = id_buf[..id_end].to_vec();
    id_wide.push(0);

    let mut list_size: u32 = 0;
    let cr = unsafe {
        CM_Get_Device_Interface_List_SizeW(
            &mut list_size,
            &GUID_DEVINTERFACE_USB_HUB,
            id_wide.as_ptr(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };
    if cr != CR_SUCCESS || list_size <= 1 {
        return None;
    }

    let mut list_buf: Vec<u16> = vec![0u16; list_size as usize];
    let cr = unsafe {
        CM_Get_Device_Interface_ListW(
            &GUID_DEVINTERFACE_USB_HUB,
            id_wide.as_ptr(),
            list_buf.as_mut_ptr(),
            list_size,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };
    if cr != CR_SUCCESS {
        return None;
    }

    let path_end = list_buf.iter().position(|&c| c == 0).unwrap_or(list_buf.len());
    if path_end == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&list_buf[..path_end]))
}

/// Send a USB GET_DESCRIPTOR request to the hub for the device on `port`.
/// Returns the descriptor bytes (without the USB_DESCRIPTOR_REQUEST header).
fn hub_request(hub_path: &str, port: u32, desc_type: u8, w_index: u16, w_length: u16) -> Option<Vec<u8>> {
    let wide: Vec<u16> = hub_path.encode_utf16().chain(core::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    let result = send_hub_descriptor_request(handle, port, desc_type, w_index, w_length);
    unsafe { CloseHandle(handle) };
    result
}

fn send_hub_descriptor_request(
    hub: HANDLE,
    port: u32,
    desc_type: u8,
    w_index: u16,
    w_length: u16,
) -> Option<Vec<u8>> {
    // USB_DESCRIPTOR_REQUEST layout (12-byte fixed header + variable Data):
    //   [0..4]  ConnectionIndex (u32 LE) = port number
    //   [4]     bmRequest  = 0x81 (D→H | Standard | Interface)
    //   [5]     bRequest   = 0x06 (GET_DESCRIPTOR)
    //   [6..8]  wValue     = (desc_type << 8) | index (LE)
    //   [8..10] wIndex     = interface number (LE)
    //   [10..12] wLength   = max bytes to return (LE)
    //   [12..]  Data[]     (output filled by driver)
    let total = USB_REQ_HDR + w_length as usize;
    let mut buf = vec![0u8; total];

    buf[0..4].copy_from_slice(&port.to_le_bytes());
    buf[4] = 0x81; // D→H | Standard | Interface
    buf[5] = 0x06; // GET_DESCRIPTOR
    buf[6..8].copy_from_slice(&((desc_type as u16) << 8).to_le_bytes());
    buf[8..10].copy_from_slice(&w_index.to_le_bytes());
    buf[10..12].copy_from_slice(&w_length.to_le_bytes());

    let mut bytes_returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            hub,
            IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION,
            buf.as_ptr() as *const core::ffi::c_void,
            total as u32,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            total as u32,
            &mut bytes_returned,
            core::ptr::null_mut(),
        )
    };

    if ok != 0 && bytes_returned as usize > USB_REQ_HDR {
        Some(buf[USB_REQ_HDR..bytes_returned as usize].to_vec())
    } else {
        None
    }
}
