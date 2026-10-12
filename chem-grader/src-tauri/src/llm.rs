use crate::config::Config;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Msg<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<Msg<'a>>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Content,
}

#[derive(Deserialize)]
struct Content {
    content: String,
}

/// OpenAI 兼容的 `chat/completions` 调用（DeepSeek 与智谱都适用）。
pub async fn chat(cfg: &Config, system: Option<&str>, user: &str) -> Result<String, String> {
    let p = &cfg.llm;
    if p.api_key.is_empty() {
        return Err("未配置 LLM API Key（设置 → API 配置）".into());
    }
    let url = format!("{}/chat/completions", p.base_url.trim_end_matches('/'));
    let mut messages = Vec::new();
    if let Some(s) = system {
        messages.push(Msg { role: "system", content: s });
    }
    messages.push(Msg { role: "user", content: user });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", p.api_key))
        .json(&ChatReq { model: &p.model, messages, temperature: 0.2 })
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }
    let parsed: ChatResp =
        serde_json::from_str(&text).map_err(|e| format!("LLM 响应解析失败: {e} | {text}"))?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| format!("LLM 响应为空 | {text}"))
}
