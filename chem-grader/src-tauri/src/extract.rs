use crate::config::Config;
use crate::{cache, ocr};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Emitter};

/// 归一化结果：主读文本 + 第二读文本（双读）+ 降级说明 + 是否命中缓存
pub struct Extracted {
    pub text: String,
    pub cached: bool,
    /// 第二份 OCR 的文本；None 表示没做（非 OCR 路径）
    pub cross: Option<String>,
    /// 第二读失败原因（降级）；空串表示成功或未尝试
    pub cross_note: String,
    /// 文档级说明（如 docx 转 PDF 失败而降级）
    pub media_note: String,
}

/// 提取上下文：进度上报 + 缓存策略。这些都不参与提取算法本身。
pub struct ExtractCtx<'a> {
    pub app: &'a AppHandle,
    pub label: &'a str,
    pub index: usize,
    pub total: usize,
    /// true = 忽略缓存强制重新识别（测试用；结果仍会写回缓存）
    pub force: bool,
    /// Some(test_id) = 这是**答题卡扫描件**，缓存写进 `cache/sheets/{test_id}/`
    /// None = 普通答案文件，缓存写进 `cache/tasks/{任务名}/`
    pub sheet_test_id: Option<i64>,
}

/// PDF 渲染宽度（px）。进指纹：改了它，旧缓存自动失效。
/// 类型跟着 `pdf::pdf_pages_png` 的 `target_width: i32`。
const PDF_RENDER_W: i32 = 2000;

/// **提取配置指纹**：把「会影响提取结果」的所有因素揉成一个短串。
/// 任何一项变了 → 指纹变 → 旧缓存自动失效，不必靠人记得手动升 PARSER_VERSION。
/// 包含：解析器版本、两个 OCR 的模型与端点、图片尺寸阈值、第二读提示词、PDF 渲染宽度。
pub fn extract_fingerprint(cfg: &Config) -> String {
    let raw = format!(
        "v{}|{}|{}|{}|{}|{}|{}|{}|{}",
        cache::PARSER_VERSION,
        cfg.ocr.model,
        cfg.ocr.base_url,
        cfg.ocr2.model,
        cfg.ocr2.base_url,
        ocr::OCR_MAX_W,
        ocr::OCR_MIN_SIDE,
        ocr::VISION_PROMPT,
        PDF_RENDER_W,
    );
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in raw.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", h)
}

fn kind_of(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "webp" | "gif" | "tif" | "tiff" => "image",
        "txt" | "md" | "csv" | "json" => "text",
        "docx" => "docx",
        "pdf" => "pdf",
        _ => "unknown",
    }
}

/// 给前端发一条 `import-progress`（页级 / 转换阶段用；没有 page 时前端自动忽略）
fn emit_progress(ctx: &ExtractCtx<'_>, stage: &str, page: usize, pages: usize) {
    let _ = ctx.app.emit(
        "import-progress",
        serde_json::json!({
            "index": ctx.index, "total": ctx.total, "file": ctx.label,
            "stage": stage, "page": page, "pages": pages,
        }),
    );
}

/// 把任意格式归一化成「纯文本」，结果落本地缓存（缓存按 task 分组）。
/// OCR 路径会额外做**第二读**；第二读失败只记 `cross_note`，**不影响主流程**（降级）。
pub async fn extract_text(
    cfg: &Config,
    task: &str,
    path: &str,
    ctx: &ExtractCtx<'_>,
) -> Result<Extracted, String> {
    let mode = kind_of(path).to_string();
    let fp = extract_fingerprint(cfg);

    // force = 忽略缓存，强制重新识别（测试用）
    if !ctx.force {
        let hit = match ctx.sheet_test_id {
            Some(tid) => cache::load_sheet(ctx.app, tid, path, &fp),
            None => cache::load(ctx.app, task, path, &fp),
        };
        if let Some(e) = hit {
            return Ok(Extracted {
                text: e.text,
                cached: true,
                cross: e.cross,
                cross_note: String::new(),
                media_note: String::new(),
            });
        }
    }

    let mut cross: Option<String> = None;
    let mut cross_note = String::new();
    let mut media_note = String::new();

    let text = match mode.as_str() {
        "image" => {
            let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
            let mime = ocr::mime_of(path);
            let primary = ocr::ocr_bytes(cfg, &bytes, mime).await?;
            match ocr::ocr_vision(&cfg.ocr2, &bytes, mime).await {
                Ok(t2) => cross = Some(t2),
                Err(e) => cross_note = e,
            }
            primary
        }
        "text" => std::fs::read_to_string(path).map_err(|e| format!("读取文本失败: {e}"))?,
        // docx：优先转成 PDF 再走渲染+OCR（能还原 WMF/EMF 公式对象）；
        // 本机没有 Word / LibreOffice 时，降级为「按文档顺序抽文本 + 图片占位」
        "docx" => {
            // 转换要几秒，先给前端一句反馈，免得看着像卡死
            emit_progress(ctx, "convert", 0, 0);
            match docx_to_pdf(path) {
                Ok(pdf) => {
                    let p = pdf.to_string_lossy().to_string();
                    let (t, c, n) = pdf_extract(&p, cfg, true, ctx).await?;
                    cross = c;
                    cross_note = n;
                    t
                }
                Err(e) => {
                    media_note = format!(
                        "本机没有可用的 Word / LibreOffice（{e}），已降级为纯文本提取，图片里的公式无法识别"
                    );
                    docx_text(path)?
                }
            }
        }
        "pdf" => {
            let (t, c, n) = pdf_extract(path, cfg, false, ctx).await?;
            cross = c;
            cross_note = n;
            t
        }
        _ => {
            return Err(format!(
                "不支持的文件类型: {path}（.doc 老格式请先另存为 .docx）"
            ))
        }
    };

    match ctx.sheet_test_id {
        Some(tid) => cache::store_sheet(ctx.app, tid, path, &mode, &text, cross.clone(), &fp)?,
        None => cache::store(ctx.app, task, path, &mode, &text, cross.clone(), &fp)?,
    }
    Ok(Extracted { text, cached: false, cross, cross_note, media_note })
}

