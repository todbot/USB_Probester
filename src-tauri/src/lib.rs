mod formatter;

#[cfg(target_os = "macos")]
use usb_collector_macos::MacCollector;
#[cfg(target_os = "linux")]
use usb_collector_linux::LinuxCollector;
#[cfg(target_os = "windows")]
use usb_collector_windows::WindowsCollector;
use usb_types::UsbDevice;

use tauri::Emitter;
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};

#[tauri::command]
#[specta::specta]
fn enumerate_usb() -> Result<Vec<UsbDevice>, String> {
    #[cfg(target_os = "macos")]
    return MacCollector::new().enumerate().map_err(|e| e.to_string());
    #[cfg(target_os = "linux")]
    return LinuxCollector::new().enumerate().map_err(|e| e.to_string());
    #[cfg(target_os = "windows")]
    return WindowsCollector::new().enumerate().map_err(|e| e.to_string());
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
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

            let save_text = MenuItem::with_id(app, "save_text", "Save Output…", true, Some("CmdOrCtrl+S"))?;
            let save_json = MenuItem::with_id(app, "save_json", "Save JSON…", true, Some("CmdOrCtrl+Shift+S"))?;
            let file_menu = SubmenuBuilder::new(app, "File")
                .item(&save_text)
                .item(&save_json)
                .build()?;

            let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, Some("CmdOrCtrl+R"))?;
            let view_tree = MenuItem::with_id(app, "view_tree", "Tree View", true, None::<&str>)?;
            let view_split = MenuItem::with_id(app, "view_split", "Split View", true, None::<&str>)?;
            let view_menu = SubmenuBuilder::new(app, "View")
                .item(&refresh)
                .separator()
                .item(&view_tree)
                .item(&view_split)
                .build()?;

            #[cfg(target_os = "macos")]
            let app_submenu = SubmenuBuilder::new(app, "USB Probester")
                .about(None)
                .separator()
                .services()
                .separator()
                .hide()
                .hide_others()
                .show_all()
                .separator()
                .quit()
                .build()?;

            #[cfg(target_os = "macos")]
            let edit_menu = SubmenuBuilder::new(app, "Edit")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;

            #[cfg(target_os = "macos")]
            let window_menu = SubmenuBuilder::new(app, "Window")
                .minimize()
                .separator()
                .close_window()
                .build()?;

            #[cfg(target_os = "macos")]
            let menu = MenuBuilder::new(app)
                .item(&app_submenu)
                .item(&file_menu)
                .item(&view_menu)
                .item(&edit_menu)
                .item(&window_menu)
                .build()?;

            #[cfg(not(target_os = "macos"))]
            let menu = MenuBuilder::new(app)
                .item(&file_menu)
                .item(&view_menu)
                .build()?;

            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                app.emit("menu-action", event.id().as_ref()).ok();
            });

            // Hotplug watcher — emits "usb-changed" on any connect/disconnect.
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use futures_util::StreamExt;
                match nusb::watch_devices() {
                    Ok(watch) => {
                        let mut watch = Box::pin(watch);
                        while watch.next().await.is_some() {
                            app_handle.emit("usb-changed", ()).ok();
                        }
                    }
                    Err(e) => eprintln!("hotplug watch failed: {e}"),
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
