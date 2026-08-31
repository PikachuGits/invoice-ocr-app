// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 设置 pdfium.dll 加载路径，必须在 pdfium-rs 初始化前执行
    std::env::set_var("PDFIUM_LIB_PATH", "pdfium.dll");

    invoice_ocr_app_lib::run()
}