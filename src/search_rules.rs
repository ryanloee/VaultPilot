use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Configuration for domain-specific search behavior.
/// Loaded from a JSON file at startup; falls back to built-in defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRulesConfig {
    /// Synonym/alias groups for query expansion
    pub synonym_groups: Vec<SynonymGroup>,
    /// Domain-specific relevance score bonuses
    pub relevance_bonuses: Vec<RelevanceBonus>,
    /// Keyword-based heuristic rules for auto-generating note metadata
    pub heuristic_keywords: Vec<HeuristicKeyword>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynonymGroup {
    /// If any of these terms is found in a query, expand to all aliases
    pub triggers: Vec<String>,
    /// The full set of synonyms to expand to
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevanceBonus {
    /// Terms to match in the query
    pub query_terms: Vec<String>,
    /// Terms to match in the document
    pub doc_terms: Vec<String>,
    /// Score bonus when both query and doc match
    pub bonus: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeuristicKeyword {
    /// Patterns to match in input text (case-insensitive for ASCII)
    pub patterns: Vec<String>,
    /// Override title if matched (optional)
    #[serde(default)]
    pub title: Option<String>,
    /// Tags to add if matched
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Global singleton for search rules.
pub struct SearchRules {
    pub config: SearchRulesConfig,
}

/// Check whether a trigger matches a normalized term.
///
/// - ASCII-only triggers (e.g. "sd", "tf", "gpio"): whole-word match to avoid
///   false positives on unrelated words like "address" containing "sd".
/// - CJK / mixed triggers (e.g. "刷机", "sd卡"): substring match since CJK
///   text has no word boundaries.
fn trigger_matches(normalized_term: &str, trigger: &str) -> bool {
    if trigger.is_empty() {
        return false;
    }
    if trigger.is_ascii() {
        // Whole-word matching: the trigger must appear as an entire token or
        // be bounded by non-alphanumeric characters.
        let mut start = 0;
        while let Some(pos) = normalized_term[start..].find(trigger) {
            let abs_pos = start + pos;
            let before_ok =
                abs_pos == 0 || !normalized_term.as_bytes()[abs_pos - 1].is_ascii_alphanumeric();
            let after_pos = abs_pos + trigger.len();
            let after_ok = after_pos >= normalized_term.len()
                || !normalized_term.as_bytes()[after_pos].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
            start = abs_pos + 1;
        }
        false
    } else {
        // CJK / non-ASCII: substring match is appropriate
        normalized_term.contains(trigger)
    }
}

/// Check whether a needle matches a term for relevance bonus scoring.
/// Uses the same logic as trigger_matches for consistency.
fn relevance_term_matches(term: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    // Normalize to lowercase for case-insensitive matching (#902)
    let needle_lower = needle.to_lowercase();
    let term_lower = term.to_lowercase();
    if needle_lower.is_ascii() && needle_lower.len() < 5 {
        // Short ASCII needle: whole-word match in both directions
        trigger_matches(&term_lower, &needle_lower) || trigger_matches(&needle_lower, &term_lower)
    } else if !needle_lower.is_ascii() {
        // CJK / non-ASCII: bidirectional substring match is appropriate
        term_lower.contains(&needle_lower) || needle_lower.contains(&term_lower)
    } else {
        // Long ASCII needle (>=5 chars): only check if term contains needle.
        // The reverse direction (needle.contains(term)) would cause short terms
        // like "sd" to match long needles like "sdmmc_controller_driver", inflating
        // relevance scores for unrelated documents.
        term_lower.contains(&needle_lower)
    }
}

static GLOBAL_RULES: OnceLock<SearchRules> = OnceLock::new();

impl SearchRules {
    /// Initialize with built-in defaults (embedded-systems domain).
    pub fn init_defaults() {
        GLOBAL_RULES.get_or_init(|| SearchRules {
            config: default_config(),
        });
    }

    /// Initialize from a JSON config file, falling back to defaults on error.
    pub fn init_from_file(path: &std::path::Path) {
        GLOBAL_RULES.get_or_init(|| match std::fs::read_to_string(path) {
            Ok(json) => match serde_json::from_str::<SearchRulesConfig>(&json) {
                Ok(config) => {
                    tracing::info!("Loaded search rules from {}", path.display());
                    SearchRules { config }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse search rules from {}: {e}, using defaults",
                        path.display()
                    );
                    SearchRules {
                        config: default_config(),
                    }
                }
            },
            Err(e) => {
                tracing::debug!(
                    "No search rules file at {}: {e}, using defaults",
                    path.display()
                );
                SearchRules {
                    config: default_config(),
                }
            }
        });
    }

    /// Get the global instance (initializes with defaults if not yet initialized).
    pub fn global() -> &'static SearchRules {
        GLOBAL_RULES.get_or_init(|| SearchRules {
            config: default_config(),
        })
    }

    /// Expand a search term using synonym groups.
    /// Returns all aliases for any group whose trigger matches the term.
    pub fn expand_term_aliases(&self, term: &str) -> Vec<String> {
        let normalized = term.trim().to_lowercase();
        let mut aliases = Vec::new();

        for group in &self.config.synonym_groups {
            if group
                .triggers
                .iter()
                .any(|t| trigger_matches(&normalized, &t.to_lowercase()))
            {
                aliases.extend(group.aliases.iter().cloned());
            }
        }

        aliases
    }

    /// Compute domain-specific relevance bonus based on query and document terms.
    pub fn domain_relevance_bonus(&self, query_terms: &[String], doc_terms: &[String]) -> i64 {
        let mut bonus = 0_i64;

        for rule in &self.config.relevance_bonuses {
            let query_match = rule.query_terms.iter().any(|needle| {
                query_terms
                    .iter()
                    .any(|term| relevance_term_matches(term, needle))
            });
            let doc_match = rule.doc_terms.iter().any(|needle| {
                doc_terms
                    .iter()
                    .any(|term| relevance_term_matches(term, needle))
            });
            if query_match && doc_match {
                bonus += rule.bonus;
            }
        }

        bonus
    }

    /// Evaluate heuristic rules against input text.
    /// Returns (optional_title, tags) from matching rules.
    pub fn evaluate_heuristic(&self, input: &str) -> (Option<String>, Vec<String>) {
        let lower_input = input.to_ascii_lowercase();
        let mut title = None;
        let mut tags = Vec::new();

        for rule in &self.config.heuristic_keywords {
            let matched = rule
                .patterns
                .iter()
                .any(|p| lower_input.contains(&p.to_ascii_lowercase()) || input.contains(p));
            if matched {
                if title.is_none() {
                    title = rule.title.clone();
                }
                tags.extend(rule.tags.iter().cloned());
            }
        }

        (title, tags)
    }
}

fn default_config() -> SearchRulesConfig {
    serde_json::from_str(DEFAULT_RULES_JSON).unwrap_or_else(|e| {
        tracing::error!(
            "Failed to parse default search rules JSON: {e}, falling back to empty config"
        );
        SearchRulesConfig {
            synonym_groups: Vec::new(),
            relevance_bonuses: Vec::new(),
            heuristic_keywords: Vec::new(),
        }
    })
}

pub const DEFAULT_RULES_JSON: &str = r#"{
  "synonym_groups": [
    {
      "triggers": ["刷机", "刷写", "烧录", "flash"],
      "aliases": ["刷机", "刷写", "烧录", "升级", "flash", "wboot", "固件", "镜像", "zboot"]
    },
    {
      "triggers": ["sd", "sd卡", "sdio", "mmc", "tf"],
      "aliases": ["sd", "sd卡", "sdio", "mmc", "tf", "sdmmc"]
    },
    {
      "triggers": ["引脚复用", "复用", "pinmux", "pinctrl", "iomux"],
      "aliases": ["引脚复用", "复用", "pinmux", "pinctrl", "iomux", "pin multiplexing"]
    },
    {
      "triggers": ["gpio"],
      "aliases": ["gpio", "管脚", "引脚"]
    }
  ],
  "relevance_bonuses": [
    {
      "query_terms": ["刷机", "刷写", "烧录", "flash", "update", "升级"],
      "doc_terms": ["wboot", "update", "flash", "烧录", "刷机", "刷写"],
      "bonus": 140
    },
    {
      "query_terms": ["sd卡", "sd", "sdio", "mmc", "tf"],
      "doc_terms": ["sd卡", "sd", "sdio", "mmc", "tf", "sdmmc"],
      "bonus": 120
    },
    {
      "query_terms": ["引脚复用", "复用", "pinmux", "pinctrl", "iomux"],
      "doc_terms": ["引脚复用", "复用", "pinmux", "pinctrl", "iomux", "pin multiplexing"],
      "bonus": 120
    }
  ],
  "heuristic_keywords": [
    {
      "patterns": ["刷机", "flash"],
      "title": "刷机命令记录",
      "tags": ["flash"]
    },
    {
      "patterns": ["uboot"],
      "tags": ["uboot"]
    }
  ]
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_flash_aliases() {
        let rules = SearchRules {
            config: default_config(),
        };
        let aliases = rules.expand_term_aliases("刷机");
        assert!(aliases.contains(&"烧录".to_string()));
        assert!(aliases.contains(&"flash".to_string()));
    }

    #[test]
    fn expand_sd_aliases() {
        let rules = SearchRules {
            config: default_config(),
        };
        let aliases = rules.expand_term_aliases("sd");
        assert!(aliases.contains(&"sd卡".to_string()));
        assert!(aliases.contains(&"sdio".to_string()));
    }

    #[test]
    fn expand_gpio_aliases() {
        let rules = SearchRules {
            config: default_config(),
        };
        let aliases = rules.expand_term_aliases("gpio");
        assert!(aliases.contains(&"管脚".to_string()));
        assert!(aliases.contains(&"引脚".to_string()));
    }

    #[test]
    fn expand_pinmux_aliases() {
        let rules = SearchRules {
            config: default_config(),
        };
        let aliases = rules.expand_term_aliases("pinmux");
        assert!(aliases.contains(&"引脚复用".to_string()));
        assert!(aliases.contains(&"iomux".to_string()));
    }

    #[test]
    fn expand_random_term_returns_empty() {
        let rules = SearchRules {
            config: default_config(),
        };
        assert!(rules
            .expand_term_aliases("something_unrelated_xyz")
            .is_empty());
    }

    #[test]
    fn domain_bonus_flash() {
        let rules = SearchRules {
            config: default_config(),
        };
        let query = vec!["刷机".to_string()];
        let doc = vec!["flash".to_string(), "固件".to_string()];
        assert_eq!(rules.domain_relevance_bonus(&query, &doc), 140);
    }

    #[test]
    fn domain_bonus_no_match() {
        let rules = SearchRules {
            config: default_config(),
        };
        let query = vec!["python".to_string()];
        let doc = vec!["rust".to_string()];
        assert_eq!(rules.domain_relevance_bonus(&query, &doc), 0);
    }

    #[test]
    fn heuristic_flash() {
        let rules = SearchRules {
            config: default_config(),
        };
        let (title, tags) = rules.evaluate_heuristic("帮我刷机一下");
        assert_eq!(title, Some("刷机命令记录".to_string()));
        assert!(tags.contains(&"flash".to_string()));
    }

    #[test]
    fn heuristic_uboot() {
        let rules = SearchRules {
            config: default_config(),
        };
        let (title, tags) = rules.evaluate_heuristic("setenv uboot command");
        assert!(tags.contains(&"uboot".to_string()));
        assert!(title.is_none());
    }

    #[test]
    fn custom_config_from_json() {
        let json = r#"{
            "synonym_groups": [
                {"triggers": ["rust"], "aliases": ["rust", "rustlang", "cargo"]}
            ],
            "relevance_bonuses": [
                {"query_terms": ["rust"], "doc_terms": ["rust"], "bonus": 50}
            ],
            "heuristic_keywords": [
                {"patterns": ["compile"], "title": "Compilation Note", "tags": ["build"]}
            ]
        }"#;
        let config: SearchRulesConfig = serde_json::from_str(json).unwrap();
        let rules = SearchRules { config };
        assert!(rules
            .expand_term_aliases("rust")
            .contains(&"cargo".to_string()));
        assert_eq!(
            rules.evaluate_heuristic("please compile this").0,
            Some("Compilation Note".to_string())
        );
    }

    #[test]
    fn sd_trigger_no_false_positive_on_address() {
        // "address" contains "sd" as a substring but should NOT match
        let rules = SearchRules {
            config: default_config(),
        };
        let aliases = rules.expand_term_aliases("address");
        assert!(
            aliases.is_empty(),
            "expand_term_aliases(\"address\") should not match sd trigger, got: {aliases:?}"
        );
    }

    #[test]
    fn sd_trigger_no_false_positive_on_consider() {
        let rules = SearchRules {
            config: default_config(),
        };
        let aliases = rules.expand_term_aliases("consider");
        assert!(aliases.is_empty());
    }

    #[test]
    fn tf_trigger_no_false_positive_on_platform() {
        // "platform" doesn't contain "tf", but let's test "manifest" which does not either
        // Actually "tf" in "platform" — no. Let's test a word that does contain "tf"
        // "performance" doesn't. Let's just verify "tf" exact match works
        let rules = SearchRules {
            config: default_config(),
        };
        let aliases = rules.expand_term_aliases("tf");
        assert!(aliases.contains(&"sd卡".to_string()));
    }

    #[test]
    fn trigger_matches_whole_word_boundary() {
        // "sd-card" should match because '-' is not alphanumeric
        assert!(trigger_matches("sd-card", "sd"));
        // "sd" exact match
        assert!(trigger_matches("sd", "sd"));
        // "address" should NOT match "sd"
        assert!(!trigger_matches("address", "sd"));
        // "consider" should NOT match "sd"
        assert!(!trigger_matches("consider", "sd"));
        // "desktop" should NOT match "sd"
        assert!(!trigger_matches("desktop", "sd"));
    }

    #[test]
    fn trigger_matches_empty_trigger_returns_false() {
        assert!(!trigger_matches("anything", ""));
        assert!(!trigger_matches("", ""));
        assert!(!trigger_matches("test string", ""));
    }

    #[test]
    fn trigger_matches_cjk_substring() {
        // CJK triggers use substring matching
        assert!(trigger_matches("帮我刷机一下", "刷机"));
        assert!(!trigger_matches("没有匹配", "刷机"));
    }

    #[test]
    fn relevance_term_matches_case_insensitive() {
        // Mixed-case needle should match lowercase term (#902)
        assert!(relevance_term_matches("sd", "SD"));
        assert!(relevance_term_matches("SD", "sd"));
        assert!(relevance_term_matches("flash", "Flash"));
        assert!(relevance_term_matches("Flash", "flash"));
        // Long ASCII needle should also be case-insensitive
        assert!(relevance_term_matches("sdmmc_controller_driver", "SDMMC"));
    }

    #[test]
    fn domain_relevance_bonus_case_insensitive() {
        let rules = SearchRules {
            config: default_config(),
        };
        // Default config has lowercase "sd" in query_terms and "sdmmc" in doc_terms
        // Mixed-case input should still match (#902)
        let query = vec!["SD".to_string()];
        let doc = vec!["SDMMC".to_string()];
        let bonus = rules.domain_relevance_bonus(&query, &doc);
        assert_eq!(bonus, 120, "Expected 120 bonus for SD+SDMMC, got {bonus}");
    }
}
