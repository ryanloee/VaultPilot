#![allow(dead_code)] // Storage layer; CLI/agent wiring + UI land in follow-ups.

//! PDF annotations — highlights, bookmarks, and margin notes (#3157).
//!
//! VaultPilot already supports PDF preview and search (#1767), but engineers
//! reading datasheets, reference manuals, and papers need to annotate them.
//! This module provides the **storage layer** for PDF annotations using a
//! version-control-friendly Markdown sidecar format.
//!
//! Each PDF in the vault (e.g. `papers/alloca.pdf`) gets a sibling
//! `.annotations.md` file (e.g. `papers/.alloca.annotations.md`) that stores
//! highlights, bookmarks, and margin notes as a YAML frontmatter + Markdown
//! body. The sidecar stays plain-text so it diffs cleanly under git / Syncthing
//! / Obsidian Sync, mirroring how flashcards and bases configs are stored.
//!
//! ## Sidecar format
//!
//! ```markdown
//! ---
//! pdf: papers/alloca.pdf
//! page_count: 12
//! updated_at: "2026-08-02T10:00:00+00:00"
//! annotations:
//!   - id: 7f3a...
//!     type: highlight
//!     page: 3
//!     color: yellow
//!     text: "stack pointer must be..."
//!     note: ""
//!     created_at: "2026-08-02T09:00:00+00:00"
//!   - id: 9b1c...
//!     type: bookmark
//!     page: 5
//!     color: ""
//!     text: ""
//!     note: "key table"
//!     created_at: "2026-08-02T09:30:00+00:00"
//! ---
//!
//! # Annotations for papers/alloca.pdf
//!
//! Annotations are stored in the YAML frontmatter above; this body is a
//! human-readable summary auto-generated from the data.
//! ```
//!
//! The frontmatter is the source of truth — the body is regenerated on save so
//! the file remains readable in any Markdown viewer while staying diffable.
//!
//! UI layers (WinUI / Mobile) overlay this data on the PDF reader; this module
//! only handles parsing, serialization, CRUD, and query — it is UI-agnostic.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Annotation types ─────────────────────────────────────────────────

/// The kind of annotation a user placed on a PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AnnotationKind {
    /// A highlighted text range on a page.
    #[default]
    Highlight,
    /// A page-level bookmark (no text range).
    Bookmark,
    /// A free-text margin note anchored to a page.
    MarginNote,
}

impl AnnotationKind {
    /// Lowercase tag used in serialized form.
    pub fn as_str(&self) -> &'static str {
        match self {
            AnnotationKind::Highlight => "highlight",
            AnnotationKind::Bookmark => "bookmark",
            AnnotationKind::MarginNote => "margin_note",
        }
    }

    fn parse(raw: &str) -> AnnotationKind {
        match raw.trim().to_ascii_lowercase().as_str() {
            "bookmark" => AnnotationKind::Bookmark,
            "margin_note" | "marginnote" | "margin-note" => AnnotationKind::MarginNote,
            _ => AnnotationKind::Highlight,
        }
    }
}

/// Highlight colors supported by the annotation layer.
///
/// These are the canonical Obsidian/PDF.js color names so the UI can map them
/// to concrete RGB values without a lookup table here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HighlightColor {
    #[default]
    Yellow,
    Green,
    Blue,
    Pink,
    Purple,
}

impl HighlightColor {
    /// Parse a color name, falling back to the default (Yellow) on unknown.
    pub fn parse(raw: &str) -> HighlightColor {
        match raw.trim().to_ascii_lowercase().as_str() {
            "green" => HighlightColor::Green,
            "blue" => HighlightColor::Blue,
            "pink" => HighlightColor::Pink,
            "purple" => HighlightColor::Purple,
            _ => HighlightColor::Yellow,
        }
    }
}

/// A single annotation placed on a PDF.
///
/// All fields are stored as plain types so the struct serializes cleanly to
/// YAML frontmatter and survives round-trips without data loss.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// Unique identifier (UUID, generated client-side).
    pub id: String,
    /// The kind of annotation.
    #[serde(default)]
    pub kind: AnnotationKind,
    /// 1-based page number the annotation lives on.
    #[serde(default)]
    pub page: u32,
    /// Highlight color (ignored for bookmarks with no fill).
    #[serde(default)]
    pub color: HighlightColor,
    /// The highlighted text (empty for bookmarks / pure margin notes).
    #[serde(default)]
    pub text: String,
    /// A free-text note the user attached to the annotation.
    #[serde(default)]
    pub note: String,
    /// Creation timestamp (ISO-8601).
    pub created_at: String,
}

