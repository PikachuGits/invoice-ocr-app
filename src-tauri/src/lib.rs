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
use tauri::Manager;

fn default_db_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("invoice-ocr-app");
    std::fs::create_dir_all(&path).ok();
    path.push("invoices.db");
    path
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 注意：pdfium 的初始化统一收敛在 pdf::get_pdfium() 中（惰性、进程内只初始化一次，
    // 兼容 pdfium-render 0.9.x 的全局单例约束），这里不再提前加载。

    // ========== 1. 初始化数据库 ==========
    let db_path = default_db_path();
    let db = Database::new(db_path.to_str().unwrap()).expect("Failed to init DB");

    // ========== 2. 启动 Tauri ==========
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(db)
        .setup(|app| {
            // 注入 Tauri 解析出的资源目录，供 pdf.rs 定位随安装包分发的 pdfium 动态库
            if let Ok(dir) = app.path().resource_dir() {
                crate::pdf::set_resource_dir(dir);
            }
            Ok(())
        })
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