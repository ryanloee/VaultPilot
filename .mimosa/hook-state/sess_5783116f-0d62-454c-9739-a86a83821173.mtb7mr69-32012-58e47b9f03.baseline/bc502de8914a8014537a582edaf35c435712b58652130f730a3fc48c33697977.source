//! People-aware context index — Phase 1 core of #1807 (人物感知上下文面板).
//!
//! Where existing surfaces (`context_surface`, `calendar`) aggregate notes by
//! *content* or *calendar event*, this module aggregates them by *person*.
//! It extracts people from a note's frontmatter (`participants` / `attendees` /
//! `people` / `with`) and from `@mentions` in the body, resolves aliases to a
//! canonical name, and builds a reverse index `person -> [notes]` so callers
//! can answer "which notes involve this person, most recent first".
//!
//! This is the foundational index layer the issue calls out as a prerequisite
//! ("需要先有人物索引层"). CLI query commands and the WinUI/Android side panel
//! are follow-up phases that build on this API.

use std::collections::BTreeMap;

/// Frontmatter keys that hold participant/person lists, checked case-insensitively.
const PEOPLE_KEYS: &[&str] = &["participants", "attendees", "people", "with"];

/// A reference to a note that involves one or more people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRef {
    /// Stable identifier for the note (path, id, or slug — caller's choice).
    pub id: String,
    /// Optional display title.
    pub title: Option<String>,
    /// Optional sort key (e.g. RFC3339 timestamp). Larger sorts first (newest).
    pub timestamp: Option<String>,
}

impl NoteRef {
    /// Convenience constructor for a note reference with just an id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: None,
            timestamp: None,
        }
    }

    /// Builder: attach a title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Builder: attach a sort timestamp.
    pub fn with_timestamp(mut self, ts: impl Into<String>) -> Self {
        self.timestamp = Some(ts.into());
        self
    }
}

/// Maps alternate spellings/nicknames of a person to one canonical name.
///
/// e.g. `老王`, `王明`, `Wang Ming` -> `王明`. Lookups are case-insensitive and
/// whitespace-normalized so `"  alice "` and `"Alice"` resolve identically.
#[derive(Debug, Default, Clone)]
pub struct PersonAliasMap {
    /// normalized alias -> canonical display name
    aliases: BTreeMap<String, String>,
}

impl PersonAliasMap {
    /// Create an empty alias map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `alias` as another name for `canonical`.
    pub fn add_alias(&mut self, alias: impl AsRef<str>, canonical: impl Into<String>) {
        let key = normalize_key(alias.as_ref());
        if key.is_empty() {
            return;
        }
        self.aliases.insert(key, canonical.into());
    }

    /// Resolve `name` to its canonical form. When no alias is registered the
    /// trimmed input is returned unchanged (so unknown people still index).
    pub fn resolve(&self, name: &str) -> String {
        let key = normalize_key(name);
        if let Some(canonical) = self.aliases.get(&key) {
            canonical.clone()
        } else {
            name.trim().to_string()
        }
    }
}

/// Lower-cased, whitespace-collapsed key used for alias/de-dup matching.
fn normalize_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Extract people names from a note's `frontmatter` (raw YAML, no `---` fences)
/// and `body`. Returns canonical-agnostic raw names, de-duplicated in first-seen
/// order. Apply [`PersonAliasMap::resolve`] to canonicalize.
pub fn extract_people(frontmatter: Option<&str>, body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut push = |name: String, out: &mut Vec<String>| {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let key = normalize_key(&trimmed);
        if seen.insert(key) {
            out.push(trimmed);
        }
    };

    if let Some(fm) = frontmatter {
        for name in people_from_frontmatter(fm) {
            push(name, &mut out);
        }
    }
    for name in mentions_from_body(body) {
        push(name, &mut out);
    }
    out
}

