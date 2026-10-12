use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// 归一化解析器版本：**改动 docx / PDF / 图片的提取逻辑时必须 +1**，
/// 否则源文件没变、旧缓存照命中，新逻辑永远不生效。
/// 1 = 最初版本；2 = docx 按文档顺序解析 + 图片占位；3 = docx 走「转 PDF → 渲染 OCR」；
/// 4 = 送 OCR 前做分辨率归一（过大缩小 / 过小放大）
pub const PARSER_VERSION: u32 = 4;

/// 缓存条目：记录所属任务、源文件、指纹、提取方式与文本
#[derive(Serialize, Deserialize)]
pub struct CacheEntry {
    pub task: String,
    pub source: String,
    pub mode: String,
    pub mtime: u64,
    pub size: u64,
    pub created_at: u64,
    pub text: String,
    /// 第二份 OCR 的文本（双读用）；老缓存无此字段
    #[serde(default)]
    pub cross: Option<String>,
    /// 生成该缓存时的解析器版本；老缓存为 0，会因版本不符自动失效
    #[serde(default)]
    pub parser: u32,
    /// 生成该缓存时的**提取配置指纹**（模型名、端点、图片阈值、OCR 提示词…）。
    /// 老缓存为空串 → 必然与当前指纹不符 → 自动失效，不必靠人记得改代码。
    #[serde(default)]
    pub fingerprint: String,
    /// 分类：`answer`（答案文件）/ `sheet`（答题卡扫描件）
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 绑定到哪一场考试（答案文件保存后回填，答题卡写入时即带上）
    #[serde(default)]
    pub test_id: Option<i64>,
}

fn default_kind() -> String {
    "answer".to_string()
}

/// 缓存根目录（按任务分组，避免不同任务串味）
pub fn task_dir(app: &AppHandle, task: &str) -> Result<PathBuf, String> {
    let name = {
        let s = sanitize(task.trim());
        if s.is_empty() { "_untitled".to_string() } else { s }
    };
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("cache")
        .join("tasks")
        .join(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// FNV-1a：稳定、无依赖的字符串哈希
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 把文件名里非法的字符替换掉（保留中文）
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if r#"\/*?:"><|"#.contains(c) || c == ' ' { '_' } else { c })
        .collect()
}

/// 缓存文件名：`{源文件名词干}_{源绝对路径指纹8位}.cache.json`
/// —— 词干人类可读；路径指纹保证同名文件不冲突
pub fn entry_path_in(dir: &Path, source: &str) -> PathBuf {
    let stem = Path::new(source)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let key = format!("{:016x}", fnv1a(&source.to_ascii_lowercase()));
    dir.join(format!("{}_{}.cache.json", sanitize(stem), &key[..8]))
}

/// 答案文件缓存路径：`cache/tasks/{任务名}/…`
pub fn entry_path(app: &AppHandle, task: &str, source: &str) -> Result<PathBuf, String> {
    Ok(entry_path_in(&task_dir(app, task)?, source))
}

/// **答题卡扫描件**缓存目录：`cache/sheets/{test_id}/`
/// 按 test_id 分目录 —— 这就是"分类 & 绑定"的落点（清单另存 DB 的 file_log）
pub fn sheet_dir(app: &AppHandle, test_id: i64) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("cache")
        .join("sheets")
        .join(test_id.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn file_stamp(path: &str) -> Option<(u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((mtime, md.len()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 命中缓存返回条目（含第二读文本）。
/// 三种情况都视为失效：源文件被改动、解析器版本变了、**提取配置指纹变了**（如换了 OCR 模型）。
pub fn load(app: &AppHandle, task: &str, source: &str, fingerprint: &str) -> Option<CacheEntry> {
    load_in(&entry_path(app, task, source).ok()?, source, fingerprint)
}

pub fn store(
    app: &AppHandle,
    task: &str,
    source: &str,
    mode: &str,
    text: &str,
    cross: Option<String>,
    fingerprint: &str,
) -> Result<(), String> {
    store_in(
        &entry_path(app, task, source)?,
        task,
        source,
        mode,
        text,
        cross,
        fingerprint,
        "answer",
        None,
    )
}

/// **答题卡扫描件**：读缓存（按 test_id 隔离，绑定在 CacheEntry 上）
pub fn load_sheet(app: &AppHandle, test_id: i64, source: &str, fingerprint: &str) -> Option<CacheEntry> {
    load_in(&entry_path_in(&sheet_dir(app, test_id).ok()?, source), source, fingerprint)
}

/// **答题卡扫描件**：写缓存
pub fn store_sheet(
    app: &AppHandle,
    test_id: i64,
    source: &str,
    mode: &str,
    text: &str,
    cross: Option<String>,
    fingerprint: &str,
) -> Result<(), String> {
    store_in(
        &entry_path_in(&sheet_dir(app, test_id)?, source),
        &format!("test:{test_id}"),
        source,
        mode,
        text,
        cross,
        fingerprint,
        "sheet",
        Some(test_id),
    )
}

/// 通用读：三种情况视为失效 —— 源文件被改动 / 解析器版本变 / **提取配置指纹变**
fn load_in(path: &Path, source: &str, fingerprint: &str) -> Option<CacheEntry> {
    let raw = std::fs::read_to_string(path).ok()?;
    let entry: CacheEntry = serde_json::from_str(&raw).ok()?;
    let (mtime, size) = file_stamp(source)?;
    if entry.parser == PARSER_VERSION
        && entry.fingerprint == fingerprint
        && entry.mtime == mtime
        && entry.size == size
    {
        Some(entry)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn store_in(
    path: &Path,
    task: &str,
    source: &str,
    mode: &str,
    text: &str,
    cross: Option<String>,
    fingerprint: &str,
    kind: &str,
    test_id: Option<i64>,
) -> Result<(), String> {
    let (mtime, size) = file_stamp(source).ok_or("源文件不可读，无法缓存")?;
    let entry = CacheEntry {
        task: task.to_string(),
        source: source.to_string(),
        mode: mode.to_string(),
        mtime,
        size,
        created_at: now(),
        text: text.to_string(),
        cross,
        parser: PARSER_VERSION,
        fingerprint: fingerprint.to_string(),
        kind: kind.to_string(),
        test_id,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&entry).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct CacheStats {
    pub dir: String,
    pub files: u64,
    pub bytes: u64,
}

/// 当前任务缓存占用
pub fn stats(app: &AppHandle, task: &str) -> Result<CacheStats, String> {
    let dir = task_dir(app, task)?;
    let mut files = 0u64;
    let mut bytes = 0u64;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    files += 1;
                    bytes += md.len();
                }
            }
        }
    }
    Ok(CacheStats {
        dir: dir.to_string_lossy().to_string(),
        files,
        bytes,
    })
}

/// 清除当前任务的缓存，返回删掉的文件数
pub fn clear(app: &AppHandle, task: &str) -> Result<u64, String> {
    let dir = task_dir(app, task)?;
    let mut n = 0u64;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.path().is_file() && std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
    }
    Ok(n)
}
