use crate::{
    cache, config, config::Config, db, extract, llm, mathfmt, ocr, structure,
    structure::Question,
};
use serde::Serialize;
use tauri::Emitter;

/// 单个文件导入失败
#[derive(Serialize)]
pub struct FileFailure {
    pub path: String,
    pub error: String,
}

/// 导入结果：题目 + 失败清单 + 命中缓存数 + 降级说明
#[derive(Serialize)]
pub struct ImportOutcome {
    pub questions: Vec<Question>,
    pub failed: Vec<FileFailure>,
    pub reused: usize,
    /// 第二读不可用时的降级说明（如额度不足），不影响导入
    pub notes: Vec<String>,
}

/// 进度事件负载
#[derive(Serialize, Clone)]
struct Progress {
    index: usize,
    total: usize,
    file: String,
    stage: String,
}

/// 返回数据库文件路径，供前端「设置 - 数据目录」显示
#[tauri::command]
pub fn db_status(app: tauri::AppHandle) -> Result<String, String> {
    Ok(db::db_path(&app)?.to_string_lossy().to_string())
}

/// 应用信息
#[tauri::command]
pub fn app_info() -> serde_json::Value {
    serde_json::json!({
        "name": "清风AI系统",
        "version": env!("CARGO_PKG_VERSION")
    })
}

/// 读取配置（含 API Key）
#[tauri::command]
pub fn get_config(app: tauri::AppHandle) -> Result<Config, String> {
    config::load(&app)
}

/// 保存配置（前端把整份 Config 传回来）
#[tauri::command]
pub fn save_config(app: tauri::AppHandle, cfg: Config) -> Result<(), String> {
    config::save(&app, &cfg)
}

/// OCR：本地图片路径 → 识别文本（走配置里的 ocr 提供方）
#[tauri::command]
pub async fn ocr_image(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let cfg = config::load(&app)?;
    ocr::ocr_image(&cfg, &path).await
}

/// 判分：标准答案 + 学生答案 → LLM 语义判分，返回 JSON 文本
#[tauri::command]
pub async fn grade_answer(
    app: tauri::AppHandle,
    question: String,
    standard: String,
    student: String,
    score: f64,
    rubric: Option<String>,
) -> Result<String, String> {
    let cfg = config::load(&app)?;
    let system = "你是高中化学阅卷老师。判断学生答案与标准答案是否语义等价，\
        注意化学符号、方程式、离子式的等价写法。\
        若给了评分细则，**必须按细则逐条给分**（细则没提到的部分不要额外扣分）。\
        只输出 JSON，不要多余文字：\
        {\"score\": 得分数字, \"verdict\": \"correct|partial|wrong\", \
        \"confidence\": 0到1, \"needs_review\": true或false, \"reason\": \"简短理由\"}";
    let rubric_line = match rubric.as_deref().map(str::trim) {
        Some(r) if !r.is_empty() => format!("\n评分细则：{r}"),
        _ => String::new(),
    };
    let user = format!(
        "题目：{question}\n标准答案：{standard}\n学生答案：{student}\n满分：{score} 分{rubric_line}"
    );
    llm::chat(&cfg, Some(system), &user).await
}

/// 测试 LLM 连通性（也顺带验证 Key 是否有效）
#[tauri::command]
pub async fn test_llm(app: tauri::AppHandle) -> Result<String, String> {
    let cfg = config::load(&app)?;
    llm::chat(&cfg, None, "只回复两个字：正常").await
}