/// Parse the people-bearing frontmatter keys into a flat list of names.
fn people_from_frontmatter(frontmatter: &str) -> Vec<String> {
    let mut names = Vec::new();
    let value: serde_yaml_ng::Value = match serde_yaml_ng::from_str(frontmatter) {
        Ok(v) => v,
        Err(_) => return names,
    };
    let map = match value.as_mapping() {
        Some(m) => m,
        None => return names,
    };
    for (k, v) in map {
        let key = match k.as_str() {
            Some(s) => s.to_lowercase(),
            None => continue,
        };
        if !PEOPLE_KEYS.contains(&key.as_str()) {
            continue;
        }
        collect_yaml_names(v, &mut names);
    }
    names
}

/// Flatten a YAML value (string, comma-separated string, or sequence) into names.
fn collect_yaml_names(value: &serde_yaml_ng::Value, out: &mut Vec<String>) {
    match value {
        serde_yaml_ng::Value::String(s) => {
            for part in s.split(',') {
                let t = part.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
        }
        serde_yaml_ng::Value::Sequence(seq) => {
            for item in seq {
                collect_yaml_names(item, out);
            }
        }
        _ => {}
    }
}

/// Extract `@mention` names from note body text.
///
/// A mention starts at `@` that is not preceded by a word character (so email
/// addresses like `alice@example.com` are ignored) and runs over subsequent
/// letters/digits (Unicode, incl. CJK), `_` and `-`.
fn mentions_from_body(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = body.chars().collect();
    let is_name_char = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let prev_is_word = i > 0 && is_name_char(chars[i - 1]);
            if !prev_is_word {
                let mut j = i + 1;
                let start = j;
                while j < chars.len() && is_name_char(chars[j]) {
                    j += 1;
                }
                if j > start {
                    let name: String = chars[start..j].iter().collect();
                    let name = name.trim_matches('-').to_string();
                    if !name.is_empty() {
                        out.push(name);
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Reverse index from canonical person name to the notes they appear in.
#[derive(Debug, Default)]
pub struct PeopleIndex {
    alias_map: PersonAliasMap,
    /// canonical name -> notes (kept sorted newest-first on query)
    by_person: BTreeMap<String, Vec<NoteRef>>,
}

impl PeopleIndex {
    /// Create an index with the given alias map.
    pub fn new(alias_map: PersonAliasMap) -> Self {
        Self {
            alias_map,
            by_person: BTreeMap::new(),
        }
    }

    /// Index a note: extract its people, canonicalize, and record the reference
    /// under each person. Returns the canonical people found in the note.
    pub fn add_note(
        &mut self,
        note: NoteRef,
        frontmatter: Option<&str>,
        body: &str,
    ) -> Vec<String> {
        let raw = extract_people(frontmatter, body);
        let mut canonical: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in raw {
            let c = self.alias_map.resolve(&name);
            if c.is_empty() {
                continue;
            }
            let key = normalize_key(&c);
            if !seen.insert(key) {
                continue;
            }
            canonical.push(c.clone());
            let entry = self.by_person.entry(c).or_default();
            if !entry.iter().any(|n| n.id == note.id) {
                entry.push(note.clone());
            }
        }
        canonical
    }

    /// All canonical people known to the index, alphabetically.
    pub fn people(&self) -> Vec<String> {
        self.by_person.keys().cloned().collect()
    }

    /// Notes involving `person`, newest-first (by `timestamp`, then id).
    /// The `person` argument is resolved through the alias map first.
    pub fn notes_for(&self, person: &str) -> Vec<NoteRef> {
        let canonical = self.alias_map.resolve(person);
        let mut notes = match self.by_person.get(&canonical) {
            Some(v) => v.clone(),
            None => return Vec::new(),
        };
        notes.sort_by(|a, b| {
            match (&b.timestamp, &a.timestamp) {
                (Some(bt), Some(at)) => bt.cmp(at),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| a.id.cmp(&b.id))
        });
        notes
    }

    /// Number of distinct people indexed.
    pub fn len(&self) -> usize {
        self.by_person.len()
    }

    /// Whether the index has no people.
    pub fn is_empty(&self) -> bool {
        self.by_person.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_frontmatter_list_and_string() {
        let fm = "title: 1:1\nparticipants:\n  - Alice\n  - Bob\n";
        let people = extract_people(Some(fm), "");
        assert_eq!(people, vec!["Alice".to_string(), "Bob".to_string()]);

        let fm2 = "attendees: Alice, Bob, Carol\n";
        let people2 = extract_people(Some(fm2), "");
        assert_eq!(
            people2,
            vec!["Alice".to_string(), "Bob".to_string(), "Carol".to_string()]
        );
    }

    #[test]
    fn extract_body_mentions_and_skips_emails() {
        let body = "Met with @alice and @bob_dev. Ping alice@example.com later. cc @王明";
        let people = extract_people(None, body);
        assert_eq!(
            people,
            vec![
                "alice".to_string(),
                "bob_dev".to_string(),
                "王明".to_string()
            ]
        );
        // The email local-part must not be treated as a mention.
        assert!(!people.contains(&"example".to_string()));
    }

    #[test]
    fn extract_dedups_across_sources_case_insensitive() {
        let fm = "participants: [Alice]";
        let body = "Follow up with @alice and @Alice";
        let people = extract_people(Some(fm), body);
        assert_eq!(people, vec!["Alice".to_string()]);
    }

    #[test]
    fn alias_map_resolves_to_canonical() {
        let mut aliases = PersonAliasMap::new();
        aliases.add_alias("老王", "王明");
        aliases.add_alias("Wang Ming", "王明");
        assert_eq!(aliases.resolve("老王"), "王明");
        assert_eq!(aliases.resolve("  wang ming "), "王明");
        // Unknown name passes through trimmed.
        assert_eq!(aliases.resolve("  Zoe  "), "Zoe");
    }

    #[test]
    fn index_aggregates_notes_by_person_newest_first() {
        let mut aliases = PersonAliasMap::new();
        aliases.add_alias("老王", "王明");
        let mut idx = PeopleIndex::new(aliases);

        idx.add_note(
            NoteRef::new("n1").with_timestamp("2026-07-01T09:00:00Z"),
            Some("participants: [Alice, 老王]"),
            "",
        );
        idx.add_note(
            NoteRef::new("n2").with_timestamp("2026-07-10T09:00:00Z"),
            Some("participants: [王明]"),
            "kickoff with @Alice",
        );
        idx.add_note(
            NoteRef::new("n3").with_timestamp("2026-07-05T09:00:00Z"),
            None,
            "solo note, no people",
        );

        // Alias-folded: 老王 and 王明 are the same person.
        let wang = idx.notes_for("老王");
        let ids: Vec<&str> = wang.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["n2", "n1"]); // newest first

        // Alice appears in n1 and n2 (frontmatter + mention), newest first.
        let alice = idx.notes_for("Alice");
        let alice_ids: Vec<&str> = alice.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(alice_ids, vec!["n2", "n1"]);

        assert_eq!(idx.people(), vec!["Alice".to_string(), "王明".to_string()]);
        assert_eq!(idx.len(), 2);
        assert!(!idx.is_empty());
    }

    #[test]
    fn notes_for_unknown_person_is_empty() {
        let idx = PeopleIndex::new(PersonAliasMap::new());
        assert!(idx.notes_for("Nobody").is_empty());
        assert!(idx.is_empty());
    }

    #[test]
    fn same_note_not_duplicated_when_person_appears_twice() {
        let mut idx = PeopleIndex::new(PersonAliasMap::new());
        let found = idx.add_note(
            NoteRef::new("n1"),
            Some("participants: [Alice]"),
            "reminder for @Alice and @alice",
        );
        assert_eq!(found, vec!["Alice".to_string()]);
        assert_eq!(idx.notes_for("Alice").len(), 1);
    }

    #[test]
    fn empty_and_malformed_frontmatter_is_safe() {
        assert!(extract_people(Some(""), "").is_empty());
        assert!(extract_people(Some(": : bad yaml ["), "").is_empty());
        assert!(extract_people(None, "").is_empty());
    }
}
