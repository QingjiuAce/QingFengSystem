use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 一个 API 提供方（端点 + Key + 模型）
#[derive(Serialize, Deserialize, Clone)]
pub struct Provider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

fn default_ocr() -> Provider {
    Provider {
        base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
        api_key: String::new(),
        model: "glm-ocr".into(),
    }
}
fn default_llm() -> Provider {
    Provider {
        base_url: "https://api.deepseek.com/v1".into(),
        api_key: String::new(),
        model: "deepseek-chat".into(),
    }
}
fn default_ocr2() -> Provider {
    Provider {
        base_url: "https://maas.qianwenaiapi.com/compatible-mode/v1".into(),
        api_key: String::new(),
        model: "qwen-vl-ocr".into(),
    }
}

/// 应用配置：主 OCR（智谱 GLM-OCR）/ LLM / 第二 OCR（Qwen-VL-OCR，供双读交叉验证）
/// 三个字段都带 serde default —— 旧 config.json 缺字段也能正常解析
#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_ocr")]
    pub ocr: Provider,
    #[serde(default = "default_llm")]
    pub llm: Provider,
    #[serde(default = "default_ocr2")]
    pub ocr2: Provider,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ocr: default_ocr(),
            llm: default_llm(),
            ocr2: default_ocr2(),
        }
    }
}

pub fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

pub fn load(app: &AppHandle) -> Result<Config, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut cfg: Config =
        serde_json::from_str(&text).map_err(|e| format!("config.json 解析失败: {e}"))?;
    // 迁移：第二 OCR 若仍是旧默认模型名（qwen3-vl-plus），自动升级到 qwen-vl-ocr
    if cfg.ocr2.model.trim().is_empty() || cfg.ocr2.model == "qwen3-vl-plus" {
        cfg.ocr2.model = "qwen-vl-ocr".to_string();
    }
    Ok(cfg)
}

pub fn save(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let path = config_path(app)?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}