/// 导入答案文件：按选择顺序逐份「归一化 → 整份发 LLM 结构化」
/// **逐份容错**——单个文件失败不中断其余；全程推送 `import-progress` 事件
/// `force=true` 时忽略缓存、强制重新识别（测试用；结果仍写回缓存）
/// `prefer_new=true` 时，重复题号保留**后导入**的版本（默认保留先导入的）
#[tauri::command]
pub async fn import_answer_files(
    app: tauri::AppHandle,
    task: String,
    paths: Vec<String>,
    force: bool,
    prefer_new: bool,
) -> Result<ImportOutcome, String> {
    if paths.is_empty() {
        return Err("未选择任何文件".into());
    }
    let cfg = config::load(&app)?;
    let total = paths.len();
    let mut questions: Vec<Question> = Vec::new();
    let mut failed: Vec<FileFailure> = Vec::new();
    let mut reused = 0usize;
    let mut notes: Vec<String> = Vec::new();
    if force {
        notes.push("已按「忽略缓存」重新识别（未复用任何旧结果）".into());
    }

    for (i, p) in paths.iter().enumerate() {
        let name = p
            .rsplit(|c| c == '\\' || c == '/')
            .next()
            .unwrap_or(p.as_str())
            .to_string();
        let _ = app.emit(
            "import-progress",
            Progress { index: i + 1, total, file: name.clone(), stage: "extract".into() },
        );

        let ctx = extract::ExtractCtx {
            app: &app,
            label: &name,
            index: i + 1,
            total,
            force,
            sheet_test_id: None, // 答案文件走 cache/tasks/{任务名}/
        };
        let ex = match extract::extract_text(&cfg, &task, p, &ctx).await {
            Ok(v) => v,
            Err(e) => {
                failed.push(FileFailure { path: p.clone(), error: e });
                continue;
            }
        };
        if ex.cached {
            reused += 1;
        }
        if ex.text.trim().is_empty() {
            failed.push(FileFailure { path: p.clone(), error: "未提取到文本".into() });
            continue;
        }
        // 第二读失败 → 降级（记录说明，不中断）
        if !ex.cross_note.is_empty() {
            notes.push(format!("{name}：第二读不可用，已降级为单读（{}）", ex.cross_note));
        }
        // 文档级降级（例如 docx 转 PDF 失败）
        if !ex.media_note.is_empty() {
            notes.push(format!("{name}：{}", ex.media_note));
        }

        let _ = app.emit(
            "import-progress",
            Progress { index: i + 1, total, file: name.clone(), stage: "structure".into() },
        );

        // 【刻意不缓存结构化结果】——只缓存"读出来的原始文本"。
        // 答案数据必须等用户点「确认并保存」才落库：未确认的一版默认视为作废，
        // 否则用户改了却没保存、再导入同一文件时，缓存会把作废的旧版"复活"、覆盖用户改动。
        match structure::structure_answers(&cfg, &ex.text).await {
            Ok(mut qs) => {
                // ③ 题号规范化（全角括号/各种分隔符统一成 16(1) 这类形式）
                for q in qs.iter_mut() {
                    q.no = normalize_no(&q.no);
                }

                // ①【原文回溯校验】答案必须在原文里找到出处，否则清空并打标。
                // 单字符答案（选择题 A/B/C/D）改看「显式答案区」，见 sourced()
                let src = normalize_text(&ex.text);
                let declared = declared_answers(&ex.text);
                let mut dropped = 0usize;
                for q in qs.iter_mut() {
                    if let Some(a) = q.answer.clone() {
                        if !a.trim().is_empty() && !sourced(&src, &declared, &a) {
                            q.answer = None;
                            q.unverified = true;
                            dropped += 1;
                        }
                    }
                    // ⑤ 原文片段对照：把答案在原文里的上下文一并带上，老师核对时不用来回翻
                    if let Some(a) = &q.answer {
                        q.src = window_around(&ex.text, a, 90);
                    }
                }
                // 整份文件看起来就不像答案文档 → 明确提示
                if !looks_like_answer_doc(&ex.text) {
                    notes.push(format!(
                        "{name}：原文里找不到「参考答案/评分细则」一类的标记，**这份可能不是答案文件**"
                    ));
                }
                if dropped > 0 {
                    notes.push(format!(
                        "{name}：有 {dropped} 处答案在原文中**找不到出处**，已清空（疑似模型自行作答，请人工补录）"
                    ));
                }
                // 文档里还有没被识别的图片/公式对象 → 明说，不静默丢失
                let (imgs, vector) = extract::docx_image_stats(&ex.text);
                if imgs > 0 {
                    notes.push(format!(
                        "{name}：文档里有 {imgs} 处图片/公式对象**尚未识别**（其中 {vector} 处是 WMF/EMF 矢量图）；\
                         标准答案若在这些图里，对应题目会显示为空"
                    ));
                }

                // ⑥【双读比对】答案是否也出现在第二读文本里；不符时把第二读的对应片段也带出来
                if let Some(cross) = &ex.cross {
                    let cn = normalize_text(cross);
                    for q in qs.iter_mut() {
                        match &q.answer {
                            Some(a) => {
                                let ok = answer_in(&cn, a);
                                q.agree = Some(ok);
                                if !ok {
                                    q.cross_src = locate_question(cross, &q.no, 90);
                                }
                            }
                            None => q.agree = Some(true), // 已清空的答案不参与双读判定
                        }
                    }
                }

                // ⑦ 最后一步才「人化」答案（GLM-OCR 的 LaTeX → 老师能读的形态）。
                // **必须放在校验/比对之后**，否则会把原文里真实存在的 `\mathrm{Fe}` 改掉、
                // 导致原文校验对不上。
                // ⚠️ 只人化 answer：`src` / `cross_src` 是**对照用的原文**，
                // 必须保持 OCR 原样，否则老师无从判断"是人化错了还是识别错了"。
                for q in qs.iter_mut() {
                    if let Some(a) = &q.answer {
                        q.answer = Some(mathfmt::humanize(a));
                    }
                }
                questions.append(&mut qs);
            }
            Err(e) => failed.push(FileFailure { path: p.clone(), error: e }),
        }
    }

    // ④ 跨文件按题号自然排序 + 同题号去重。
    // 冲突不再静默丢弃：把重复题号报出来，并按 prefer_new 决定保留哪一份。
    questions.sort_by_key(|q| no_key(&q.no));
    let mut where_at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut merged: Vec<Question> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    for q in questions.into_iter() {
        match where_at.get(&q.no).copied() {
            Some(idx) => {
                if !conflicts.contains(&q.no) {
                    conflicts.push(q.no.clone());
                }
                if prefer_new {
                    merged[idx] = q; // 以新导入的为准
                }
            }
            None => {
                where_at.insert(q.no.clone(), merged.len());
                merged.push(q);
            }
        }
    }
    let questions = merged;
    if !conflicts.is_empty() {
        let shown = conflicts.iter().take(8).cloned().collect::<Vec<_>>().join("、");
        let more = if conflicts.len() > 8 { " 等" } else { "" };
        notes.push(format!(
            "检测到 {} 个重复题号（{shown}{more}），已统一保留「{}」的版本；\
             如需反过来，可在导入时勾选「重复题号以新导入的为准」",
            conflicts.len(),
            if prefer_new { "后导入" } else { "先导入" }
        ));
    }

    // ⑦ 题号连续性检查（防原文漏题而无人察觉）
    let mains: Vec<i64> = questions.iter().map(|q| no_key(&q.no).0).filter(|n| *n > 0).collect();
    if let (Some(&lo), Some(&hi)) = (mains.iter().min(), mains.iter().max()) {
        let missing: Vec<i64> = (lo..=hi).filter(|n| !mains.contains(n)).collect();
        if !missing.is_empty() {
            let list = missing.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("、");
            notes.push(format!("题号不连续，缺少第 {list} 题，请确认原文是否漏题"));
        }
    }

    // ⑥ 空答案专项提示：告诉老师"空的是哪几题、大概为什么"，而不是只留一排空格
    let empties: Vec<String> = questions
        .iter()
        .filter(|q| q.answer.is_none() && !q.unverified)
        .map(|q| q.no.clone())
        .collect();
    if !empties.is_empty() {
        let shown = empties.iter().take(10).cloned().collect::<Vec<_>>().join("、");
        let more = if empties.len() > 10 { " 等" } else { "" };
        notes.push(format!(
            "有 {} 题**没提取到答案**（{shown}{more}）。常见原因是答案以公式图 / 结构式 / 手写形式呈现，\
             模型读不到——建议对照原文件手工补录",
            empties.len()
        ));
    }

    if questions.is_empty() && failed.is_empty() {
        return Err("未从文件中解析出任何题目".into());
    }

    // ⑨ 记录本次导入用到的源文件（纯日志，失败不影响导入结果）。
    // 此处还不知道 test_id —— 「确认并保存」拿到 test_id 后再回填绑定。
    let failed_paths: std::collections::HashSet<&str> =
        failed.iter().map(|f| f.path.as_str()).collect();
    let logs: Vec<db::FileLogRow> = paths
        .iter()
        .map(|p| db::FileLogRow {
            path: p.clone(),
            status: if failed_paths.contains(p.as_str()) { "failed" } else { "done" }.into(),
            kind: "answer".into(),
            test_id: None,
        })
        .collect();
    let _ = db::log_files(&app, &logs);

    Ok(ImportOutcome { questions, failed, reused, notes })
}

