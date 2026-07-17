//! Source-grounded citation extraction for AI answers (#2985).
//!
//! VaultPilot already injects RAG context from the vault into the system
//! prompt, but answers come back as plain text — the user cannot tell which
//! sentence came from which note. To close that gap we ask the model to mark
//! sources inline using two lightweight, parseable conventions:
//!
//! 1. Wikilink style (Obsidian-friendly, human-readable):
//!    `According to [[Note Title#Section]] the sky is blue.`
//!    The `#Section` anchor is optional.
//!
//! 2. Compact machine style:
//!    `[#cite:path/to/note.md:offset]`
//!    where `offset` is the 0-based character offset of the cited span inside
//!    the note body (used to pull a snippet). Both `:offset` and `:path` are
//!    optional.
//!
//! This module parses those markers out of raw model output into structured
//! [`crate::models::AnswerCitation`] objects and rewrites the answer text so the
//! markers are replaced by stable `[n]` footnotes. UI layers (mobile / WinUI)
//! render the footnote as a clickable link that jumps to the note.

use crate::models::AnswerCitation;

/// Resolves a citation label (note title or path fragment) to its canonical
/// `(note_id, title, path)` triple, using the same RAG index that produced the
/// answer's context. Returning `None` leaves the citation unresolved (raw
/// label kept) so the UI can still surface it.
pub type TitleResolver = dyn Fn(&str) -> Option<(String, String, String)>;

/// A single inline citation marker found in answer text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawCitation {
    /// Display label with the marker stripped (e.g. "Note Title#Section").
    label: String,
    /// Optional note path (compact style).
    path: Option<String>,
    /// Optional character offset into the note body (compact style).
    offset: Option<usize>,
}

/// Replace every citation marker in `text` with a `[n]` footnote, returning
/// the rewritten text together with the ordered list of raw citations.
///
/// Order is preserved: wikilink markers left-to-right, then compact markers
/// left-to-right, each receiving the next index. This matches how a reader
/// encounters the markers while scanning the answer.
fn rewrite_with_footnotes(text: &str) -> (String, Vec<RawCitation>) {
    // Collect every recognized marker together with the raw citation data it
    // represents. We keep spans and citations as parallel entries here, then
    // sort by text position so emission order (and thus footnote numbers)
    // always matches `cites[footnote - 1]`.
    //
    // Previously `cites` was built wikilink-first then compact, while `spans`
    // were later reordered by position. When a compact marker preceded a
    // wikilink, sorting flipped the emission order and footnote `[n]` pointed
    // at the wrong `cites[n-1]` entry (see #3002).
    let mut entries: Vec<(usize, usize, RawCitation)> = Vec::new(); // (start, end, raw)

    // wikilinks
    {
        let mut j = 0;
        while let Some(start) = text[j..].find("[[") {
            let abs_start = j + start;
            let rest = &text[abs_start + 2..];
            if let Some(end_rel) = rest.find("]]") {
                let abs_end = abs_start + 2 + end_rel;
                let inner = text[abs_start + 2..abs_end].trim();
                if !inner.is_empty() {
                    entries.push((
                        abs_start,
                        abs_end + 2,
                        RawCitation {
                            label: inner.to_string(),
                            path: None,
                            offset: None,
                        },
                    ));
                }
                j = abs_end + 2;
            } else {
                break;
            }
        }
    }
    // compact
    {
        let mut j = 0;
        while let Some(start) = text[j..].find("[#cite:") {
            let abs_start = j + start;
            let rest = &text[abs_start + 7..];
            if let Some(end_rel) = rest.find(']') {
                let abs_end = abs_start + 7 + end_rel;
                let inner = &text[abs_start + 7..abs_end];
                // inner := "path:offset" or "path" or "path:" or ":offset"
                let (path, offset) = match inner.split_once(':') {
                    None => (Some(inner.to_string()), None),
                    Some((p, o)) => {
                        let path = if p.is_empty() {
                            None
                        } else {
                            Some(p.to_string())
                        };
                        let offset = if o.is_empty() {
                            None
                        } else {
                            o.parse::<usize>().ok()
                        };
                        (path, offset)
                    }
                };
                if path.is_some() || offset.is_some() {
                    entries.push((
                        abs_start,
                        abs_end + 1,
                        RawCitation {
                            label: path.clone().unwrap_or_else(|| "source".to_string()),
                            path,
                            offset,
                        },
                    ));
                }
                j = abs_end + 1;
            } else {
                break;
            }
        }
    }

    if entries.is_empty() {
        return (text.to_string(), Vec::new());
    }

    // Sort by text position so footnote numbers follow reading order and
    // `cites[footnote - 1]` always aligns with the emitted `[footnote]`.
    entries.sort_by_key(|e| e.0);

    // Build a single-pass rewrite. We walk the original text and, whenever we
    // see a marker we recognized, emit the `[n]` footnote instead.
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    let mut cites = Vec::with_capacity(entries.len());
    for (start, end, raw) in &entries {
        if *start < i {
            // Overlapping/already-emitted span — skip defensively.
            continue;
        }
        out.push_str(&text[i..*start]);
        out.push_str(&format!("[{}]", cites.len() + 1));
        cites.push(raw.clone());
        i = *end;
    }
    out.push_str(&text[i..]);
    (out, cites)
}

