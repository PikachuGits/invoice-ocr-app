mod commands;
mod error;
mod html_parser;
mod invoice_extractor;
mod ocr_client;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::recognize_invoice,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