/// 归一化：去空白、统一常见全角标点与符号、去掉对识别无意义的标点
fn normalize_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_whitespace() {
            continue;
        }
        let c = match c {
            '，' | '、' => ',',
            '。' => '.',
            '；' => ';',
            '：' => ':',
            '（' => '(',
            '）' => ')',
            '＝' => '=',
            '＋' => '+',
            '－' => '-',
            other => other,
        };
        if matches!(c, '.' | ',' | ';' | ':' | '(' | ')' | '·' | '-' | '_' | '/' | '\\' | '*' | '#')
        {
            continue;
        }
        out.push(c);
    }
    out
}

/// 该题答案是否（大致）出现在另一份文本里。
/// 太短的（≤3 字）不判，避免噪音；短答案允许 4 字窗口、长答案 6 字窗口命中（容忍局部改写与标点差异）。
fn answer_in(other_norm: &str, answer: &str) -> bool {
    let a = normalize_text(answer);
    let chars: Vec<char> = a.chars().collect();
    let n = chars.len();
    if n <= 3 {
        return true;
    }
    if other_norm.contains(&a) {
        return true;
    }
    let w = if n <= 10 { 4 } else { 6 };
    chars
        .windows(w)
        .any(|x| other_norm.contains(&x.iter().collect::<String>()))
}

