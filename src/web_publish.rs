//! Web Publish — 将单篇 Markdown 笔记渲染为自包含 HTML 页面 (#2811).
//!
//! MVP: 纯 Rust、零外部 crate、处理 YAML frontmatter + 基本 Markdown 语法
//! + `[[wikilinks]]`。输出单文件 HTML（内联 CSS）。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::info;

/// 核心公开函数：发布一篇笔记到指定输出目录。
///
/// 返回最终的 HTML 文件路径。
pub fn publish_note(
    settings_vault_dir: &Path,
    note_rel_path: &str,
    output_root: &Path,
) -> Result<PathBuf> {
    let note_abs = resolve_note_path(settings_vault_dir, note_rel_path)?;
    let raw = fs::read_to_string(&note_abs)
        .with_context(|| format!("failed to read note at {}", note_abs.display()))?;
    let normalized = raw.replace("\r\n", "\n");

    let (frontmatter, body) = crate::storage::notes::split_frontmatter(&normalized)
        .with_context(|| "failed to parse frontmatter")?;

    let title = if frontmatter.title.trim().is_empty() {
        crate::storage::notes::detect_title(body, &note_abs)
    } else {
        frontmatter.title.trim().to_string()
    };

    let html = compose_html_document(&title, body, &frontmatter);

    // 输出目录命名：使用 note_rel_path 的 stem
    let slug = slugify(note_rel_path);
    let out_dir = output_root.join(&slug);
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create output dir {}", out_dir.display()))?;

    let out_file = out_dir.join("index.html");
    fs::write(&out_file, &html)
        .with_context(|| format!("failed to write {}", out_file.display()))?;

    info!(path = %out_file.display(), "Published note");
    Ok(out_file)
}

// ── 内部辅助 ─────────────────────────────────────────────────────

/// 解析输入笔记路径：如果以 `vault:` 开头，去掉前缀并以 vault_dir 为基路径；
/// 否则解释为绝对路径或当前目录下的相对路径。
fn resolve_note_path(vault_dir: &Path, input: &str) -> Result<PathBuf> {
    let stripped = input.strip_prefix("vault:").unwrap_or(input);
    let candidate = Path::new(stripped);
    if candidate.is_absolute() {
        Ok(candidate.to_owned())
    } else {
        Ok(vault_dir.join(stripped))
    }
}

/// 从路径生成 URL-safe slug（去掉扩展名、替换特殊字符）。
///
/// 使用完整相对路径（而非仅文件名）生成 slug，确保不同目录下的同名笔记
/// 映射到不同的输出目录（#2854）。
fn slugify(raw: &str) -> String {
    // 去掉可选的 vault: 前缀
    let stripped = raw.strip_prefix("vault:").unwrap_or(raw);
    // 去掉文件扩展名
    let noext = stripped
        .rsplit_once('.')
        .map(|(s, _ext)| s)
        .unwrap_or(stripped);
    // 将目录分隔符替换为连字符，保留目录层级信息
    noext
        .replace('/', "-")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase()
}

// ── Markdown → HTML 渲染（纯 Rust） ─────────────────────────────

/// 组装完整 HTML 文档（内联 CSS）。
fn compose_html_document(title: &str, body: &str, fm: &crate::storage::Frontmatter) -> String {
    let rendered = render_body_html(body);
    let source_note = fm.source.trim();
    let source_html = if source_note.is_empty() {
        String::new()
    } else {
        format!(
            r#"    <div class="source">📄 Source: <code>vault:{}</code></div>
"#,
            html_escape(source_note)
        )
    };

    let tags_html = if fm.tags.is_empty() {
        String::new()
    } else {
        let tags: Vec<String> = fm
            .tags
            .iter()
            .map(|t| format!("<span class=\"tag\">{}</span>", html_escape(t)))
            .collect();
        format!(
            r#"    <div class="tags">{}</div>
"#,
            tags.join(" ")
        )
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{title}</title>
<style>
{STYLE_CSS}
</style>
</head>
<body>
<article>
    <header>
        <h1>{title}</h1>
{source_html}{tags_html}    </header>
    <div class="content">
{rendered}
    </div>
</article>
<footer>
    <p>Published by <a href="https://github.com/ryanloee/VaultPilot">VaultPilot</a> Web Publish (MVP)</p>
</footer>
</body>
</html>"#,
        title = html_escape(title),
        source_html = source_html,
        tags_html = tags_html,
        rendered = rendered,
    )
}

