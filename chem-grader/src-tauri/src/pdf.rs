use pdfium_render::prelude::*;

/// 依次尝试的 pdfium.dll 位置：可执行文件同目录 → 工作目录 → src-tauri（dev）
fn candidates() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("pdfium.dll").to_string_lossy().to_string());
        }
    }
    v.push("pdfium.dll".to_string());
    v.push("src-tauri/pdfium.dll".to_string());
    v
}

/// 绑定 PDFium 原生库
fn new_pdfium() -> Result<Pdfium, String> {
    for c in candidates() {
        if std::path::Path::new(&c).exists() {
            if let Ok(bindings) = Pdfium::bind_to_library(&c) {
                return Ok(Pdfium::new(bindings));
            }
        }
    }
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| format!("加载 PDFium 失败（需把 pdfium.dll 放到程序目录）: {e}"))
}

/// 抽取 PDF 的**文字层**。电子版 PDF 直接可用；扫描版会返回空串。
pub fn pdf_text(path: &str) -> Result<String, String> {
    let pdfium = new_pdfium()?;
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("打开 PDF 失败: {e}"))?;
    let mut out = String::new();
    for page in doc.pages().iter() {
        let text = page
            .text()
            .map_err(|e| format!("读取 PDF 文字失败: {e}"))?;
        out.push_str(&text.all());
        out.push('\n');
    }
    Ok(out.trim().to_string())
}

/// 把每一页渲染成 PNG 字节（扫描版 PDF 走 OCR 用）。
pub fn pdf_pages_png(path: &str, target_width: i32) -> Result<Vec<Vec<u8>>, String> {
    let pdfium = new_pdfium()?;
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("打开 PDF 失败: {e}"))?;
    let mut pages = Vec::new();
    for page in doc.pages().iter() {
        let config = PdfRenderConfig::new().set_target_width(target_width);
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| format!("渲染 PDF 页面失败: {e}"))?;
        let (w, h) = (bitmap.width() as u32, bitmap.height() as u32);
        let img = image::RgbaImage::from_raw(w, h, bitmap.as_rgba_bytes())
            .ok_or("PDF 位图转换失败")?;
        let mut buf: Vec<u8> = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| format!("PNG 编码失败: {e}"))?;
        pages.push(buf);
    }
    Ok(pages)
}
