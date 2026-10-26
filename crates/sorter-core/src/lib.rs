//! Pure domain model and deterministic matching engine.

use globset::{GlobBuilder, GlobMatcher};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("rule id must not be empty")]
    EmptyRuleId,
    #[error("duplicate rule id: {0}")]
    DuplicateRuleId(String),
    #[error("rule {rule_id} has no matching condition")]
    EmptyMatch { rule_id: String },
    #[error("rule {rule_id} has an invalid glob {pattern:?}: {source}")]
    InvalidGlob {
        rule_id: String,
        pattern: String,
        source: globset::Error,
    },
    #[error("rule {rule_id} has an invalid regular expression {pattern:?}: {source}")]
    InvalidRegex {
        rule_id: String,
        pattern: String,
        source: regex::Error,
    },
    #[error("rule {rule_id} has an empty destination")]
    EmptyDestination { rule_id: String },
    #[error("source path has no filename: {0}")]
    MissingFilename(PathBuf),
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchOperator {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MatchSpec {
    pub operator: MatchOperator,
    #[serde(
        default,
        alias = "exact_filename",
        deserialize_with = "deserialize_one_or_many"
    )]
    pub exact_filenames: Vec<String>,
    #[serde(
        default,
        alias = "filename_glob",
        deserialize_with = "deserialize_one_or_many"
    )]
    pub filename_globs: Vec<String>,
    #[serde(
        default,
        alias = "filename_regex",
        deserialize_with = "deserialize_one_or_many"
    )]
    pub filename_regexes: Vec<String>,
    #[serde(
        default,
        alias = "extension",
        deserialize_with = "deserialize_one_or_many"
    )]
    pub extensions: Vec<String>,
    pub case_sensitive: bool,
}

fn deserialize_one_or_many<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(value) => vec![value],
        OneOrMany::Many(values) => values,
    })
}

impl MatchSpec {
    fn group_count(&self) -> usize {
        [
            self.exact_filenames.is_empty(),
            self.filename_globs.is_empty(),
            self.filename_regexes.is_empty(),
            self.extensions.is_empty(),
        ]
        .into_iter()
        .filter(|empty| !empty)
        .count()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    pub destination: PathBuf,
    #[serde(rename = "match")]
    pub match_spec: MatchSpec,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug)]
struct CompiledRule {
    rule: Rule,
    declaration_index: usize,
    globs: Vec<GlobMatcher>,
    regexes: Vec<Regex>,
}