/// 将 Markdown body 转换为 HTML 片段（不包含 document wrapper）。
fn render_body_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_content = String::new();
    let mut code_fence_count = 0u32;
    let mut in_paragraph = false;
    let mut in_list = false;

    for line in text.lines() {
        let trimmed = line.trim();

        // ── 代码块 fence ──
        if let Some(rest) = trimmed.strip_prefix("```") {
            if !in_code_block {
                // 打开代码块
                close_list(&mut out, &mut in_list);
                in_code_block = true;
                code_lang = rest.trim().to_string();
                code_content.clear();
                code_fence_count = 1;
                // 不输出开始标记
                continue;
            } else if code_fence_count == 1 {
                // 关闭代码块（第一个 lone fence 线，在块内暂停后）
                // 检查是否真的是关闭 fence（周围没有更多 fences）
                if trimmed == "```" {
                    close_paragraph(&mut out, &mut in_paragraph);
                    close_list(&mut out, &mut in_list);
                    out.push_str(&render_code_block(&code_lang, &code_content));
                    in_code_block = false;
                    code_lang.clear();
                    code_content.clear();
                    code_fence_count = 0;
                    continue;
                }
            }
        }

        if in_code_block {
            if !code_content.is_empty() {
                code_content.push('\n');
            }
            code_content.push_str(line);
            // 检测 fence 计数 —— 连续的 ``` 线可能形成 fence 序列
            if trimmed == "```" {
                code_fence_count += 1;
            } else {
                code_fence_count = 0;
            }
            continue;
        }

        // ── 空行 → 关闭段落 ──
        if trimmed.is_empty() {
            close_paragraph(&mut out, &mut in_paragraph);
            close_list(&mut out, &mut in_list);
            continue;
        }

        // ── 标题 ──
        if let Some(heading) = detect_heading(trimmed) {
            close_paragraph(&mut out, &mut in_paragraph);
            close_list(&mut out, &mut in_list);
            out.push_str(&heading);
            continue;
        }

        // ── 水平分割线 ──
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            close_paragraph(&mut out, &mut in_paragraph);
            close_list(&mut out, &mut in_list);
            out.push_str("<hr>\n");
            continue;
        }

        // ── 无序列表（连续条目合并为同一个 <ul>，#2837） ──
        if let Some(item) = detect_unordered_list(trimmed) {
            close_paragraph(&mut out, &mut in_paragraph);
            if !in_list {
                out.push_str("<ul>\n");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>\n", item));
            continue;
        }

        // ── 引用块 ──
        if let Some(quoted) = trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix('>'))
        {
            close_paragraph(&mut out, &mut in_paragraph);
            close_list(&mut out, &mut in_list);
            out.push_str(&format!(
                "<blockquote><p>{}</p></blockquote>\n",
                inline_format(quoted)
            ));
            continue;
        }

        // ── 普通段落行 ──
        if !in_paragraph {
            close_list(&mut out, &mut in_list);
            in_paragraph = true;
            out.push_str("<p>");
        } else {
            out.push(' ');
        }
        out.push_str(&inline_format(trimmed));
    }

    // 处理未关闭的代码块（文档末尾仍在块中）
    if in_code_block && !code_content.is_empty() {
        close_paragraph(&mut out, &mut in_paragraph);
        out.push_str(&render_code_block(&code_lang, &code_content));
    }

    close_list(&mut out, &mut in_list);
    close_paragraph(&mut out, &mut in_paragraph);
    out
}

fn close_paragraph(out: &mut String, in_paragraph: &mut bool) {
    if *in_paragraph {
        out.push_str("</p>\n");
        *in_paragraph = false;
    }
}

