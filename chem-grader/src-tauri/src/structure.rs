use crate::config::Config;
use crate::llm;
use serde::{Deserialize, Serialize};

/// 一道结构化答案。score / type / rubric 缺失时为 null（前端据此打「需确认」）
#[derive(Serialize, Deserialize, Clone)]
pub struct Question {
    pub no: String,
    /// 标准答案；**原文没有就为 null**（禁止模型自行作答）
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(rename = "type", default)]
    pub qtype: Option<String>,
    #[serde(default)]
    pub rubric: Option<String>,
    /// 双读一致性：None=未做第二读；Some(false)=两份 OCR 对不上，需人工复核
    #[serde(default)]
    pub agree: Option<bool>,
    /// 原文校验未通过：答案在原文里找不到出处，已被清空（疑似模型编造）
    #[serde(default)]
    pub unverified: bool,
    /// 该题答案在**原文**里的上下文片段（供老师核对，只读）
    #[serde(default)]
    pub src: Option<String>,
    /// 双读不符时，**第二读文本**里该题附近的片段（供对比）
    #[serde(default)]
    pub cross_src: Option<String>,
}

const SYSTEM: &str = "你是试卷答案整理助手。用户会给一份**答案文档**的原文。\
你的唯一任务是把原文里**已经存在**的内容整理成结构化 JSON。\
绝对不允许自己解题、补全、改写或推测答案。\
字段：no(题号,字符串，如 \"16(1)\")、answer(标准答案,字符串或null)、\
score(分值,数字或null)、\
type(题型：优先从 选择题/填空题/化学方程式/计算题/实验题/推断题/有机合成/简答题 里选，\
都不合适时可自拟一个简短类型名，判断不了就 null)、\
rubric(评分细则,字符串或null)。\
铁律：\
1) answer 必须**逐字来自原文**；原文没有给出答案时，answer 填 null。\
2) score 只在原文明确写出分值时才填，否则 null。\
3) **严禁凭化学知识补全答案**——宁可留空，也不要猜。\
只输出 JSON 数组，不要任何解释、不要代码块围栏。";

/// 把一份答案文本整份发给 LLM，产出结构化题目数组
pub async fn structure_answers(cfg: &Config, text: &str) -> Result<Vec<Question>, String> {
    let raw = llm::chat(cfg, Some(SYSTEM), text).await?;
    let json = extract_json(&raw);
    serde_json::from_str::<Vec<Question>>(&json)
        .map_err(|e| format!("结构化解析失败: {e}\n模型原始输出: {raw}"))
}

/// 从模型输出里抠出 JSON 数组（容忍代码块围栏与前后缀文字）
fn extract_json(s: &str) -> String {
    let t = s.trim();
    let t = t
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let (Some(a), Some(b)) = (t.find('['), t.rfind(']')) {
        if b > a {
            return t[a..=b].to_string();
        }
    }
    t.to_string()
}