#[derive(Debug)]
pub struct MatchEngine {
    rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuleMatch {
    pub rule_id: String,
    pub destination: PathBuf,
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlannedMove {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub rule_id: String,
    pub priority: i32,
}

impl MatchEngine {
    /// Compiles patterns once and sorts rules by descending priority, then by
    /// declaration order. First match wins.
    pub fn compile(rules: &[Rule]) -> Result<Self, CoreError> {
        let mut seen = HashSet::new();
        let mut compiled = Vec::with_capacity(rules.len());

        for (declaration_index, rule) in rules.iter().enumerate() {
            validate_rule(rule, &mut seen)?;
            if !rule.enabled {
                continue;
            }

            let globs = rule
                .match_spec
                .filename_globs
                .iter()
                .map(|pattern| {
                    GlobBuilder::new(pattern)
                        .case_insensitive(!rule.match_spec.case_sensitive)
                        .literal_separator(true)
                        .build()
                        .map(|glob| glob.compile_matcher())
                        .map_err(|source| CoreError::InvalidGlob {
                            rule_id: rule.id.clone(),
                            pattern: pattern.clone(),
                            source,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;

            let regexes = rule
                .match_spec
                .filename_regexes
                .iter()
                .map(|pattern| {
                    RegexBuilder::new(pattern)
                        .case_insensitive(!rule.match_spec.case_sensitive)
                        .build()
                        .map_err(|source| CoreError::InvalidRegex {
                            rule_id: rule.id.clone(),
                            pattern: pattern.clone(),
                            source,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;

            compiled.push(CompiledRule {
                rule: rule.clone(),
                declaration_index,
                globs,
                regexes,
            });
        }

        compiled.sort_by(|left, right| {
            right
                .rule
                .priority
                .cmp(&left.rule.priority)
                .then_with(|| left.declaration_index.cmp(&right.declaration_index))
        });

        Ok(Self { rules: compiled })
    }

    pub fn find_match(&self, path: &Path) -> Result<Option<RuleMatch>, CoreError> {
        let filename = path
            .file_name()
            .ok_or_else(|| CoreError::MissingFilename(path.to_path_buf()))?;
        let filename_lossy = filename.to_string_lossy();

        for compiled in &self.rules {
            if compiled.matches(&filename_lossy, path.extension()) {
                return Ok(Some(RuleMatch {
                    rule_id: compiled.rule.id.clone(),
                    destination: compiled.rule.destination.clone(),
                    priority: compiled.rule.priority,
                }));
            }
        }
        Ok(None)
    }

    pub fn plan(&self, source: &Path) -> Result<Option<PlannedMove>, CoreError> {
        let Some(matched) = self.find_match(source)? else {
            return Ok(None);
        };
        let filename = source
            .file_name()
            .ok_or_else(|| CoreError::MissingFilename(source.to_path_buf()))?;
        Ok(Some(PlannedMove {
            source: source.to_path_buf(),
            destination: matched.destination.join(filename),
            rule_id: matched.rule_id,
            priority: matched.priority,
        }))
    }
}

impl CompiledRule {
    fn matches(&self, filename: &str, extension: Option<&std::ffi::OsStr>) -> bool {
        let spec = &self.rule.match_spec;
        let comparable_name = if spec.case_sensitive {
            filename.to_owned()
        } else {
            filename.to_lowercase()
        };

        let mut groups = Vec::with_capacity(spec.group_count());
        if !spec.exact_filenames.is_empty() {
            groups.push(spec.exact_filenames.iter().any(|candidate| {
                if spec.case_sensitive {
                    candidate == filename
                } else {
                    candidate.to_lowercase() == comparable_name
                }
            }));
        }
        if !self.globs.is_empty() {
            groups.push(self.globs.iter().any(|glob| glob.is_match(filename)));
        }
        if !self.regexes.is_empty() {
            groups.push(self.regexes.iter().any(|regex| regex.is_match(filename)));
        }
        if !spec.extensions.is_empty() {
            let actual = extension.map(|value| value.to_string_lossy());
            groups.push(actual.is_some_and(|actual| {
                spec.extensions.iter().any(|candidate| {
                    let candidate = candidate.trim_start_matches('.');
                    if spec.case_sensitive {
                        candidate == actual
                    } else {
                        candidate.eq_ignore_ascii_case(&actual)
                    }
                })
            }));
        }

        match spec.operator {
            MatchOperator::All => groups.iter().all(|matched| *matched),
            MatchOperator::Any => groups.iter().any(|matched| *matched),
        }
    }
}

fn validate_rule(rule: &Rule, seen: &mut HashSet<String>) -> Result<(), CoreError> {
    if rule.id.trim().is_empty() {
        return Err(CoreError::EmptyRuleId);
    }
    if !seen.insert(rule.id.clone()) {
        return Err(CoreError::DuplicateRuleId(rule.id.clone()));
    }
    if rule.destination.as_os_str().is_empty() {
        return Err(CoreError::EmptyDestination {
            rule_id: rule.id.clone(),
        });
    }
    if rule.match_spec.group_count() == 0 {
        return Err(CoreError::EmptyMatch {
            rule_id: rule.id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rule(id: &str, priority: i32, match_spec: MatchSpec) -> Rule {
        Rule {
            id: id.into(),
            enabled: true,
            priority,
            destination: PathBuf::from(format!("/sorted/{id}")),
            match_spec,
        }
    }

    #[test]
    fn higher_priority_wins() {
        let broad = rule(
            "documents",
            10,
            MatchSpec {
                extensions: vec!["pdf".into()],
                ..MatchSpec::default()
            },
        );
        let invoice = rule(
            "invoices",
            100,
            MatchSpec {
                filename_regexes: vec![r"invoice.*\.pdf$".into()],
                ..MatchSpec::default()
            },
        );
        let engine = MatchEngine::compile(&[broad, invoice]).unwrap();
        assert_eq!(
            engine
                .find_match(Path::new("Invoice-2026.PDF"))
                .unwrap()
                .unwrap()
                .rule_id,
            "invoices"
        );
    }

    #[test]
    fn declaration_order_breaks_priority_ties() {
        let rules = [
            rule(
                "first",
                10,
                MatchSpec {
                    filename_globs: vec!["*.txt".into()],
                    ..MatchSpec::default()
                },
            ),
            rule(
                "second",
                10,
                MatchSpec {
                    extensions: vec!["txt".into()],
                    ..MatchSpec::default()
                },
            ),
        ];
        let engine = MatchEngine::compile(&rules).unwrap();
        assert_eq!(
            engine
                .find_match(Path::new("notes.txt"))
                .unwrap()
                .unwrap()
                .rule_id,
            "first"
        );
    }

    #[test]
    fn all_and_any_have_distinct_semantics() {
        let mut spec = MatchSpec {
            filename_globs: vec!["invoice*".into()],
            extensions: vec!["pdf".into()],
            ..MatchSpec::default()
        };
        let all = MatchEngine::compile(&[rule("all", 1, spec.clone())]).unwrap();
        assert!(all.find_match(Path::new("invoice.txt")).unwrap().is_none());

        spec.operator = MatchOperator::Any;
        let any = MatchEngine::compile(&[rule("any", 1, spec)]).unwrap();
        assert!(any.find_match(Path::new("invoice.txt")).unwrap().is_some());
    }

    proptest! {
        #[test]
        fn matching_is_deterministic(name in "[a-zA-Z0-9_-]{1,40}\\.(pdf|txt|jpg)") {
            let engine = MatchEngine::compile(&[
                rule("docs", 10, MatchSpec { extensions: vec!["pdf".into(), "txt".into()], ..MatchSpec::default() }),
                rule("images", 10, MatchSpec { extensions: vec!["jpg".into()], ..MatchSpec::default() }),
            ]).unwrap();
            let first = engine.find_match(Path::new(&name)).unwrap();
            let second = engine.find_match(Path::new(&name)).unwrap();
            prop_assert_eq!(first, second);
        }
    }
}