fn close_list(out: &mut String, in_list: &mut bool) {
    if *in_list {
        out.push_str("</ul>\n");
        *in_list = false;
    }
}

/// 将 char 索引（基于 text.chars() 的位置）转换为 text 的字节索引。
/// URL 切分时必须用 byte 索引切 &str，否则非 ASCII 前缀会 panic（#2836）。
fn char_idx_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

// ── 行级格式化 ──

fn detect_heading(line: &str) -> Option<String> {
    for (prefix, level) in [
        ("###### ", 6),
        ("##### ", 5),
        ("#### ", 4),
        ("### ", 3),
        ("## ", 2),
        ("# ", 1),
    ] {
        if let Some(content) = line.strip_prefix(prefix) {
            return Some(format!(
                "<h{level}>{content}</h{level}>\n",
                level = level,
                content = inline_format(content)
            ));
        }
    }
    None
}

fn detect_unordered_list(line: &str) -> Option<String> {
    let content = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))?;
    Some(inline_format(content))
}

/// 行内格式化：**粗体**、*斜体*、`代码`、[[wikilinks]]、URL。
fn inline_format(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // **bold**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing(&chars, i + 2, "**") {
                let inner = inline_format(&chars[i + 2..end].iter().collect::<String>());
                out.push_str(&format!("<strong>{}</strong>", inner));
                i = end + 2;
                continue;
            }
        }

        // *italic*
        if chars[i] == '*' {
            if let Some(end) = find_closing(&chars, i + 1, "*") {
                let inner = inline_format(&chars[i + 1..end].iter().collect::<String>());
                out.push_str(&format!("<em>{}</em>", inner));
                i = end + 1;
                continue;
            }
        }

        // `code`
        if chars[i] == '`' {
            if let Some(end) = find_closing(&chars, i + 1, "`") {
                let inner: String = chars[i + 1..end].iter().collect();
                out.push_str(&format!("<code>{}</code>", html_escape(&inner)));
                i = end + 1;
                continue;
            }
        }

        // [[wikilink]]
        if i + 1 < len && chars[i] == '[' && chars[i + 1] == '[' {
            if let Some(end) = find_closing(&chars, i + 2, "]]") {
                let link_text: String = chars[i + 2..end].iter().collect();
                // 处理别名： [[target|display]]
                let (target, display) = if let Some(pos) = link_text.find('|') {
                    (&link_text[..pos], &link_text[pos + 1..])
                } else {
                    (link_text.as_str(), link_text.as_str())
                };
                out.push_str(&format!(
                    r#"<a class="wikilink" href="javascript:void(0)" title="vault:{}">{}</a>"#,
                    html_escape(target),
                    html_escape(display)
                ));
                i = end + 2;
                continue;
            }
        }

        // URL (heuristic: starts with http)
        // 注意：i 是 char 索引（基于 chars Vec），必须用 byte 索引切 text，
        // 否则非 ASCII 前缀会 panic 或误切字符串（#2836）。
        if chars[i] == 'h' && i + 4 < len {
            let is_http = chars[i] == 'h'
                && chars[i + 1] == 't'
                && chars[i + 2] == 't'
                && chars[i + 3] == 'p';
            if is_http {
                let byte_i = char_idx_to_byte(text, i);
                let byte_end = text[byte_i..]
                    .find(|c: char| c.is_whitespace())
                    .map(|p| byte_i + p)
                    .unwrap_or(text.len());
                let url = &text[byte_i..byte_end];
                out.push_str(&format!(
                    r#"<a href="{}" target="_blank" rel="noopener">{}</a>"#,
                    html_escape(url),
                    html_escape(url)
                ));
                // 把 i 推进到 URL 结尾对应的 char 索引
                i = text[..byte_end].chars().count();
                continue;
            }
        }

        // 普通正文文本：必须转义，否则字面 `<` `>` `&` `"` 会注入到
        // 生成的 HTML 中（存储型 XSS，见 #2830）。
        out.push_str(&html_escape(&chars[i].to_string()));
        i += 1;
    }
    out
}