/// docx → PDF：优先 LibreOffice（headless，无窗口），其次 Word COM。
/// 结果落临时目录并按源文件时间戳复用，避免重复转换。两者都没有则报错（调用方降级）。
fn docx_to_pdf(docx: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("qingfeng-docx");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let stem: String = Path::new(docx)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .chars()
        .map(|c| if r#"\/*?:"><|"#.contains(c) { '_' } else { c })
        .collect();
    let target = dir.join(format!("{stem}_{}.pdf", short_hash(docx)));

    // 已转换过、且不比源文件旧 → 直接复用
    if let (Ok(a), Ok(b)) = (std::fs::metadata(&target), std::fs::metadata(docx)) {
        if let (Ok(ta), Ok(tb)) = (a.modified(), b.modified()) {
            if ta >= tb {
                return Ok(target);
            }
        }
    }

    // 1) LibreOffice
    for cand in [
        r"C:\Program Files\LibreOffice\program\soffice.exe",
        r"C:\Program Files (x86)\LibreOffice\program\soffice.exe",
    ] {
        if !Path::new(cand).exists() {
            continue;
        }
        let dirs = dir.to_string_lossy().to_string();
        let ok = Command::new(cand)
            .args(["--headless", "--norestore", "--convert-to", "pdf", "--outdir", &dirs, docx])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            let produced = dir.join(format!("{stem}.pdf"));
            if produced.exists() {
                std::fs::rename(&produced, &target).map_err(|e| e.to_string())?;
                return Ok(target);
            }
        }
    }

    // 2) Word（COM，经 PowerShell 调用）
    if word_to_pdf(docx, &target).is_ok() && target.exists() {
        return Ok(target);
    }

    Err("未找到 Word 或 LibreOffice".into())
}

/// 用 Word 的 COM 接口导出 PDF。
/// 脚本以 UTF-16LE + base64 经 `-EncodedCommand` 传入，**避开中文路径的编码问题**。
fn word_to_pdf(docx: &str, out: &Path) -> Result<(), String> {
    let q = |s: &str| s.replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop';\
         $w=New-Object -ComObject Word.Application;$w.Visible=$false;$w.DisplayAlerts=0;\
         $d=$w.Documents.Open('{}',$false,$true);\
         $d.ExportAsFixedFormat('{}',17);\
         $d.Close(0);$w.Quit();",
        q(docx),
        q(out.to_string_lossy().as_ref())
    );
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = STANDARD.encode(&utf16);

    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encoded,
    ])
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW，避免闪黑框
    }
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if ok && out.exists() {
        Ok(())
    } else {
        Err("Word 导出 PDF 失败".into())
    }
}

/// FNV-1a 取前 8 位，用于临时文件名
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", h)[..8].to_string()
}

/// PDF：先抽文字层；为空说明是扫描版 → 每页渲染成图走 OCR（并做第二读）。
/// `force_ocr`：docx 转出来的 PDF 必须强制走渲染 —— 它的文字层拿不到公式（公式是矢量图），
/// 只看文字层会得到"空洞"文本。
async fn pdf_extract(
    path: &str,
    cfg: &Config,
    force_ocr: bool,
    ctx: &ExtractCtx<'_>,
) -> Result<(String, Option<String>, String), String> {
    let text = crate::pdf::pdf_text(path)?;
    if !force_ocr && !text.trim().is_empty() {
        // 原生 PDF 且带文字层 → 本身就是精确文本，无需 OCR/双读
        return Ok((text, None, String::new()));
    }
    let pages = crate::pdf::pdf_pages_png(path, PDF_RENDER_W)?;
    if pages.is_empty() {
        return Err("PDF 无页面".into());
    }
    let mut acc = String::new();
    let mut cross = String::new();
    let mut note = String::new();
    let n = pages.len();
    for (i, png) in pages.iter().enumerate() {
        // 逐页上报：一份多页文档可能要跑一两分钟，不能让进度条一直不动
        emit_progress(ctx, "page", i + 1, n);
        let t = ocr::ocr_bytes(cfg, png, "image/png").await?;
        acc.push_str(&format!("\n--- 第 {} 页 ---\n{}", i + 1, t));
        if note.is_empty() {
            match ocr::ocr_vision(&cfg.ocr2, png, "image/png").await {
                Ok(t2) => cross.push_str(&format!("\n--- 第 {} 页 ---\n{}", i + 1, t2)),
                Err(e) => note = e, // 第二读失败即降级，后续页不再尝试
            }
        }
    }
    let cross = if cross.trim().is_empty() { None } else { Some(cross.trim().to_string()) };
    Ok((acc.trim().to_string(), cross, note))
}

