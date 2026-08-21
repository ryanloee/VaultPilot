//! Quick capture: append a timestamped text entry to a daily note or inbox.
//!
//! Used by both the CLI (`vaultpilot capture`) and the WinUI JSON-RPC backend
//! (`vaultpilot-agent` "capture" method). #2969

use anyhow::Result;
use chrono::Local;
use serde_json::Value;

use crate::models::NoteDocument;
use crate::storage::{load_note_with_context, save_note_with_context, NoteNotFound};

/// Append a one-line text capture to today's daily note or inbox.
///
/// Creates the target note (and the capture section) if they don't exist yet,
/// then saves through `save_note_with_context` which triggers incremental
/// indexing so the captured text is immediately searchable.
pub fn handle_capture(
    context: &crate::storage::StorageContext,
    text: &str,
    target: &str,
    section: &str,
) -> Result<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        anyhow::bail!("capture text is empty");
    }

    let now = Local::now();
    let note_id = match target {
        "daily" => format!("Daily/{}", now.format("%Y-%m-%d")),
        "inbox" => "Inbox".to_string(),
        other => anyhow::bail!("unknown capture target '{other}': expected 'daily' or 'inbox'"),
    };
    let timestamp = now.format("%H:%M").to_string();

    let (note, existed) = match load_note_with_context(context, &note_id) {
        Ok(mut doc) => {
            doc.body = append_capture_entry(&doc.body, section, &timestamp, trimmed);
            doc.meta.updated_at = chrono::Utc::now().to_rfc3339();
            (doc, true)
        }
        Err(ref e) if e.downcast_ref::<NoteNotFound>().is_some() => {
            // Note genuinely doesn't exist — create a new one.
            let (title, tags) = if target == "daily" {
                (
                    now.format("%Y-%m-%d").to_string(),
                    vec!["daily".to_string()],
                )
            } else {
                ("Inbox".to_string(), vec!["inbox".to_string()])
            };
            let body = append_capture_entry("", section, &timestamp, trimmed);
            let now_rfc = chrono::Utc::now().to_rfc3339();
            let note = NoteDocument {
                meta: crate::models::NoteMeta {
                    id: note_id.clone(),
                    title,
                    tags,
                    summary: String::new(),
                    source: String::new(),
                    created_at: now_rfc.clone(),
                    updated_at: now_rfc,
                    ..Default::default()
                },
                body,
                search_snippet: None,
                search_score: None,
            };
            (note, false)
        }
        // IO/parse errors must propagate — silently creating a duplicate note
        // would violate the "append to today's journal" contract.
        Err(e) => return Err(e),
    };

    let saved = save_note_with_context(context, note)?;
    Ok(serde_json::json!({
        "status": if existed { "appended" } else { "created" },
        "noteId": note_id,
        "note_id": note_id,
        "target": target,
        "section": section,
        "timestamp": timestamp,
        "captured": trimmed,
        "title": saved.meta.title,
    }))
}

/// Append a timestamped bullet under the given section heading.
///
/// If the section heading does not exist, it is appended at the end of the
/// body (after the last non-blank line).  If the section already exists,
/// the bullet is inserted right after the last non-blank line before the
/// next `## ` heading (or EOF).
fn append_capture_entry(body: &str, section: &str, timestamp: &str, text: &str) -> String {
    let bullet = format!("- {timestamp} {text}");
    let heading = format!("## {section}");
    let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();

    match lines.iter().position(|l| l.trim() == heading) {
        None => {
            if !lines.is_empty() {
                while lines.last().is_some_and(|s| s.trim().is_empty()) {
                    lines.pop();
                }
                if !lines.is_empty() {
                    lines.push(String::new());
                }
            }
            lines.push(heading);
            lines.push(bullet);
        }
        Some(head_idx) => {
            // Locate the end of this section: the next "## " heading, or EOF.
            let mut end = lines.len();
            for (offset, line) in lines.iter().enumerate().skip(head_idx + 1) {
                if line.starts_with("## ") {
                    end = offset;
                    break;
                }
            }
            // Insert right after the last non-blank line within the section.
            let mut insert_at = head_idx + 1;
            for (offset, line) in lines.iter().enumerate().take(end).skip(head_idx + 1) {
                if !line.trim().is_empty() {
                    insert_at = offset + 1;
                }
            }
            lines.insert(insert_at, bullet);
        }
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_capture_entry_creates_section_when_missing() {
        let body = "# Hello\n\nSome text\n";
        let result = append_capture_entry(body, "捕获", "12:00", "test note");
        assert!(result.contains("## 捕获"));
        assert!(result.contains("- 12:00 test note"));
    }

    #[test]
    fn append_capture_entry_appends_to_existing_section() {
        let body = "# Hello\n\n## 捕获\n- 11:00 old note\n\n## Other\n";
        let result = append_capture_entry(body, "捕获", "12:00", "new note");
        assert!(result.contains("- 11:00 old note\n- 12:00 new note"));
    }

    #[test]
    fn append_capture_entry_does_not_cross_section_boundary() {
        let body = "## 捕获\n- 11:00 note\n\n## Tasks\n- task 1\n";
        let result = append_capture_entry(body, "捕获", "12:00", "new");
        // "new" should be in 捕获 section, not Tasks
        let cap_pos = result.find("- 12:00 new").unwrap();
        let tasks_pos = result.find("- task 1").unwrap();
        assert!(
            cap_pos < tasks_pos,
            "new entry should appear before Tasks section"
        );
    }
}
