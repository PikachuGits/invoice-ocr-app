mod commands;
mod db;
mod error;
mod exporter;
mod html_parser;
mod invoice_extractor;
mod ocr_client;
mod pdf;

use db::Database;
use std::path::PathBuf;

fn default_db_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("invoice-ocr-app");
    std::fs::create_dir_all(&path).ok();
    path.push("invoices.db");
    path
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = default_db_path();
    let db = Database::new(db_path.to_str().unwrap()).expect("Failed to init DB");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(db)
        .invoke_handler(tauri::generate_handler![
            commands::recognize_invoice,
            commands::re_recognize_invoice,
            commands::get_invoice_list,
            commands::get_invoice_detail,
            commands::delete_invoices,
            commands::export_invoices_excel,
            commands::get_config_value,
            commands::set_config_value,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