impl Annotation {
    /// Create a new annotation with a fresh id and timestamp.
    pub fn new(kind: AnnotationKind, page: u32) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            page,
            color: HighlightColor::Yellow,
            text: String::new(),
            note: String::new(),
            created_at: Utc::now().to_rfc3339(),
        }
    }
}

// ─── Sidecar file model ───────────────────────────────────────────────

/// The frontmatter model for a `.annotations.md` sidecar file.
///
/// Only the fields we control are strongly typed; the body is regenerated from
/// `annotations` on save so it never drifts from the data.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationFile {
    /// Vault-relative path to the PDF this file annotates.
    #[serde(default)]
    pub pdf: String,
    /// Total page count of the PDF (optional, informational).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    /// Last write timestamp (ISO-8601).
    #[serde(default)]
    pub updated_at: String,
    /// All annotations for this PDF.
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

impl AnnotationFile {
    /// Create an empty file model for a given PDF path.
    pub fn for_pdf(pdf: impl Into<String>) -> Self {
        Self {
            pdf: pdf.into(),
            page_count: None,
            updated_at: Utc::now().to_rfc3339(),
            annotations: Vec::new(),
        }
    }

    /// Add an annotation, returning its id.
    pub fn add(&mut self, mut ann: Annotation) -> &str {
        if ann.id.is_empty() {
            ann.id = Uuid::new_v4().to_string();
        }
        if ann.created_at.is_empty() {
            ann.created_at = Utc::now().to_rfc3339();
        }
        self.annotations.push(ann);
        self.updated_at = Utc::now().to_rfc3339();
        self.annotations.last().unwrap().id.as_str()
    }

    /// Remove an annotation by id. Returns `true` if it existed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.annotations.len();
        self.annotations.retain(|a| a.id != id);
        let removed = self.annotations.len() < before;
        if removed {
            self.updated_at = Utc::now().to_rfc3339();
        }
        removed
    }

    /// Update the note text on an existing annotation. Returns `true` if found.
    pub fn set_note(&mut self, id: &str, note: &str) -> bool {
        if let Some(ann) = self.annotations.iter_mut().find(|a| a.id == id) {
            ann.note = note.to_string();
            self.updated_at = Utc::now().to_rfc3339();
            true
        } else {
            false
        }
    }

    /// All annotations on a given page, in creation order.
    pub fn for_page(&self, page: u32) -> Vec<&Annotation> {
        self.annotations.iter().filter(|a| a.page == page).collect()
    }

    /// Group annotations by page number, ascending.
    pub fn by_page(&self) -> Vec<(u32, Vec<&Annotation>)> {
        let mut groups: HashMap<u32, Vec<&Annotation>> = HashMap::new();
        for ann in &self.annotations {
            groups.entry(ann.page).or_default().push(ann);
        }
        let mut ordered: Vec<(u32, Vec<&Annotation>)> = groups.into_iter().collect();
        ordered.sort_by_key(|(page, _)| *page);
        ordered
    }

    /// All bookmarks, in page order.
    pub fn bookmarks(&self) -> Vec<&Annotation> {
        self.annotations
            .iter()
            .filter(|a| a.kind == AnnotationKind::Bookmark)
            .collect()
    }
}

// ─── Serialization (frontmatter + body) ───────────────────────────────

/// YAML frontmatter delimiter.
const FRONTMATTER_DELIM: &str = "---";

impl AnnotationFile {
    /// Serialize to the `.annotations.md` sidecar format (frontmatter + body).
    pub fn to_markdown(&self) -> Result<String, serde_yaml_ng::Error> {
        let frontmatter = serde_yaml_ng::to_string(self)?;
        let body = self.render_body();
        Ok(format!(
            "{delim}\n{fm}{delim}\n\n{body}",
            delim = FRONTMATTER_DELIM,
            fm = frontmatter,
            body = body
        ))
    }

    /// Parse a `.annotations.md` sidecar from its full text.
    ///
    /// Returns an empty model (with `pdf` blank) if the text has no
    /// frontmatter, so callers can detect a malformed file by checking
    /// [`AnnotationFile::pdf`].
    pub fn from_markdown(text: &str) -> Self {
        let trimmed = text.trim_start();
        if !trimmed.starts_with(FRONTMATTER_DELIM) {
            return AnnotationFile::default();
        }
        // Find the closing delimiter on its own line. The YAML block sits
        // between the opening `---` (already consumed) and the next `\n---`.
        let after_open = &trimmed[FRONTMATTER_DELIM.len()..];
        let close = match after_open.find("\n---") {
            Some(offset) => offset,
            None => return AnnotationFile::default(),
        };
        let yaml_block = &after_open[..close];
        match serde_yaml_ng::from_str::<AnnotationFile>(yaml_block) {
            Ok(mut file) => {
                // Normalize: drop any annotation without an id (legacy files).
                file.annotations.retain(|a| !a.id.is_empty());
                file
            }
            Err(_) => AnnotationFile::default(),
        }
    }