/// Resolve a parsed raw citation into a structured [`AnswerCitation`].
///
/// `title_resolver` maps a note title (from a wikilink label or a compact
/// path) to its canonical `(note_id, title, path)`. When resolution fails we
/// still emit a citation carrying the raw label so the UI can surface it.
fn resolve_citation(raw: &RawCitation, title_resolver: &TitleResolver) -> AnswerCitation {
    // For wikilink style the label is "Title" or "Title#Section".
    let (title_part, section) = match raw.label.split_once('#') {
        Some((t, s)) => (t.trim(), Some(s.trim().to_string())),
        None => (raw.label.trim(), None),
    };

    let (note_id, title, path) = if let Some((id, t, p)) = title_resolver(title_part) {
        (id, t, p)
    } else if let Some(p) = &raw.path {
        // Compact style with a path but no title match: best-effort.
        (p.clone(), title_part.to_string(), p.clone())
    } else {
        (
            title_part.to_string(),
            title_part.to_string(),
            String::new(),
        )
    };

    AnswerCitation {
        note_id,
        title: if title.is_empty() {
            title_part.to_string()
        } else {
            title
        },
        path,
        snippet: section.unwrap_or_default(),
        score: None,
    }
}

/// Parse citation markers from an AI `answer` and return the rewritten answer
/// (markers replaced by `[n]` footnotes) plus resolved [`AnswerCitation`]s.
///
/// `title_resolver` lets callers enrich citations with real note ids/paths
/// from the vault (using the same RAG index that produced the context). When
/// `None` is supplied, citations keep the raw label as both title and id, which
/// is sufficient for UI display and for later re-resolution.
pub fn extract_citations(
    answer: &str,
    title_resolver: &TitleResolver,
) -> (String, Vec<AnswerCitation>) {
    let (rewritten, raw) = rewrite_with_footnotes(answer);
    let citations = raw
        .iter()
        .map(|r| resolve_citation(r, title_resolver))
        .collect();
    (rewritten, citations)
}

