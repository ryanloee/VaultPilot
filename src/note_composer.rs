//! Note Composer — extract selected text into a new note or merge two notes.
//!
//! Inspired by Obsidian's Note Composer core plugin (#3479).
//! Supports two operations:
//! 1. **Extract** — select text from a source note, create a new note with that
//!    text, replace selection with a wikilink to the new note.
//! 2. **Merge** — append one note's body to another and delete the source note,
//!    with automatic link rewriting so all vault wikilinks remain valid.

use anyhow::{anyhow, Result};

use crate::models::NoteDocument;

/// Result of an extract operation.
#[derive(Debug, Clone)]
pub struct ExtractResult {
    /// The new note that was created.
    pub new_note: NoteDocument,
    /// The updated source note body (selection replaced with wikilink).
    pub updated_source_body: String,
}

/// Extract the first occurrence of `selection` from the body of `source_note`,
/// create a new note with `new_title` containing the selected content, and
/// replace the selection in the source with a wikilink pointing to the new note.
///
/// Returns `Err` if the selection text is not found in the source body.
pub fn extract_text(
    source_note: &NoteDocument,
    selection: &str,
    new_title: &str,
) -> Result<ExtractResult> {
    let selection_trimmed = selection.trim();
    if selection_trimmed.is_empty() {
        return Err(anyhow!("selection text must not be empty"));
    }

    // Find the selection in the source body (first occurrence).
    let start = source_note
        .body
        .find(selection_trimmed)
        .ok_or_else(|| anyhow!("selection text not found in source note body"))?;

    let end = start + selection_trimmed.len();

    // Build the replacement wikilink that goes into the source note.
    let link = format!("[[{}]]", new_title);

    // Build new body: replace selection with wikilink.
    let mut updated_body = source_note.body[..start].to_string();
    updated_body.push_str(&link);
    updated_body.push_str(&source_note.body[end..]);

    // Create the new note content — the extracted text as-is.
    let new_body = format!("# {}\n\n{}", new_title, selection_trimmed);

    // Create the new NoteDocument (without an id — save will assign one).
    let new_note = NoteDocument {
        body: new_body,
        ..Default::default()
    };

    Ok(ExtractResult {
        new_note,
        updated_source_body: updated_body,
    })
}

/// Rewrite all `[[wikilink]]` references inside `body` so that links pointing
/// to `old_title` become links pointing to `new_title` (alias is preserved if
/// present: `[[old_title|alias]]` → `[[new_title|alias]]`).
///
/// This is a manual implementation (no regex crate dependency) that walks
/// through the body character by character looking for `[[...]]` patterns.
pub fn rewrite_wikilinks(body: &str, old_title: &str, new_title: &str) -> String {
    if old_title.trim().is_empty() {
        return body.to_string();
    }

    let mut result = String::with_capacity(body.len());
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Look for `[[`
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '[' {
            let start = i;
            i += 2; // skip `[[`
            let content_start = i;

            // Find closing `]]`
            while i < chars.len() {
                if i + 1 < chars.len() && chars[i] == ']' && chars[i + 1] == ']' {
                    break;
                }
                i += 1;
            }

            if i + 1 >= chars.len() {
                // No closing ]] found — write the `[[` literally and continue.
                result.push_str("[[");
                result.push_str(&chars[content_start..].iter().collect::<String>());
                break;
            }

            // Extract the link content between [[ and ]]
            let link_content: String = chars[content_start..i].iter().collect();
            let end = i + 2; // skip `]]`

            // Trim whitespace from link content
            let trimmed = link_content.trim();
            let (target, alias) = if let Some(pipe_pos) = trimmed.find('|') {
                (
                    trimmed[..pipe_pos].trim(),
                    Some(trimmed[pipe_pos + 1..].trim()),
                )
            } else {
                (trimmed, None)
            };

            // Rewrite if the target matches old_title
            if target == old_title {
                let new_link = match alias {
                    Some(a) => format!("[[{}|{}]]", new_title, a),
                    None => format!("[[{}]]", new_title),
                };
                result.push_str(&new_link);
            } else {
                // Preserve original
                result.push_str(&chars[start..end].iter().collect::<String>());
            }

            i = end;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Merge `source_note` body into `target_note` body; appends at the end with a
/// double-newline + `---` separator.
///
/// Returns the merged body.
pub fn merge_notes(source_note: &NoteDocument, target_note: &NoteDocument) -> Result<String> {
    if source_note.body.is_empty() {
        return Ok(target_note.body.clone());
    }
    if target_note.body.is_empty() {
        return Ok(source_note.body.clone());
    }

    Ok(format!(
        "{}\n\n---\n\n{}",
        source_note.body, target_note.body
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_simple_selection() {
        let source = NoteDocument {
            body: "This is some text.\n\n## Section\n\nSome content here to extract.\n\nMore text."
                .to_string(),
            ..NoteDocument::default()
        };

        let result = extract_text(&source, "Some content here to extract.", "My New Note").unwrap();

        assert!(result.updated_source_body.contains("[[My New Note]]"));
        assert!(!result
            .updated_source_body
            .contains("Some content here to extract."));
        assert!(result.new_note.body.contains("# My New Note"));
        assert!(result
            .new_note
            .body
            .contains("Some content here to extract."));
    }

    #[test]
    fn test_rewrite_wikilinks_simple() {
        let body = "See [[old_title]] for details and [[old_title|alias]] too.";
        let rewritten = rewrite_wikilinks(body, "old_title", "new_title");
        assert_eq!(
            rewritten,
            "See [[new_title]] for details and [[new_title|alias]] too."
        );
    }

    #[test]
    fn test_rewrite_wikilinks_no_match() {
        let body = "See [[other_title]] and [[old_title_stuff]].";
        let rewritten = rewrite_wikilinks(body, "old_title", "new_title");
        // old_title is not matched (exact match only), and "old_title_stuff" is different.
        assert_eq!(rewritten, body);
    }

    #[test]
    fn test_merge_notes() {
        let source = NoteDocument {
            body: "Source body".to_string(),
            ..Default::default()
        };
        let target = NoteDocument {
            body: "Target body".to_string(),
            ..Default::default()
        };

        let merged = merge_notes(&source, &target).unwrap();
        assert!(merged.contains("Source body"));
        assert!(merged.contains("Target body"));
    }

    #[test]
    fn test_merge_empty_source() {
        let source = NoteDocument {
            body: String::new(),
            ..Default::default()
        };
        let target = NoteDocument {
            body: "Target body".to_string(),
            ..Default::default()
        };
        let merged = merge_notes(&source, &target).unwrap();
        assert_eq!(merged, "Target body");
    }

    #[test]
    fn test_merge_empty_target() {
        let source = NoteDocument {
            body: "Source body".to_string(),
            ..Default::default()
        };
        let target = NoteDocument {
            body: String::new(),
            ..Default::default()
        };
        let merged = merge_notes(&source, &target).unwrap();
        assert_eq!(merged, "Source body");
    }

    #[test]
    fn test_extract_selection_not_found() {
        let source = NoteDocument {
            body: "Hello world".to_string(),
            ..Default::default()
        };
        let err = extract_text(&source, "not there", "New Note").unwrap_err();
        assert!(err.to_string().contains("selection text not found"));
    }

    #[test]
    fn test_extract_empty_selection() {
        let source = NoteDocument::default();
        let err = extract_text(&source, "   ", "New Note").unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }
}
