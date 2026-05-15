#[cfg(target_os = "macos")]
use usb_collector_macos::MacCollector;
use usb_types::UsbDevice;

#[tauri::command]
#[specta::specta]
fn enumerate_usb() -> Result<Vec<UsbDevice>, String> {
    #[cfg(target_os = "macos")]
    return MacCollector::new()
        .enumerate()
        .map_err(|e| e.to_string());

    #[cfg(not(target_os = "macos"))]
    return Ok(vec![]);
}

pub fn run() {
    let builder = tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![enumerate_usb]);

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            "../src/bindings.ts",
        )
        .expect("failed to export typescript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