/// 答案是否能在**原文**里找到出处（拦住模型编造）。
/// 单字符答案（选择题 A/B/C/D）不能靠"全文任意位置"判断——所以改看「显式答案区」。
fn sourced(src_norm: &str, declared: &str, answer: &str) -> bool {
    let a = normalize_text(answer);
    if a.chars().count() < 2 {
        // 原文里没有任何答案区标记时无法判定，不误杀（另有整体警示兜着）
        if declared.is_empty() {
            return true;
        }
        return declared.contains(&a);
    }
    answer_in(src_norm, &a)
}

/// 抽出原文里「显式声明的答案」（如 【答案】A / 答案：A）。
/// 用**原始文本**匹配——归一化会把冒号这类标点删掉。
fn declared_answers(raw: &str) -> String {
    let mut out = String::new();
    for key in ["【答案】", "答案：", "答案:", "答案是", "答案."] {
        let mut rest = raw;
        while let Some(i) = rest.find(key) {
            rest = &rest[i + key.len()..];
            if let Some(c) = rest.chars().find(|c| !c.is_whitespace()) {
                out.push(c);
            }
        }
    }
    out
}

/// 题号规范化：去空白、全角括号转半角、剥掉「第…题」外壳与多余分隔点
fn normalize_no(s: &str) -> String {
    let mut t: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '（' => '(',
            '）' => ')',
            '．' | '。' | '、' => '.',
            other => other,
        })
        .collect();
    if let Some(r) = t.strip_prefix('第') {
        t = r.to_string();
    }
    if let Some(r) = t.strip_suffix('题') {
        t = r.to_string();
    }
    t = t.replace(".(", "(");
    while t.ends_with('.') {
        t.pop();
    }
    t
}

/// 题号的排序键：(主号, 各级子号) —— 让 1,2,…,15(1),15(2),…,18(8) 自然有序
fn no_key(no: &str) -> (i64, Vec<i64>) {
    let mut main = 0i64;
    let mut subs: Vec<i64> = Vec::new();
    let mut num = String::new();
    let mut first = true;
    for c in no.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            num.push(c);
        } else if !num.is_empty() {
            let v: i64 = num.parse().unwrap_or(0);
            if first {
                main = v;
                first = false;
            } else {
                subs.push(v);
            }
            num.clear();
        }
    }
    (main, subs)
}