    /// Render the human-readable Markdown body from the annotation data.
    ///
    /// The body is a convenience for reading the file in any Markdown viewer;
    /// the frontmatter is the source of truth and the body is regenerated on
    /// every save so the two never drift.
    fn render_body(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# Annotations for {}\n\n", self.pdf));
        if self.annotations.is_empty() {
            out.push_str("_No annotations yet._\n");
            return out;
        }
        for (page, anns) in self.by_page() {
            out.push_str(&format!("## Page {page}\n\n"));
            for ann in anns {
                let kind = ann.kind.as_str();
                let color = match ann.kind {
                    AnnotationKind::Highlight => format!(" ({:?})", ann.color).to_lowercase(),
                    _ => String::new(),
                };
                out.push_str(&format!("- **{}{color}** `#{}` — ", kind, ann.id));
                if !ann.text.is_empty() {
                    out.push_str(&format!("\"{}\"", ann.text.replace('\n', " ")));
                }
                if !ann.note.is_empty() {
                    out.push_str(&format!(" → {}", ann.note));
                }
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }
}

// ─── Filesystem helpers ───────────────────────────────────────────────

/// Build the sidecar path for a given PDF.
///
/// `papers/alloca.pdf` → `papers/.alloca.annotations.md`
/// The dotfile naming keeps sidecars out of most note listings while staying
/// alongside the PDF for easy syncing.
pub fn sidecar_path<P: AsRef<Path>>(pdf: P) -> PathBuf {
    let p = pdf.as_ref();
    let dir = p.parent().unwrap_or_else(|| Path::new(""));
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string());
    dir.join(format!(".{stem}.annotations.md"))
}

/// Load annotations for a PDF from its sidecar.
///
/// Returns an empty model if the sidecar does not exist yet (new PDF). If the
/// sidecar exists but is malformed, the malformed content is ignored and an
/// empty model is returned so the UI can recover rather than crash.
pub fn load<P: AsRef<Path>>(pdf: P) -> AnnotationFile {
    let path = sidecar_path(&pdf);
    let pdf_str = pdf.as_ref().to_string_lossy().into_owned();
    match fs::read_to_string(&path) {
        Ok(text) => {
            let mut file = AnnotationFile::from_markdown(&text);
            // `from_markdown` returns `AnnotationFile::default()` (with an
            // empty `pdf` field) when the sidecar YAML is malformed.  Restore
            // the correct PDF path so a subsequent `save()` doesn't write
            // `pdf: ""` and permanently corrupt the sidecar (#3774).
            if file.pdf.is_empty() {
                file.pdf = pdf_str;
            }
            file
        }
        Err(_) => AnnotationFile::for_pdf(pdf_str),
    }
}

/// Save annotations for a PDF to its sidecar, creating the file if needed.
pub fn save<P: AsRef<Path>>(pdf: P, file: &AnnotationFile) -> Result<(), std::io::Error> {
    let path = sidecar_path(&pdf);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let markdown = file.to_markdown().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("yaml error: {e}"))
    })?;
    fs::write(&path, markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yellow_highlight(page: u32, text: &str) -> Annotation {
        Annotation {
            id: format!("id-{page}-{text}"),
            kind: AnnotationKind::Highlight,
            page,
            color: HighlightColor::Yellow,
            text: text.to_string(),
            note: String::new(),
            created_at: "2026-08-02T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn sidecar_path_naming() {
        let p = sidecar_path("papers/alloca.pdf");
        assert_eq!(p, Path::new("papers/.alloca.annotations.md"));
    }

    #[test]
    fn sidecar_path_no_directory() {
        let p = sidecar_path("doc.pdf");
        assert_eq!(p, Path::new(".doc.annotations.md"));
    }

    #[test]
    fn sidecar_path_preserves_subdirectory() {
        let p = sidecar_path("a/b/c/manual.pdf");
        assert_eq!(p, Path::new("a/b/c/.manual.annotations.md"));
    }

    #[test]
    fn roundtrip_empty_file() {
        let f = AnnotationFile::for_pdf("papers/x.pdf");
        let md = f.to_markdown().unwrap();
        let parsed = AnnotationFile::from_markdown(&md);
        assert_eq!(parsed.pdf, "papers/x.pdf");
        assert!(parsed.annotations.is_empty());
    }

    #[test]
    fn roundtrip_with_annotations() {
        let mut f = AnnotationFile::for_pdf("papers/alloca.pdf");
        f.page_count = Some(12);
        f.add(yellow_highlight(3, "stack pointer"));
        f.add(Annotation {
            id: "bm-5".to_string(),
            kind: AnnotationKind::Bookmark,
            page: 5,
            color: HighlightColor::Yellow,
            text: String::new(),
            note: "key table".to_string(),
            created_at: "2026-08-02T00:00:00+00:00".to_string(),
        });
        let md = f.to_markdown().unwrap();
        let parsed = AnnotationFile::from_markdown(&md);
        assert_eq!(parsed.pdf, "papers/alloca.pdf");
        assert_eq!(parsed.page_count, Some(12));
        assert_eq!(parsed.annotations.len(), 2);
        assert_eq!(parsed.annotations[0].page, 3);
        assert_eq!(parsed.annotations[0].text, "stack pointer");
        assert_eq!(parsed.annotations[1].kind, AnnotationKind::Bookmark);
        assert_eq!(parsed.annotations[1].note, "key table");
    }

    #[test]
    fn parse_tolerates_missing_frontmatter() {
        let parsed = AnnotationFile::from_markdown("just some prose, no yaml");
        assert!(parsed.pdf.is_empty());
        assert!(parsed.annotations.is_empty());
    }

    #[test]
    fn parse_tolerates_malformed_yaml() {
        let bad = "---\npdf: [unclosed\nannotations: !!bad\n---\nbody";
        let parsed = AnnotationFile::from_markdown(bad);
        // Malformed YAML → empty model (graceful degradation).
        assert!(parsed.annotations.is_empty());
    }

    #[test]
    fn add_assigns_id_and_timestamp() {
        let mut f = AnnotationFile::for_pdf("d.pdf");
        let mut ann = Annotation::new(AnnotationKind::Highlight, 1);
        ann.id.clear();
        ann.created_at.clear();
        f.add(ann);
        let stored = &f.annotations[0];
        assert!(!stored.id.is_empty());
        assert!(!stored.created_at.is_empty());
    }

    #[test]
    fn remove_by_id() {
        let mut f = AnnotationFile::for_pdf("d.pdf");
        f.add(yellow_highlight(1, "a"));
        f.add(yellow_highlight(2, "b"));
        let id0 = f.annotations[0].id.clone();
        assert!(f.remove(&id0));
        assert_eq!(f.annotations.len(), 1);
        assert!(!f.remove(&id0)); // already gone
    }

    #[test]
    fn set_note_updates_existing() {
        let mut f = AnnotationFile::for_pdf("d.pdf");
        f.add(yellow_highlight(1, "a"));
        let id = f.annotations[0].id.clone();
        assert!(f.set_note(&id, "important"));
        assert_eq!(f.annotations[0].note, "important");
        assert!(!f.set_note("nope", "x"));
    }

    #[test]
    fn for_page_filters_correctly() {
        let mut f = AnnotationFile::for_pdf("d.pdf");
        f.add(yellow_highlight(1, "a"));
        f.add(yellow_highlight(2, "b"));
        f.add(yellow_highlight(2, "c"));
        assert_eq!(f.for_page(1).len(), 1);
        assert_eq!(f.for_page(2).len(), 2);
        assert!(f.for_page(99).is_empty());
    }

    #[test]
    fn by_page_groups_ascending() {
        let mut f = AnnotationFile::for_pdf("d.pdf");
        f.add(yellow_highlight(3, "c"));
        f.add(yellow_highlight(1, "a"));
        f.add(yellow_highlight(3, "c2"));
        let grouped = f.by_page();
        let pages: Vec<u32> = grouped.iter().map(|(p, _)| *p).collect();
        assert_eq!(pages, vec![1, 3]);
        assert_eq!(grouped[0].1.len(), 1);
        assert_eq!(grouped[1].1.len(), 2);
    }

    #[test]
    fn bookmarks_filter() {
        let mut f = AnnotationFile::for_pdf("d.pdf");
        f.add(yellow_highlight(1, "a"));
        f.add(Annotation {
            id: "bm".to_string(),
            kind: AnnotationKind::Bookmark,
            page: 2,
            color: HighlightColor::Yellow,
            text: String::new(),
            note: String::new(),
            created_at: "t".to_string(),
        });
        assert_eq!(f.bookmarks().len(), 1);
        assert_eq!(f.bookmarks()[0].page, 2);
    }

    #[test]
    fn annotation_kind_roundtrip() {
        for kind in [
            AnnotationKind::Highlight,
            AnnotationKind::Bookmark,
            AnnotationKind::MarginNote,
        ] {
            let s = kind.as_str();
            assert_eq!(AnnotationKind::parse(s), kind);
        }
    }

    #[test]
    fn annotation_kind_parse_unknown_defaults_highlight() {
        assert_eq!(AnnotationKind::parse("???"), AnnotationKind::Highlight);
    }

    #[test]
    fn highlight_color_parse_known() {
        assert_eq!(HighlightColor::parse("green"), HighlightColor::Green);
        assert_eq!(HighlightColor::parse("BLUE"), HighlightColor::Blue);
        assert_eq!(HighlightColor::parse("purple"), HighlightColor::Purple);
    }

    #[test]
    fn highlight_color_parse_unknown_defaults_yellow() {
        assert_eq!(HighlightColor::parse("orange"), HighlightColor::Yellow);
        assert_eq!(HighlightColor::parse(""), HighlightColor::Yellow);
    }

    #[test]
    fn render_body_shows_page_and_text() {
        let mut f = AnnotationFile::for_pdf("papers/x.pdf");
        f.add(yellow_highlight(3, "key insight"));
        let body = f.render_body();
        assert!(body.contains("Page 3"));
        assert!(body.contains("key insight"));
        assert!(body.contains("highlight"));
    }

    #[test]
    fn render_body_empty_file_has_placeholder() {
        let f = AnnotationFile::for_pdf("papers/x.pdf");
        let body = f.render_body();
        assert!(body.contains("No annotations"));
    }

    #[test]
    fn save_and_load_roundtrip_on_disk() {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-pdf-ann-test-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("doc.pdf");

        let mut file = AnnotationFile::for_pdf(pdf.to_string_lossy());
        file.page_count = Some(5);
        file.add(yellow_highlight(2, "hello"));
        save(&pdf, &file).unwrap();

        let loaded = load(&pdf);
        assert_eq!(loaded.pdf, pdf.to_string_lossy());
        assert_eq!(loaded.page_count, Some(5));
        assert_eq!(loaded.annotations.len(), 1);
        assert_eq!(loaded.annotations[0].text, "hello");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_sidecar_returns_empty_model() {
        let pdf =
            std::env::temp_dir().join(format!("vaultpilot-pdf-ann-missing-{}.pdf", Uuid::new_v4()));
        let loaded = load(&pdf);
        assert_eq!(loaded.pdf, pdf.to_string_lossy());
        assert!(loaded.annotations.is_empty());
    }

    #[test]
    fn test_regression_3774_load_restores_pdf_path_on_malformed_sidecar() {
        // Before #3774: when the sidecar file existed but contained malformed
        // YAML, `from_markdown` returned `AnnotationFile::default()` with an
        // empty `pdf` field. `load` passed it through without restoring the
        // path, so a subsequent `save()` would write `pdf: ""` into the
        // sidecar and permanently corrupt it.
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-pdf-ann-malformed-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("corrupt.pdf");
        let sidecar = sidecar_path(&pdf);
        // Write a malformed sidecar that `from_markdown` can't parse.
        fs::write(
            &sidecar,
            "---\npdf: [unclosed\nannotations: !!bad\n---\nbody",
        )
        .unwrap();

        let loaded = load(&pdf);
        // The pdf path must be restored to the argument, not empty.
        assert_eq!(loaded.pdf, pdf.to_string_lossy());
        assert!(loaded.annotations.is_empty());

        // A save after load should NOT corrupt the sidecar with empty pdf.
        save(&pdf, &loaded).unwrap();
        let rewritten = fs::read_to_string(&sidecar).unwrap();
        assert!(
            !rewritten.contains("pdf: \"\""),
            "sidecar should not contain empty pdf after load+save"
        );
        assert!(
            rewritten.contains("corrupt.pdf"),
            "sidecar should contain the restored pdf path"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_creates_parent_directory() {
        let dir = std::env::temp_dir().join(format!(
            "vaultpilot-pdf-ann-nested-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        // parent dir does not exist yet
        let pdf = dir.join("sub").join("deep").join("doc.pdf");
        let file = AnnotationFile::for_pdf(pdf.to_string_lossy());
        save(&pdf, &file).unwrap();
        assert!(sidecar_path(&pdf).exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn margin_note_kind_serializes_lowercase() {
        let ann = Annotation::new(AnnotationKind::MarginNote, 7);
        assert_eq!(ann.kind.as_str(), "margin_note");
        // round-trips through the kind parse
        assert_eq!(
            AnnotationKind::parse(ann.kind.as_str()),
            AnnotationKind::MarginNote
        );
    }
}
