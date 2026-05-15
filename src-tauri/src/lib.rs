mod formatter;

#[cfg(target_os = "macos")]
use usb_collector_macos::MacCollector;
#[cfg(target_os = "linux")]
use usb_collector_linux::LinuxCollector;
use usb_types::UsbDevice;

#[tauri::command]
#[specta::specta]
fn enumerate_usb() -> Result<Vec<UsbDevice>, String> {
    #[cfg(target_os = "macos")]
    return MacCollector::new().enumerate().map_err(|e| e.to_string());
    #[cfg(target_os = "linux")]
    return LinuxCollector::new().enumerate().map_err(|e| e.to_string());
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Ok(vec![]);
}

#[tauri::command]
#[specta::specta]
fn format_as_text(devices: Vec<UsbDevice>) -> String {
    formatter::format_devices(&devices)
}

#[tauri::command]
#[specta::specta]
fn write_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

pub fn run() {
    let builder = tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            enumerate_usb,
            format_as_text,
            write_text_file,
        ]);

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            "../src/bindings.ts",
        )
        .expect("failed to export typescript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