/// 在文本里定位 needle（**忽略空白**匹配），返回其前后 span 个字符的窗口
fn window_around(text: &str, needle: &str, span: usize) -> Option<String> {
    let target: String = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if target.chars().count() < 2 {
        return None;
    }
    let mut plain = String::new();
    let mut map: Vec<usize> = Vec::new();
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            continue;
        }
        plain.push(c);
        map.push(i);
    }
    let probe: String = target.chars().take(12).collect();
    let pos = plain.find(&probe)?;
    let ci = plain[..pos].chars().count();
    let b0 = map.get(ci.saturating_sub(span)).copied()?;
    let end_char = (ci + probe.chars().count() + span).min(map.len());
    let b1 = if end_char >= map.len() { text.len() } else { map[end_char] };
    if b0 >= b1 {
        return None;
    }
    let mut s = text[b0..b1].trim().to_string();
    if b0 > 0 {
        s.insert_str(0, "…");
    }
    if b1 < text.len() {
        s.push('…');
    }
    Some(s)
}

/// 到第二读文本里定位该题附近的内容（双读不符时用；先试完整题号，再退到主号）
fn locate_question(text: &str, no: &str, span: usize) -> Option<String> {
    if let Some(s) = window_around(text, no, span) {
        return Some(s);
    }
    let main = no_key(no).0;
    if main > 0 {
        return window_around(text, &main.to_string(), span);
    }
    None
}

/// 粗判这份原文像不像「答案文档」——用严格的标记，避免被试卷说明里的"答案"字样误伤
fn looks_like_answer_doc(text: &str) -> bool {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    ["参考答案", "评分细则", "评分标准", "答案与解析", "答案解析", "【答案】", "答案:"]
        .iter()
        .any(|k| t.contains(k))
}

/// 当前任务的缓存占用
#[tauri::command]
pub fn cache_stats(app: tauri::AppHandle, task: String) -> Result<cache::CacheStats, String> {
    cache::stats(&app, &task)
}

/// 清除当前任务的缓存，返回删除的文件数
#[tauri::command]
pub fn clear_cache(app: tauri::AppHandle, task: String) -> Result<u64, String> {
    cache::clear(&app, &task)
}

/// 保存答案到数据库（幂等：同任务名覆盖题目），返回 test_id。
/// 同时把本次导入的**答案文件**绑定到这场考试（test_id + kind='answer'）。
#[tauri::command]
pub fn save_answers(
    app: tauri::AppHandle,
    task: String,
    questions: Vec<Question>,
    paths: Vec<String>,
) -> Result<i64, String> {
    let test_id = db::save_answers(&app, &task, &questions)?;
    // 绑定失败不该让保存失败 —— 只是分类/绑定缺失
    let _ = db::bind_files(&app, test_id, &paths, "answer");
    Ok(test_id)
}

/// 答题卡扫描件导入结果
#[derive(Serialize)]
pub struct SheetsOutcome {
    pub test_id: i64,
    pub total: usize,
    /// 命中缓存（复用）的份数
    pub reused: usize,
    /// 已绑定到本场考试的答题卡总数
    pub bound: i64,
    pub failed: Vec<FileFailure>,
}

/// 识别一份答题卡的扩展名白名单（与答案文件一致）
fn is_sheet_file(name: &str) -> bool {
    matches!(
        name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "bmp" | "webp" | "tif" | "tiff" | "pdf"
    )
}

