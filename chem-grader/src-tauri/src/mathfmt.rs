//! 把 GLM-OCR 输出的 LaTeX 化公式转成老师能直接读的形态。
//! GLM-OCR 是文档解析模型，公式天生输出 LaTeX（`$ \mathrm{F e}^{3+} $`），
//! 但化学老师要的是 `Fe³⁺`。这里全是本地字符串处理，不调任何模型。

/// 上标字符映射（化学里的 ³⁺ / ²⁻ 等）
fn sup(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴',
        '5' => '⁵', '6' => '⁶', '7' => '⁷', '8' => '⁸', '9' => '⁹',
        '+' => '⁺', '-' | '−' => '⁻', 'n' => 'ⁿ', 'i' => 'ⁱ',
        _ => return None,
    })
}

/// 下标字符映射（H₂O 的 ₂、N_A 的 ₐ 等）
fn sub(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄',
        '5' => '₅', '6' => '₆', '7' => '₇', '8' => '₈', '9' => '₉',
        '+' => '₊', '-' | '−' => '₋',
        // 字母下标（化学里常见：N_A 阿伏加德罗常数、K_sp 等）
        'a' | 'A' => 'ₐ', 'e' | 'E' => 'ₑ', 'h' => 'ₕ', 'i' | 'I' => 'ᵢ',
        'k' | 'K' => 'ₖ', 'l' => 'ₗ', 'm' => 'ₘ', 'n' | 'N' => 'ₙ',
        'o' | 'O' => 'ₒ', 'p' | 'P' => 'ₚ', 'r' => 'ᵣ', 's' | 'S' => 'ₛ',
        't' => 'ₜ', 'u' | 'U' => 'ᵤ', 'v' | 'V' => 'ᵥ', 'x' | 'X' => 'ₓ',
        _ => return None,
    })
}

fn circled(n: u32) -> Option<char> {
    Some(match n {
        1 => '①', 2 => '②', 3 => '③', 4 => '④', 5 => '⑤',
        6 => '⑥', 7 => '⑦', 8 => '⑧', 9 => '⑨', 10 => '⑩',
        11 => '⑪', 12 => '⑫', 13 => '⑬', 14 => '⑭', 15 => '⑮',
        16 => '⑯', 17 => '⑰', 18 => '⑱', 19 => '⑲', 20 => '⑳',
        _ => return None,
    })
}

/// 从 `start`（必须指向 `{`）取出配对花括号内的内容，返回 (内容, 结束后的偏移)
/// 支持嵌套：`{a_{2}}` 正确返回 `a_{2}`
fn take_group(s: &str, start: usize) -> Option<(String, usize)> {
    let mut depth = 0usize;
    let mut out = String::new();
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => {
                depth += 1;
                if depth > 1 {
                    out.push(c);
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((out, start + i + 1));
                }
                out.push(c);
            }
            _ => {
                if depth >= 1 {
                    out.push(c);
                }
            }
        }
    }
    None
}

