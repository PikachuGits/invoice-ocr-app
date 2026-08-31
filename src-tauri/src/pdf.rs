use std::path::PathBuf;
use std::sync::OnceLock;

use pdfium_render::prelude::*;

/// 全局唯一的 Pdfium 实例。
/// pdfium-render 0.9.x 内部使用全局 OnceCell 保存绑定，进程内只允许初始化一次，
/// 因此必须把所有 PDF 操作收敛到这一个实例上（惰性初始化，线程安全）。
/// 存 Result 是为了兼容旧版 rustc（get_or_try_init 尚未稳定）。
static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

/// 打包后由 Tauri 注入的资源目录（app.path().resource_dir()），
/// 用于定位随安装包分发的 pdfium 动态库（避免运行时联网下载）。
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 由 lib.rs 在应用启动时调用，注入 Tauri 解析出的资源目录。
pub fn set_resource_dir(dir: PathBuf) {
    let _ = RESOURCE_DIR.set(dir);
}

/// 当前平台 pdfium 动态库的文件名。
fn pdfium_lib_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "pdfium.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libpdfium.dylib"
    }
    #[cfg(target_os = "linux")]
    {
        "libpdfium.so"
    }
}

/// 查找打包/内置的 pdfium 动态库路径（存在才返回，否则返回最后的兜底路径）。
/// - 打包后：Tauri resource_dir()（Windows 在安装目录、macOS 在 .app/Contents/Resources）
/// - 开发时：项目 ./resources/ 目录
fn bundled_pdfium_path() -> PathBuf {
    let name = pdfium_lib_name();

    // 1. Tauri 资源目录（打包后），兼容资源映射到 resources/ 子目录或根目录两种情况
    if let Some(dir) = RESOURCE_DIR.get() {
        for candidate in [dir.join("resources").join(name), dir.join(name)] {
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // 2. 可执行文件同目录下的 resources/（未注入 resource_dir 时的兜底）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join("resources").join(name);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // 3. 旧版 Tauri v1 布局：%APPDATA%/{identifier}/resources/
    #[cfg(target_os = "windows")]
    if let Some(app_data) = std::env::var_os("APPDATA") {
        let candidate = PathBuf::from(app_data)
            .join("com.dongzhuo.invoice-ocr-app") // ← 与 tauri.conf.json identifier 一致
            .join("resources")
            .join(name);
        if candidate.exists() {
            return candidate;
        }
    }

    // 4. 开发模式：CWD 下的 ./resources/
    PathBuf::from("./resources").join(name)
}

/// 获取全局 Pdfium 实例（惰性初始化，整个进程生命周期内只初始化一次）。
/// 加载顺序：打包/内置动态库 → 系统库 → pdfium-auto 自动下载缓存。
fn get_pdfium() -> Result<&'static Pdfium, String> {
    PDFIUM
        .get_or_init(|| {
            // 1. 优先加载打包/内置的 pdfium 动态库
            let bundled = bundled_pdfium_path();
            if bundled.exists() {
                match Pdfium::bind_to_library(&bundled) {
                    Ok(bindings) => return Ok(Pdfium::new(bindings)),
                    Err(e) => log::warn!("内置 pdfium 库加载失败 {:?}: {}", bundled, e),
                }
            }

            // 2. 尝试从系统库加载
            if let Ok(bindings) = Pdfium::bind_to_system_library() {
                return Ok(Pdfium::new(bindings));
            }

            // 3. 兜底：pdfium-auto 下载/查找动态库（缓存到本地，不重复下载）
            let path = pdfium_auto::ensure_pdfium_library(None)
                .map_err(|e| format!("下载/查找 PDFium 库失败: {}", e))?;
            let bindings = Pdfium::bind_to_library(path)
                .map_err(|e| format!("加载 PDFium 库失败: {}", e))?;
            Ok(Pdfium::new(bindings))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// 渲染 PDF 为 PNG 图片列表，返回 (图片路径列表, 临时目录)。
/// 调用方在识别结束后应删除临时目录。
pub fn render_pdf_to_images(pdf_path: &str) -> Result<(Vec<String>, PathBuf), String> {
    let dir = std::env::temp_dir().join(format!("invoice-ocr-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {}", e))?;

    let pdfium = get_pdfium()?;

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
        // 0.9.x 中 as_image() 返回 Result，需要先解包
        let image = bitmap
            .as_image()
            .map_err(|e| format!("转换 PDF 第 {} 页图片失败: {}", i + 1, e))?;
        let out_path = dir.join(format!("page_{:03}.png", i + 1));
        image
            .save_with_format(&out_path, image::ImageFormat::Png)
            .map_err(|e| format!("保存 PDF 页图片失败: {}", e))?;
        paths.push(out_path.to_string_lossy().to_string());
    }

    Ok((paths, dir))
}