/// 【答题卡】批量识别选中的全班扫描件，**按 test_id 分类缓存并绑定**。
/// 逐份容错：单份失败只记 failed，不中断。全程推送 `sheet-progress`。
/// 这里只做「识别 + 缓存 + 绑定」，**发给 LLM 核对是下一步**。
#[tauri::command]
pub async fn import_sheets(
    app: tauri::AppHandle,
    test_id: i64,
    paths: Vec<String>,
) -> Result<SheetsOutcome, String> {
    let mut files: Vec<String> = paths.into_iter().filter(|p| is_sheet_file(p)).collect();
    // 文件名排序，保证进度与结果稳定可复现
    files.sort_by_key(|p| p.to_ascii_lowercase());
    if files.is_empty() {
        return Err("没有可识别的图片 / PDF（支持 png/jpg/bmp/webp/tif/pdf）".into());
    }

    let cfg = config::load(&app)?;
    let total = files.len();
    let mut failed: Vec<FileFailure> = Vec::new();
    let mut reused = 0usize;
    let mut logs: Vec<db::FileLogRow> = Vec::new();

    for (i, p) in files.iter().enumerate() {
        let name = p
            .rsplit(|c| c == '\\' || c == '/')
            .next()
            .unwrap_or(p.as_str())
            .to_string();
        let _ = app.emit(
            "sheet-progress",
            Progress { index: i + 1, total, file: name.clone(), stage: "sheet".into() },
        );
        let ctx = extract::ExtractCtx {
            app: &app,
            label: &name,
            index: i + 1,
            total,
            force: false,
            sheet_test_id: Some(test_id),
        };
        // task 参数在这里只用于进度与日志；缓存实际走 sheets/{test_id}/
        match extract::extract_text(&cfg, "", p, &ctx).await {
            Ok(ex) => {
                if ex.cached {
                    reused += 1;
                }
                if ex.text.trim().is_empty() {
                    failed.push(FileFailure { path: p.clone(), error: "未提取到文本".into() });
                    logs.push(db::FileLogRow {
                        path: p.clone(),
                        status: "failed".into(),
                        kind: "sheet".into(),
                        test_id: Some(test_id),
                    });
                } else {
                    logs.push(db::FileLogRow {
                        path: p.clone(),
                        status: "done".into(),
                        kind: "sheet".into(),
                        test_id: Some(test_id),
                    });
                }
            }
            Err(e) => {
                failed.push(FileFailure { path: p.clone(), error: e });
                logs.push(db::FileLogRow {
                    path: p.clone(),
                    status: "failed".into(),
                    kind: "sheet".into(),
                    test_id: Some(test_id),
                });
            }
        }
    }

    let _ = db::log_files(&app, &logs);
    let bound = db::count_files(&app, test_id, "sheet").unwrap_or(0);
    Ok(SheetsOutcome { test_id, total, reused, bound, failed })
}

/// 单项连通性测试结果（前端按行展示）
#[derive(Serialize)]
pub struct TestResult {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

fn clip(s: &str, n: usize) -> String {
    let s: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if s.chars().count() > n {
        s.chars().take(n).collect::<String>() + "…"
    } else {
        s
    }
}

/// 生成一张纯白测试图，仅用于连通性探测（不实际识别内容）
/// 生成一张测试图，仅用于连通性探测（不实际识别内容）。
/// 尺寸必须够大：智谱 OCR 对最小尺寸有要求，过小会返回 400 且报错文案误导为"格式不支持"。
/// 实测 320×120 会 400，1200×800 正常，这里取 1024×768 留足余量。
fn probe_png() -> Vec<u8> {
    let img = image::RgbImage::from_fn(1024, 768, |_x, _y| image::Rgb([255, 255, 255]));
    let mut buf: Vec<u8> = Vec::new();
    let _ = img.write_to(
        &mut std::io::Cursor::new(&mut buf),
        image::ImageFormat::Png,
    );
    buf
}

/// 一次测三项：DeepSeek / GLM-OCR / Qwen-VL-OCR，返回逐项结果
#[tauri::command]
pub async fn test_providers(app: tauri::AppHandle) -> Vec<TestResult> {
    let cfg = match config::load(&app) {
        Ok(c) => c,
        Err(e) => {
            return vec![TestResult { name: "配置".into(), ok: false, detail: clip(&e, 300) }]
        }
    };
    let png = probe_png();
    let mut out = Vec::new();

    out.push(match llm::chat(&cfg, None, "只回复两个字：正常").await {
        Ok(t) => TestResult { name: "DeepSeek V4 Flash".into(), ok: true, detail: clip(t.trim(), 20) },
        Err(e) => TestResult { name: "DeepSeek V4 Flash".into(), ok: false, detail: clip(&e, 300) },
    });
    out.push(match ocr::probe_ocr(&cfg, &png, "image/png").await {
        Ok(_) => TestResult { name: "GLM-OCR".into(), ok: true, detail: "连通".into() },
        Err(e) => TestResult { name: "GLM-OCR".into(), ok: false, detail: clip(&e, 300) },
    });
    out.push(match ocr::probe_vision(&cfg.ocr2, &png, "image/png").await {
        Ok(_) => TestResult { name: "Qwen-VL-OCR".into(), ok: true, detail: "连通".into() },
        Err(e) => TestResult { name: "Qwen-VL-OCR".into(), ok: false, detail: clip(&e, 300) },
    });
    out
}