/// 去掉 `$...$` 数学环境：既去壳，也把其中的空白全部删掉
/// （数学环境里的空格几乎都是 OCR 把字拆开产生的伪影：`F e` / `1 0^{3}`）
fn squeeze_math(s: &str) -> String {
    let s = s.replace("$$", "$");
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('$') {
            Some(j) => {
                let inner: String = after[..j].chars().filter(|c| !c.is_whitespace()).collect();
                out.push_str(&inner);
                rest = &after[j + 1..];
            }
            None => {
                rest = &rest[i..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 展开 `\cmd{...}` → `...`（递归处理嵌套）。
/// `math_mode=true` 时顺带删掉内容里的空白 —— `\mathrm{F e O}` 里的空格必然是伪影。
fn unwrap_cmd(s: &str, cmd: &str, math_mode: bool) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(cmd) {
        let after = &rest[i + cmd.len()..];
        let t = after.trim_start();
        if !t.starts_with('{') {
            out.push_str(&rest[..i + cmd.len()]);
            rest = after;
            continue;
        }
        let brace = rest.len() - t.len();
        out.push_str(&rest[..i]);
        match take_group(rest, brace) {
            Some((inner, end)) => {
                let inner = unwrap_cmd(&inner, cmd, math_mode);
                if math_mode {
                    out.extend(inner.chars().filter(|c| !c.is_whitespace()));
                } else {
                    out.push_str(&inner);
                }
                rest = &rest[end..];
            }
            None => {
                out.push_str(&rest[i..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `\frac{A}{B}` → `A/B`（A、B 含运算符时补括号）
fn fold_frac(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("\\frac") {
        let after = &rest[i + 5..];
        let t = after.trim_start();
        if !t.starts_with('{') {
            out.push_str(&rest[..i + 5]);
            rest = after;
            continue;
        }
        let Some((num, e1)) = take_group(rest, rest.len() - t.len()) else {
            out.push_str(&rest[i..]);
            return out;
        };
        let after2 = &rest[e1..];
        let t2 = after2.trim_start();
        if !t2.starts_with('{') {
            out.push_str(&rest[..e1]);
            rest = after2;
            continue;
        }
        let Some((den, e2)) = take_group(rest, rest.len() - t2.len()) else {
            out.push_str(&rest[i..]);
            return out;
        };
        out.push_str(&rest[..i]);
        let n = fold_frac(&num);
        let d = fold_frac(&den);
        out.push_str(&wrap_group(&n));
        out.push('/');
        out.push_str(&wrap_group(&d));
        rest = &rest[e2..];
    }
    out.push_str(rest);
    out
}

/// 多字符且含运算符时才补括号：(a-b) / (312) 里只有前者需要括号
fn wrap_group(x: &str) -> String {
    let n = x.chars().count();
    if n > 1 && x.chars().any(|c| matches!(c, '+' | '-' | '×' | '·' | '/' | ' ')) {
        format!("({x})")
    } else {
        x.to_string()
    }
}

/// `\sqrt[3]{x}` → `∛(x)`；`\sqrt{x}` → `√(x)`
fn fold_sqrt(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("\\sqrt") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 5..];
        let mut t = after.trim_start();
        let mut idx = String::new();
        if t.starts_with('[') {
            if let Some(j) = t.find(']') {
                idx = t[1..j].to_string();
                t = t[j + 1..].trim_start();
            }
        }
        if !t.starts_with('{') {
            out.push('√');
            rest = t;
            continue;
        }
        let Some((inner, end)) = take_group(rest, rest.len() - t.len()) else {
            out.push_str(&rest[i..]);
            return out;
        };
        match idx.as_str() {
            "3" => out.push('∛'),
            "4" => out.push('∜'),
            "" => out.push('√'),
            other => {
                out.push_str(other);
                out.push('√');
            }
        }
        out.push('(');
        out.push_str(&fold_sqrt(&inner));
        out.push(')');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// `^{3+}` → `³⁺`；`_{2}` → `₂`；映射不了就退回 `^(...)` / `_(...)`
fn fold_scripts(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(['^', '_']) {
        let ch = rest[i..].chars().next().unwrap_or('^');
        out.push_str(&rest[..i]);
        let after = &rest[i + ch.len_utf8()..];
        let t = after.trim_start();
        if t.starts_with('{') {
            match take_group(rest, rest.len() - t.len()) {
                Some((inner, end)) => {
                    let mapped: Option<String> = inner
                        .chars()
                        .map(|c| if ch == '^' { sup(c) } else { sub(c) })
                        .collect();
                    match mapped {
                        Some(m) if !m.is_empty() => out.push_str(&m),
                        _ => {
                            out.push(ch);
                            out.push('(');
                            out.push_str(&inner);
                            out.push(')');
                        }
                    }
                    rest = &rest[end..];
                }
                None => {
                    out.push(ch);
                    rest = after;
                }
            }
        } else {
            match t.chars().next() {
                Some(c) => {
                    let m = if ch == '^' { sup(c) } else { sub(c) };
                    match m {
                        Some(mm) => {
                            out.push(mm);
                            rest = &t[c.len_utf8()..];
                        }
                        None => {
                            out.push(ch);
                            rest = t;
                        }
                    }
                }
                None => {
                    out.push(ch);
                    rest = t;
                }
            }
        }
    }
    out.push_str(rest);
    out
}

/// `\textcircled{1}` / `\circled{10}` → ① / ⑩
fn fold_circled(s: &str) -> String {
    const CMDS: [&str; 2] = ["\\textcircled", "\\circled"];
    let mut out = String::new();
    let mut rest = s;
    loop {
        // 取最靠前的命中，命令长度直接用 cmd.len()，避免手数错
        let hit = CMDS
            .iter()
            .filter_map(|c| rest.find(c).map(|i| (i, *c)))
            .min_by_key(|(i, _)| *i);
        let Some((i, cmd)) = hit else { break };
        out.push_str(&rest[..i]);
        let after = &rest[i + cmd.len()..];
        let t = after.trim_start();
        if t.starts_with('{') {
            match take_group(rest, rest.len() - t.len()) {
                Some((inner, end)) => {
                    let n: u32 = inner.trim().parse().unwrap_or(0);
                    match circled(n) {
                        Some(c) => out.push(c),
                        None => {
                            out.push('(');
                            out.push_str(inner.trim());
                            out.push(')');
                        }
                    }
                    rest = &rest[end..];
                }
                None => rest = after,
            }
        } else {
            rest = t;
        }
    }
    out.push_str(rest);
    out
}

/// 主入口：把 LaTeX 化的识别结果转成可读文本
pub fn humanize(input: &str) -> String {
    let mut t = squeeze_math(input);

    // 常见符号
    for (from, to) in [
        ("\\times", "×"),
        ("\\cdot", "·"),
        ("\\longrightarrow", "→"),
        ("\\rightarrow", "→"),
        ("\\to", "→"),
        ("\\Delta", "Δ"),
        ("\\uparrow", "↑"),
        ("\\downarrow", "↓"),
        ("\\left", ""),
        ("\\right", ""),
        ("\\,", ""),
        ("\\;", ""),
        ("\\!", ""),
        ("\\quad", " "),
    ] {
        t = t.replace(from, to);
    }

    t = fold_circled(&t);

    // 展开字体/文本命令（math_mode：内容里的空格一律视为伪影）
    for cmd in ["\\mathrm", "\\text", "\\mathbf", "\\mathit", "\\mathsf", "\\operatorname", "\\ce"] {
        t = unwrap_cmd(&t, cmd, true);
    }

    t = fold_frac(&t);
    t = fold_sqrt(&t);
    t = fold_scripts(&t);

    // 收尾：多空格合一、去掉行首尾
    let mut prev_space = false;
    let mut cleaned = String::new();
    for c in t.chars() {
        if c.is_whitespace() {
            if !prev_space {
                cleaned.push(' ');
            }
            prev_space = true;
        } else {
            cleaned.push(c);
            prev_space = false;
        }
    }
    cleaned.trim().to_string()
}
