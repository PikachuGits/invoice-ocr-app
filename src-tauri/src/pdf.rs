use std::path::PathBuf;

use pdfium_render::prelude::*;

/// 渲染 PDF 为 PNG 图片列表，返回 (图片路径列表, 临时目录)。
/// 调用方在识别结束后应删除临时目录。
pub fn render_pdf_to_images(pdf_path: &str) -> Result<(Vec<String>, PathBuf), String> {
    let dir = std::env::temp_dir().join(format!("invoice-ocr-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {}", e))?;

    let bindings = match Pdfium::bind_to_system_library() {
        Ok(bindings) => bindings,
        Err(_) => {
            let path = pdfium_auto::ensure_pdfium_library(None)
                .map_err(|e| format!("下载/查找 PDFium 库失败: {}", e))?;
            Pdfium::bind_to_library(path).map_err(|e| format!("加载 PDFium 库失败: {}", e))?
        }
    };
    let pdfium = Pdfium::new(bindings);

    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|e| format!("PDF 加载失败: {}", e))?;

    let page_count = document.pages().len();
    if page_count == 0 {
        return Err("PDF 没有任何页面".to_string());
    }

    let render_config = PdfRenderConfig::new().set_target_width(2200);

    let mut paths = Vec::new();
    for i in 0..page_count {
        let page = document
            .pages()
            .get(i)
            .map_err(|e| format!("读取 PDF 第 {} 页失败: {}", i + 1, e))?;
        let bitmap = page
            .render_with_config(&render_config)
            .map_err(|e| format!("渲染 PDF 第 {} 页失败: {}", i + 1, e))?;
        let image = bitmap.as_image();
        let out_path = dir.join(format!("page_{:03}.png", i + 1));
        image
            .save_with_format(&out_path, image::ImageFormat::Png)
            .map_err(|e| format!("保存 PDF 页图片失败: {}", e))?;
        paths.push(out_path.to_string_lossy().to_string());
    }

    Ok((paths, dir))
}