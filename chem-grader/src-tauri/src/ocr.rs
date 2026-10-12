use crate::config::{Config, Provider};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// 送 OCR 前的图片宽度上限：再大只是徒增耗时与费用，识别精度几乎不涨
pub const OCR_MAX_W: u32 = 2000;
/// 长边下限：太小会被智谱 OCR 直接拒收（且报错误导为"格式不支持"）
pub const OCR_MIN_SIDE: u32 = 800;
/// 第二读（Qwen-VL-OCR）的识别指令。抽成常量是为了能进「配置指纹」——
/// 改这里会导致缓存自动失效，不用靠人记得手动 +1。
pub const VISION_PROMPT: &str = "请把图片中的文字完整识别出来，保持原有排版与题号，只输出文字本身。";

/// 图片预处理：过大的缩到 OCR_MAX_W 宽，过小的放大到长边 ≥ OCR_MIN_SIDE。
/// 返回 None 表示尺寸合适，原样发送（PDF 渲染页正好 2000 宽，会走这条）。
fn prepare_for_ocr(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let out = if w > OCR_MAX_W {
        let nh = ((h as u64 * OCR_MAX_W as u64) / w as u64).max(1) as u32;
        img.resize_exact(OCR_MAX_W, nh, image::imageops::FilterType::Lanczos3)
    } else if w.max(h) < OCR_MIN_SIDE {
        let k = OCR_MIN_SIDE as f64 / w.max(h) as f64;
        img.resize_exact(
            ((w as f64 * k).round() as u32).max(1),
            ((h as f64 * k).round() as u32).max(1),
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        return None;
    };
    let mut buf = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .ok()?;
    Some(buf)
}

/// 返回（要发送的字节，对应 mime）。处理后一律是 PNG。
fn normalize_image<'a>(bytes: &'a [u8], mime: &'a str) -> (Cow<'a, [u8]>, &'a str) {
    match prepare_for_ocr(bytes) {
        Some(b) => (Cow::Owned(b), "image/png"),
        None => (Cow::Borrowed(bytes), mime),
    }
}

#[derive(Serialize)]
struct OcrReq<'a> {
    model: &'a str,
    file: String,
}

#[derive(Deserialize)]
struct OcrResp {
    md_results: Option<String>,
}

pub fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        _ => "image/png",
    }
}

fn clip(s: &str, n: usize) -> String {
    let s: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if s.chars().count() > n {
        s.chars().take(n).collect::<String>() + "…"
    } else {
        s
    }
}

/// 按文件路径 OCR（图片格式，走主 OCR 提供方）
pub async fn ocr_image(cfg: &Config, path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    ocr_bytes(cfg, &bytes, mime_of(path)).await
}

/// 按原始字节 OCR —— 供 PDF 渲染出的 PNG 复用
pub async fn ocr_bytes(cfg: &Config, bytes: &[u8], mime: &str) -> Result<String, String> {
    let text = call_layout(&cfg.ocr, bytes, mime).await?;
    if text.trim().is_empty() {
        return Err("OCR 未返回识别结果".into());
    }
    Ok(text)
}

/// 仅探测连通性：HTTP 200 即算成功（空结果不算失败）
pub async fn probe_ocr(cfg: &Config, bytes: &[u8], mime: &str) -> Result<(), String> {
    call_layout(&cfg.ocr, bytes, mime).await.map(|_| ())
}

/// 智谱 GLM-OCR `layout_parsing`。返回 md_results（可能为空串）
async fn call_layout(p: &Provider, bytes: &[u8], mime: &str) -> Result<String, String> {
    if p.api_key.is_empty() {
        return Err("未配置 OCR API Key（设置 → API 配置）".into());
    }
    let (b, m) = normalize_image(bytes, mime);
    let data_uri = format!("data:{};base64,{}", m, STANDARD.encode(b.as_ref()));
    let url = format!("{}/layout_parsing", p.base_url.trim_end_matches('/'));

    let resp = reqwest::Client::new()
        .post(&url)
        .header("Authorization", format!("Bearer {}", p.api_key))
        .json(&OcrReq { model: &p.model, file: data_uri })
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", clip(&text, 140)));
    }
    let parsed: OcrResp =
        serde_json::from_str(&text).map_err(|e| format!("OCR 响应解析失败: {e} | {}", clip(&text, 140)))?;
    Ok(parsed.md_results.unwrap_or_default())
}

/// Qwen-VL-OCR（OpenAI 兼容 vision chat）—— 双读的第二份 OCR。
/// 注意：**强制 enable_thinking=false**，否则思维链会消耗大量高价输出 token。
pub async fn ocr_vision(p: &Provider, bytes: &[u8], mime: &str) -> Result<String, String> {
    let text = call_vision(p, bytes, mime).await?;
    if text.trim().is_empty() {
        return Err("Qwen-VL-OCR 未返回识别结果".into());
    }
    Ok(text)
}

/// 探测 Qwen-VL-OCR 连通性（HTTP 200 即算成功）
pub async fn probe_vision(p: &Provider, bytes: &[u8], mime: &str) -> Result<(), String> {
    call_vision(p, bytes, mime).await.map(|_| ())
}

async fn call_vision(p: &Provider, bytes: &[u8], mime: &str) -> Result<String, String> {
    if p.api_key.is_empty() {
        return Err("未配置 Qwen-VL-OCR API Key（设置 → API 配置）".into());
    }
    let (b, m) = normalize_image(bytes, mime);
    let data_uri = format!("data:{};base64,{}", m, STANDARD.encode(b.as_ref()));
    let url = format!("{}/chat/completions", p.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": p.model,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": data_uri } },
                { "type": "text", "text": VISION_PROMPT }
            ]
        }],
        "temperature": 0.1,
        "enable_thinking": false
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .header("Authorization", format!("Bearer {}", p.api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", clip(&text, 140)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("响应解析失败: {e} | {}", clip(&text, 140)))?;
    Ok(v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}
