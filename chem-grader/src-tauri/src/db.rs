use rusqlite::Connection;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 数据库文件位于用户数据目录，避免写入只读的安装目录
pub fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("qingfeng.db"))
}

/// 初始化数据库：开启 WAL、外键，建表建索引
pub fn init(app: &AppHandle) -> Result<(), String> {
    let path = db_path(app)?;
    let conn = Connection::open(&path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    migrate(&conn)?;
    Ok(())
}

/// 老库升级：SQLite 的 ADD COLUMN 对已存在的列会报错，所以逐个忽略失败。
/// **必须在 execute_batch(SCHEMA) 之后、且任何依赖新列的索引之前执行。**
fn migrate(conn: &Connection) -> Result<(), String> {
    for sql in [
        "ALTER TABLE file_log ADD COLUMN test_id INTEGER",
        "ALTER TABLE file_log ADD COLUMN kind TEXT NOT NULL DEFAULT 'answer'",
    ] {
        let _ = conn.execute(sql, []);
    }
    // 新列这时才保证存在，索引放这里建
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_file_log_test ON file_log(test_id, kind)",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 导入日志的一行：源文件路径 + 处理结果 + 分类/绑定
pub struct FileLogRow {
    pub path: String,
    /// done / failed
    pub status: String,
    /// answer（答案文件）/ sheet（答题卡扫描件）
    pub kind: String,
    /// 绑定到哪一场考试；答案文件在导入时还不知道 test_id，保存后再回填
    pub test_id: Option<i64>,
}

fn path_hash(path: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_ascii_lowercase().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", h)
}

/// 记录本次导入用到的源文件（同路径覆盖旧记录）。
/// 纯记录用途，失败不影响导入主流程 —— 调用方应忽略返回值。
pub fn log_files(app: &AppHandle, rows: &[FileLogRow]) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let conn = Connection::open(db_path(app)?).map_err(|e| e.to_string())?;
    for r in rows {
        conn.execute(
            "INSERT INTO file_log (hash, path, status, kind, test_id) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(hash) DO UPDATE SET path = ?2, status = ?3, kind = ?4, \
             test_id = COALESCE(?5, file_log.test_id), \
             processed_at = datetime('now','localtime')",
            rusqlite::params![path_hash(&r.path), r.path, r.status, r.kind, r.test_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 【绑定】把一批文件（答案文件 / 答题卡）挂到某场考试上。
/// 答案文件在「确认并保存」后回填 test_id，这就是"缓存绑定"的落点。
pub fn bind_files(
    app: &AppHandle,
    test_id: i64,
    paths: &[String],
    kind: &str,
) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let conn = Connection::open(db_path(app)?).map_err(|e| e.to_string())?;
    for p in paths {
        conn.execute(
            "UPDATE file_log SET test_id = ?1, kind = ?2 WHERE hash = ?3",
            rusqlite::params![test_id, kind, path_hash(p)],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 某场考试已绑定的文件数（按分类）
pub fn count_files(app: &AppHandle, test_id: i64, kind: &str) -> Result<i64, String> {
    let conn = Connection::open(db_path(app)?).map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT COUNT(*) FROM file_log WHERE test_id = ?1 AND kind = ?2 AND status = 'done'",
        rusqlite::params![test_id, kind],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// 保存某任务的答案（幂等）：按任务名复用已有 test 行，替换其 question 行。
/// 返回 test_id。
pub fn save_answers(
    app: &AppHandle,
    task: &str,
    questions: &[crate::structure::Question],
) -> Result<i64, String> {
    use rusqlite::{params, OptionalExtension};

    let mut conn = Connection::open(db_path(app)?).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let existing: Option<i64> = tx
        .query_row("SELECT id FROM test WHERE name = ?1 LIMIT 1", [task], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;

    let test_id = match existing {
        Some(id) => {
            tx.execute("DELETE FROM question WHERE test_id = ?1", [id])
                .map_err(|e| e.to_string())?;
            id
        }
        None => {
            tx.execute("INSERT INTO test (name) VALUES (?1)", [task])
                .map_err(|e| e.to_string())?;
            tx.last_insert_rowid()
        }
    };

    for q in questions {
        tx.execute(
            "INSERT INTO question (test_id, no, answer, score, type, rubric) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                test_id,
                q.no,
                q.answer,
                q.score.unwrap_or(0.0), // 分值可能是 0.5 / 1.5 这类小数，不能取整
                q.qtype,
                q.rubric
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(test_id)
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS test (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT    NOT NULL,
  class_name  TEXT,
  total_score INTEGER NOT NULL DEFAULT 100,
  created_at  TEXT    NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS question (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  test_id INTEGER NOT NULL REFERENCES test(id) ON DELETE CASCADE,
  no      TEXT    NOT NULL,
  answer  TEXT,
  score   REAL    NOT NULL DEFAULT 0,
  type    TEXT,
  rubric  TEXT
);
CREATE INDEX IF NOT EXISTS idx_question_test ON question(test_id);

CREATE TABLE IF NOT EXISTS submission (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  test_id    INTEGER NOT NULL REFERENCES test(id) ON DELETE CASCADE,
  sid        TEXT,
  name       TEXT,
  file_path  TEXT,
  total      INTEGER,
  status     TEXT    NOT NULL DEFAULT 'pending',
  created_at TEXT    NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_submission_test ON submission(test_id);

CREATE TABLE IF NOT EXISTS answer (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  submission_id INTEGER NOT NULL REFERENCES submission(id) ON DELETE CASCADE,
  q_no          TEXT    NOT NULL,
  ocr_text      TEXT,
  score         REAL,
  verdict       TEXT,
  confidence    REAL,
  needs_review  INTEGER NOT NULL DEFAULT 0,
  reason        TEXT
);
CREATE INDEX IF NOT EXISTS idx_answer_submission ON answer(submission_id);
CREATE INDEX IF NOT EXISTS idx_answer_review ON answer(needs_review);

CREATE TABLE IF NOT EXISTS file_log (
  hash         TEXT PRIMARY KEY,
  path         TEXT NOT NULL,
  processed_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  status       TEXT NOT NULL DEFAULT 'done',
  test_id      INTEGER REFERENCES test(id) ON DELETE SET NULL,
  kind         TEXT NOT NULL DEFAULT 'answer'
);
-- 注意：idx_file_log_test 依赖上面两列，**不能在 SCHEMA 里建** ——
-- CREATE TABLE IF NOT EXISTS 对已存在的旧表是 no-op，此时新列还不存在。
-- 该索引统一放到 migrate() 的 ALTER 之后创建。
"#;