/// 辅助：在 chars 切片中查找字面字符串，从 start 开始。
fn find_closing(chars: &[char], start: usize, delimiter: &str) -> Option<usize> {
    let delim_chars: Vec<char> = delimiter.chars().collect();
    let dlen = delim_chars.len();
    let mut idx = start;
    while idx + dlen <= chars.len() {
        if chars[idx..idx + dlen] == delim_chars[..] {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

/// HTML 转义 `&` `<` `>` `"`。
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// 渲染代码块（带语言标签 + 内联 CSS）。
fn render_code_block(lang: &str, content: &str) -> String {
    let lang_label = if lang.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="code-lang">{}</div>"#, html_escape(lang))
    };
    format!(
        r#"<div class="code-block">
{lang_label}<pre><code>{}</code></pre>
</div>
"#,
        html_escape(content)
    )
}

// ── 内联 CSS（MVP，不引入外部依赖） ──────────────────────────

const STYLE_CSS: &str = r##"body {
    max-width: 800px;
    margin: 0 auto;
    padding: 2rem 1.5rem;
    font-family: system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;
    font-size: 17px;
    line-height: 1.7;
    color: #1a1a1a;
    background: #fafafa;
}
header h1 {
    font-size: 2rem;
    margin-bottom: 0.3rem;
    border-bottom: 2px solid #e0e0e0;
    padding-bottom: 0.4rem;
}
.source {
    font-size: 0.85rem;
    color: #666;
    margin-bottom: 0.5rem;
}
.source code {
    background: #eee;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
}
.tags {
    margin-bottom: 1rem;
}
.tag {
    display: inline-block;
    background: #e8f0fe;
    color: #1a56db;
    padding: 0.15rem 0.6rem;
    margin-right: 0.3rem;
    border-radius: 4px;
    font-size: 0.8rem;
}
.content h2 { font-size: 1.5rem; margin-top: 1.8rem; border-bottom: 1px solid #e0e0e0; padding-bottom: 0.2rem; }
.content h3 { font-size: 1.3rem; margin-top: 1.5rem; }
.content h4 { font-size: 1.1rem; margin-top: 1.2rem; }
.content a { color: #1a56db; text-decoration: none; border-bottom: 1px dotted #1a56db; }
.content a.wikilink { color: #047857; border-bottom: 1px dotted #047857; }
.content a:hover { border-bottom-style: solid; }
.content blockquote {
    border-left: 4px solid #ccc;
    margin: 1rem 0;
    padding: 0.5rem 1rem;
    background: #f5f5f5;
    color: #555;
}
.content blockquote p { margin: 0; }
.content ul { margin: 0.5rem 0; }
.content li { margin: 0.2rem 0; }
.code-block {
    margin: 1rem 0;
    border: 1px solid #e0e0e0;
    border-radius: 6px;
    overflow: hidden;
    background: #f8f8f8;
}
.code-lang {
    background: #e8e8e8;
    color: #666;
    font-size: 0.8rem;
    padding: 0.2rem 0.8rem;
    border-bottom: 1px solid #e0e0e0;
}
.code-block pre {
    margin: 0;
    padding: 0.8rem 1rem;
    overflow-x: auto;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 0.9rem;
    line-height: 1.5;
}
.code-block code {
    color: #333;
    background: transparent;
}
code {
    background: #eee;
    padding: 0.1rem 0.3rem;
    border-radius: 3px;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 0.9em;
}
footer {
    margin-top: 3rem;
    padding-top: 1rem;
    border-top: 1px solid #e0e0e0;
    font-size: 0.8rem;
    color: #999;
    text-align: center;
}
footer a { color: #666; }
@media (prefers-color-scheme: dark) {
    body { color: #ddd; background: #1a1a2e; }
    header h1 { border-bottom-color: #444; }
    .source { color: #aaa; }
    .source code { background: #333; }
    .tag { background: #1a3a5c; color: #7eb8da; }
    .content h2 { border-bottom-color: #444; }
    .content a { color: #7eb8da; }
    .content a.wikilink { color: #6ee7b7; }
    .content blockquote { background: #222; color: #bbb; border-left-color: #555; }
    .code-block { background: #151520; border-color: #333; }
    .code-lang { background: #2a2a3a; color: #aaa; border-bottom-color: #333; }
    .code-block code { color: #ddd; }
    code { background: #2a2a3a; }
    footer { border-top-color: #333; color: #777; }
    footer a { color: #aaa; }
}
"##;

// ── 测试 ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        // 普通文件 — 只有文件名
        assert_eq!(slugify("notes/rust-tips.md"), "notes-rust-tips");
        assert_eq!(slugify("Daily/2026-07-14.md"), "daily-2026-07-14");
        // 深层嵌套路径 — 保留完整目录层级
        assert_eq!(
            slugify("deep/nested/path/to/file.md"),
            "deep-nested-path-to-file"
        );
        // vault: 前缀
        assert_eq!(slugify("vault:projects/notes.md"), "projects-notes");
        // 无扩展名
        assert_eq!(slugify("noext/readme"), "noext-readme");
    }

    #[test]
    fn html_escape_ampersand() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }

    #[test]
    fn html_escape_tags() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn inline_format_bold() {
        assert_eq!(
            inline_format("hello **world** end"),
            "hello <strong>world</strong> end"
        );
    }

    #[test]
    fn inline_format_italic() {
        assert_eq!(
            inline_format("hello *world* end"),
            "hello <em>world</em> end"
        );
    }

    #[test]
    fn inline_format_code() {
        assert_eq!(
            inline_format("use `cargo` build"),
            "use <code>cargo</code> build"
        );
    }

    #[test]
    fn inline_format_wikilink() {
        let result = inline_format("see [[Rust Tips|tips]] for more");
        assert!(
            result.contains("class=\"wikilink\""),
            "expected wikilink class: {}",
            result
        );
        assert!(
            result.contains("tips"),
            "expected display text 'tips': {}",
            result
        );
        assert!(result.contains("Rust Tips"), "expected target: {}", result);
    }

    #[test]
    fn inline_format_wikilink_no_alias() {
        let result = inline_format("[[Rust]]");
        assert!(
            result.contains(">Rust<"),
            "expected display text: {}",
            result
        );
    }

    #[test]
    fn inline_format_url() {
        let result = inline_format("see https://example.com for docs");
        assert!(
            result.contains("href=\"https://example.com\""),
            "expected URL href: {}",
            result
        );
    }

    #[test]
    fn inline_format_url_after_nonascii() {
        // #2836: 非 ASCII 前缀（中文）后出现 URL，不能把 char 索引当 byte 索引切字符串
        let result = inline_format("参考 链接 https://example.com 查看");
        assert!(
            result.contains("href=\"https://example.com\""),
            "expected URL autolink: {}",
            result
        );
        assert!(result.contains("参考"), "prefix lost: {}", result);
    }

    #[test]
    fn inline_format_url_multibyte_prefix_no_panic() {
        // #2836: 多字节前缀导致 text[i..] byte 切片越界 panic
        let result = inline_format("你好世界 https://rust-lang.org 官网");
        assert!(
            result.contains("href=\"https://rust-lang.org\""),
            "expected autolink: {}",
            result
        );
    }

    #[test]
    fn inline_format_plain_text_escaped() {
        // #2830: 普通正文文本必须被 HTML 转义，否则字面 < > & " 会注入生成的
        // HTML（存储型 XSS）。
        let result = inline_format("a < b & c > d \"e\"");
        assert!(result.contains("&lt;"), "expected escaped '<': {}", result);
        assert!(result.contains("&amp;"), "expected escaped '&': {}", result);
        assert!(result.contains("&gt;"), "expected escaped '>': {}", result);
        assert!(
            result.contains("&quot;"),
            "expected escaped '\"': {}",
            result
        );
        // 确保没有原始的危险字符泄漏到输出
        assert!(!result.contains("< b"), "unescaped '<' leaked: {}", result);
    }

    #[test]
    fn render_body_html_escapes_script_injection() {
        // #2830: 整篇正文（含段落）中的 <script> 必须被转义，不能原样输出。
        let md = "# My Notes\n\nHello <script>alert('xss')</script> world";
        let html = render_body_html(md);
        assert!(
            html.contains("&lt;script&gt;"),
            "script not escaped: {}",
            html
        );
        assert!(
            !html.contains("<script>alert"),
            "raw <script> leaked into output: {}",
            html
        );
    }

    #[test]
    fn detect_heading_h1() {
        let result = detect_heading("# Hello").unwrap();
        assert_eq!(result, "<h1>Hello</h1>\n");
    }

    #[test]
    fn detect_heading_h3_with_format() {
        let result = detect_heading("### **bold** title").unwrap();
        assert_eq!(result, "<h3><strong>bold</strong> title</h3>\n");
    }

    #[test]
    fn detect_unordered_list_item() {
        let result = detect_unordered_list("- an item").unwrap();
        assert_eq!(result, "an item");
    }

    #[test]
    fn detect_unordered_list_star() {
        let result = detect_unordered_list("* star item").unwrap();
        assert_eq!(result, "star item");
    }

    #[test]
    fn render_body_html_merges_consecutive_list_items() {
        // #2837: 连续的无序列表条目应合并为同一个 <ul>
        let md = "- one\n- two\n- three";
        let html = render_body_html(md);
        let ul_count = html.matches("<ul>").count();
        assert_eq!(ul_count, 1, "expected single <ul>, got: {}", html);
        assert!(html.contains("<li>one</li>"), "missing item one: {}", html);
        assert!(html.contains("<li>two</li>"), "missing item two: {}", html);
        assert!(
            html.contains("<li>three</li>"),
            "missing item three: {}",
            html
        );
        assert!(html.contains("</ul>"), "missing closing </ul>: {}", html);
    }

    #[test]
    fn render_body_html_closes_list_on_blank_line() {
        // #2837: 列表后空行应关闭 <ul>
        let md = "- item\n\nparagraph";
        let html = render_body_html(md);
        assert!(
            html.contains("<ul>\n<li>item</li>\n</ul>"),
            "list not closed: {}",
            html
        );
        assert!(
            html.contains("<p>paragraph</p>"),
            "paragraph missing: {}",
            html
        );
    }

    #[test]
    fn render_body_html_heading_and_paragraph() {
        let md = "# Title\n\nSome paragraph text.";
        let html = render_body_html(md);
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<p>Some paragraph text.</p>"));
    }

    #[test]
    fn render_body_html_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let html = render_body_html(md);
        assert!(html.contains("class=\"code-block\""));
        assert!(html.contains("fn main()"));
        assert!(html.contains("rust"));
    }

    #[test]
    fn render_body_html_blockquote() {
        let md = "> quoted line";
        let html = render_body_html(md);
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("quoted line"));
    }

    #[test]
    fn render_body_html_horizontal_rule() {
        assert_eq!(render_body_html("---"), "<hr>\n");
    }

    #[test]
    fn compose_html_document_structure() {
        let fm = crate::storage::Frontmatter {
            title: String::new(),
            tags: vec!["rust".to_string()],
            source: "clippy.md".to_string(),
            ..Default::default()
        };
        let html = compose_html_document("Test Note", "# Hello\n\nWorld.", &fm);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>Test Note</title>"));
        assert!(html.contains("<h1>Test Note</h1>"));
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<p>World.</p>"));
        assert!(html.contains("📄 Source"));
        assert!(html.contains("<span class=\"tag\">rust</span>"));
    }

    #[test]
    fn resolve_note_path_vault_prefix() {
        let vault = Path::new("/home/user/vault");
        let result = resolve_note_path(vault, "vault:notes/test.md").unwrap();
        assert_eq!(result, PathBuf::from("/home/user/vault/notes/test.md"));
    }

    #[test]
    fn resolve_note_path_relative() {
        let vault = Path::new("/home/user/vault");
        let result = resolve_note_path(vault, "notes/test.md").unwrap();
        assert_eq!(result, PathBuf::from("/home/user/vault/notes/test.md"));
    }
}