/// 未处理图片的占位标记前缀（commands 侧据此统计，命中缓存也能算）
pub const DOCX_IMG_OPEN: &str = "【图片：";

/// 统计文本里未处理的图片数，返回 (总数, 矢量格式数)。
/// 基于**文本内容**统计 —— 无论是否命中缓存都算得出来。
pub fn docx_image_stats(text: &str) -> (usize, usize) {
    let mut total = 0usize;
    let mut vector = 0usize;
    for part in text.split(DOCX_IMG_OPEN).skip(1) {
        total += 1;
        let name = part.split('】').next().unwrap_or("").to_ascii_lowercase();
        if name.ends_with(".wmf") || name.ends_with(".emf") {
            vector += 1;
        }
    }
    (total, vector)
}

/// .docx = zip。**按文档顺序**解析正文：
/// 文字照抄、表格保结构、图片与嵌入对象留占位（不再静默丢弃）
fn docx_text(path: &str) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("打开 docx 失败: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("解析 docx(zip) 失败: {e}"))?;

    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| "docx 缺少 word/document.xml".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;

    // 关系表：rId → media 文件名（缺了也能跑，占位显示"未知"）
    let mut rels: Vec<(String, String)> = Vec::new();
    if let Ok(mut f) = zip.by_name("word/_rels/document.xml.rels") {
        let mut raw = String::new();
        if f.read_to_string(&mut raw).is_ok() {
            rels = parse_rels(&raw);
        }
    }
    Ok(docx_body(&xml, &rels))
}

/// 解析 rels：抽出 (Id, Target) 对
fn parse_rels(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(i) = rest.find("Id=\"") {
        rest = &rest[i + 4..];
        let Some(j) = rest.find('"') else { break };
        let id = rest[..j].to_string();
        rest = &rest[j..];
        let Some(k) = rest.find("Target=\"") else { break };
        let after = &rest[k + 8..];
        let Some(l) = after.find('"') else { break };
        let target = after[..l].to_string();
        rest = &after[l..];
        out.push((id, target));
    }
    out
}

/// 在图片/对象内部找出引用的 rId
fn find_rid(inner: &str) -> Option<&str> {
    for key in ["r:embed=\"", "r:id=\""] {
        if let Some(i) = inner.find(key) {
            let after = &inner[i + key.len()..];
            if let Some(j) = after.find('"') {
                return Some(&after[..j]);
            }
        }
    }
    None
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// 按文档顺序走一遍标签流：只认「有意义的」标签，其余忽略
fn docx_body(xml: &str, rels: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut rest = xml;

    while let Some(lt) = rest.find('<') {
        let tail = &rest[lt..];
        let Some(gt) = tail.find('>') else { break };
        let tag = &tail[1..gt];
        rest = &tail[gt + 1..];

        // 结束标签
        if let Some(end) = tag.strip_prefix('/') {
            match end.trim() {
                "w:p" | "w:tr" => out.push('\n'),
                "w:tc" => out.push_str(" | "),
                _ => {}
            }
            continue;
        }

        let self_closing = tag.ends_with('/');
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");

        match name {
            // 文本：连同实体一起解出来
            "w:t" if !self_closing => {
                if let Some(e) = rest.find("</w:t>") {
                    out.push_str(&decode_entities(&rest[..e]));
                    rest = &rest[e + 6..];
                }
            }
            "w:tab" => out.push(' '),
            "w:br" | "w:cr" => out.push('\n'),
            // 图片 / 嵌入对象：留占位，绝不静默丢弃
            "w:drawing" | "w:object" | "w:pict" if !self_closing => {
                let close = format!("</{name}>");
                let end = rest.find(&close).unwrap_or(rest.len());
                let inner = &rest[..end];
                let label = find_rid(inner)
                    .and_then(|rid| rels.iter().find(|(id, _)| id == rid))
                    .map(|(_, t)| t.rsplit('/').next().unwrap_or(t).to_string())
                    .unwrap_or_else(|| "未知".to_string());
                out.push_str(&format!("{DOCX_IMG_OPEN}{label}】"));
                rest = &rest[(end + close.len()).min(rest.len())..];
            }
            _ => {}
        }
    }
    cleanup(&out)
}

/// 收尾：逐行 trim、去掉表格行尾多余的 `|`、压缩连续空行
fn cleanup(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.split('\n') {
        let mut l = line.trim();
        if let Some(st) = l.strip_suffix('|') {
            l = st.trim();
        }
        if l.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim().to_string()
}