/// Convenience wrapper that performs citation extraction without vault
/// resolution (raw labels only). Useful when only rendering a preview.
pub fn extract_citations_unresolved(answer: &str) -> (String, Vec<AnswerCitation>) {
    extract_citations(answer, &|_| None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_markers_returns_text_unchanged() {
        let (out, cites) = extract_citations_unresolved("The sky is blue.");
        assert_eq!(out, "The sky is blue.");
        assert!(cites.is_empty());
    }

    #[test]
    fn wikilink_single_citation() {
        let (out, cites) = extract_citations_unresolved(
            "Per [[Rust Book#Ownership]] the borrow checker prevents bugs.",
        );
        assert_eq!(out, "Per [1] the borrow checker prevents bugs.");
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].title, "Rust Book");
        assert_eq!(cites[0].snippet, "Ownership");
    }

    #[test]
    fn wikilink_title_without_section() {
        let (out, cites) = extract_citations_unresolved("See [[Meeting Notes]] for details.");
        assert_eq!(out, "See [1] for details.");
        assert_eq!(cites[0].title, "Meeting Notes");
        assert_eq!(cites[0].snippet, "");
    }

    #[test]
    fn multiple_markers_get_sequential_footnotes() {
        let text = "A [[Alpha]] and B [[Beta#Intro]] and C [[Gamma]].";
        let (out, cites) = extract_citations_unresolved(text);
        assert_eq!(out, "A [1] and B [2] and C [3].");
        assert_eq!(cites.len(), 3);
        assert_eq!(cites[0].title, "Alpha");
        assert_eq!(cites[1].title, "Beta");
        assert_eq!(cites[1].snippet, "Intro");
        assert_eq!(cites[2].title, "Gamma");
    }

    #[test]
    fn compact_citation_with_path_and_offset() {
        let (out, cites) = extract_citations_unresolved("Fact [#cite:notes/foo.md:120] here.");
        assert_eq!(out, "Fact [1] here.");
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].path, "notes/foo.md");
    }

    #[test]
    fn compact_citation_path_only() {
        let (out, cites) = extract_citations_unresolved("See [#cite:notes/bar.md].");
        assert_eq!(out, "See [1].");
        assert_eq!(cites[0].path, "notes/bar.md");
    }

    #[test]
    fn empty_wikilink_is_ignored() {
        let (out, cites) = extract_citations_unresolved("Blank [[]] should stay.");
        assert_eq!(out, "Blank [[]] should stay.");
        assert!(cites.is_empty());
    }

    #[test]
    fn mixed_wikilink_and_compact_keep_order() {
        let text = "First [[Note A]] then [#cite:notes/b.md:0] end.";
        let (out, cites) = extract_citations_unresolved(text);
        assert_eq!(out, "First [1] then [2] end.");
        assert_eq!(cites.len(), 2);
        assert_eq!(cites[0].title, "Note A");
        assert_eq!(cites[1].path, "notes/b.md");
    }

    #[test]
    fn compact_before_wikilink_footnote_aligns_with_citation() {
        // Regression test for #3002: when a compact marker precedes a wikilink,
        // footnote numbers must still point at the correct citation.
        let text = "first [#cite:notes/b.md:0] then [[Note A]] end.";
        let (out, cites) = extract_citations_unresolved(text);
        assert_eq!(out, "first [1] then [2] end.");
        assert_eq!(cites.len(), 2);
        // Footnote [1] sits at the compact position -> must be b.md.
        assert_eq!(cites[0].path, "notes/b.md");
        // Footnote [2] sits at the wikilink position -> must be Note A.
        assert_eq!(cites[1].title, "Note A");
    }

    #[test]
    fn resolver_enriches_with_note_id_and_path() {
        let resolver = |title: &str| -> Option<(String, String, String)> {
            if title == "Rust Book" {
                Some((
                    "note_rust_123".to_string(),
                    "Rust Book".to_string(),
                    "books/rust.md".to_string(),
                ))
            } else {
                None
            }
        };
        let (out, cites) = extract_citations("Ref [[Rust Book#Ch1]] done.", &resolver);
        assert_eq!(out, "Ref [1] done.");
        assert_eq!(cites[0].note_id, "note_rust_123");
        assert_eq!(cites[0].path, "books/rust.md");
        assert_eq!(cites[0].snippet, "Ch1");
    }

    #[test]
    fn unicode_labels_preserved() {
        let (out, cites) = extract_citations_unresolved("笔记 [[会议纪要#结论]] 已记录。");
        assert_eq!(out, "笔记 [1] 已记录。");
        assert_eq!(cites[0].title, "会议纪要");
        assert_eq!(cites[0].snippet, "结论");
    }

    #[test]
    fn no_double_counting_on_adjacent_markers() {
        let text = "[[A]][[B]]";
        let (out, cites) = extract_citations_unresolved(text);
        assert_eq!(out, "[1][2]");
        assert_eq!(cites.len(), 2);
    }
}
