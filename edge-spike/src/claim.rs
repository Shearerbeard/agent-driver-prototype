//! Deterministic claim-extraction spec.
//!
//! Named classes: quoted strings, values with units, ids, timestamps,
//! hostnames/paths. Each extracted claim is string-matched against the
//! full on-disk tool capture and the task's `context_provided` frame.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::state::FoundIn;

/// Claim class produced by the deterministic extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ClaimClass {
    QuotedString,
    ValueWithUnit,
    Id,
    Timestamp,
    HostnameOrPath,
}

/// One extracted claim with its class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedClaim {
    pub text: String,
    pub class: ClaimClass,
}

/// Result of matching extracted claims against corpora.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroundingStats {
    pub extracted_by_class: Vec<(ClaimClass, usize)>,
    pub found_in_tool_capture: usize,
    pub found_in_context: usize,
    pub not_found: usize,
    pub per_class_found: Vec<(ClaimClass, usize)>,
}

/// Extract claims from a worker result.
#[must_use]
pub fn extract_claims(text: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for claim in extract_quoted_strings(text) {
        if seen.insert(claim.text.clone()) {
            out.push(claim);
        }
    }
    for claim in extract_values_with_units(text) {
        if seen.insert(claim.text.clone()) {
            out.push(claim);
        }
    }
    for claim in extract_ids(text) {
        if seen.insert(claim.text.clone()) {
            out.push(claim);
        }
    }
    for claim in extract_timestamps(text) {
        if seen.insert(claim.text.clone()) {
            out.push(claim);
        }
    }
    for claim in extract_hostnames_or_paths(text) {
        if seen.insert(claim.text.clone()) {
            out.push(claim);
        }
    }

    out
}

fn insert_unique(
    claims: &mut Vec<ExtractedClaim>,
    text: String,
    class: ClaimClass,
    seen: &mut HashSet<String>,
) {
    if seen.insert(text.clone()) {
        claims.push(ExtractedClaim { text, class });
    }
}

#[allow(
    clippy::string_slice,
    reason = "quote boundaries are ASCII bytes, so slicing on found indices is safe"
)]
fn extract_quoted_strings(text: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (open, close) in [('"', '"'), ('\'', '\'')] {
        let mut rest = text;
        while let Some(start) = rest.find(open) {
            let after = &rest[start + 1..];
            if let Some(end) = after.find(close) {
                let inner = &after[..end];
                if !inner.is_empty() && inner.len() > 2 {
                    insert_unique(
                        &mut out,
                        inner.to_owned(),
                        ClaimClass::QuotedString,
                        &mut seen,
                    );
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    out
}

fn extract_values_with_units(text: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let units = [
        "ms", "s", "sec", "secs", "seconds", "minutes", "hours", "days", "%", "MB", "GB", "KB",
        "TB", "bytes", "bpm", "rpm", "req/s",
    ];
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| {
            c == ','
                || c == '.'
                || c == ';'
                || c == ':'
                || c == '('
                || c == ')'
                || c == '['
                || c == ']'
        });
        for unit in units {
            if let Some(num_part) = trimmed.strip_suffix(unit) {
                if num_part.parse::<f64>().is_ok() || num_part.parse::<u64>().is_ok() {
                    insert_unique(
                        &mut out,
                        trimmed.to_owned(),
                        ClaimClass::ValueWithUnit,
                        &mut seen,
                    );
                }
                break;
            }
        }
    }
    out
}

fn extract_ids(text: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        if trimmed.len() >= 6
            && (trimmed.starts_with("task_")
                || trimmed.starts_with("call_")
                || trimmed.starts_with("exc-")
                || trimmed.starts_with("cs_")
                || trimmed.starts_with("app_")
                || trimmed.starts_with("pipeline_")
                || (trimmed.contains('-') && trimmed.chars().filter(|c| c == &'-').count() >= 1))
            && trimmed
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            insert_unique(&mut out, trimmed.to_owned(), ClaimClass::Id, &mut seen);
        }
    }
    out
}

fn extract_timestamps(text: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c == '.');
        if looks_like_timestamp(trimmed) {
            insert_unique(
                &mut out,
                trimmed.to_owned(),
                ClaimClass::Timestamp,
                &mut seen,
            );
        }
    }
    out
}

fn looks_like_timestamp(s: &str) -> bool {
    if s.len() < 10 {
        return false;
    }
    let mut digits = 0;
    let mut separators = 0;
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if c == '-' || c == ':' || c == 'T' || c == 'Z' || c == '.' || c == '+' {
            separators += 1;
        } else {
            return false;
        }
    }
    digits >= 8 && separators >= 2
}

fn extract_hostnames_or_paths(text: &str) -> Vec<ExtractedClaim> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for word in text.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| {
            c == '"'
                || c == '\''
                || c == ','
                || c == '.'
                || c == '('
                || c == ')'
                || c == '<'
                || c == '>'
        });
        if (trimmed.contains('/')
            || trimmed.contains(".com")
            || trimmed.contains(".org")
            || trimmed.contains(".io")
            || trimmed.contains(".ai"))
            && trimmed.len() >= 4
            && !trimmed.contains(' ')
        {
            insert_unique(
                &mut out,
                trimmed.to_owned(),
                ClaimClass::HostnameOrPath,
                &mut seen,
            );
        }
    }
    out
}

/// String-match each extracted claim against the full capture and the
/// context-provided frame. Returns the grounding stats.
#[must_use]
pub fn resolve_claims(
    claims: &[ExtractedClaim],
    full_capture: &str,
    context_provided: &str,
) -> (Vec<(ExtractedClaim, FoundIn)>, GroundingStats) {
    let mut resolved = Vec::new();
    let mut stats = GroundingStats::default();
    let mut counts = std::collections::BTreeMap::<ClaimClass, usize>::new();
    let mut found_counts = std::collections::BTreeMap::<ClaimClass, usize>::new();

    for class in [
        ClaimClass::QuotedString,
        ClaimClass::ValueWithUnit,
        ClaimClass::Id,
        ClaimClass::Timestamp,
        ClaimClass::HostnameOrPath,
    ] {
        counts.insert(class, 0);
        found_counts.insert(class, 0);
    }

    for claim in claims {
        *counts.entry(claim.class).or_default() += 1;
        let found_in = if full_capture.contains(&claim.text) {
            stats.found_in_tool_capture += 1;
            *found_counts.entry(claim.class).or_default() += 1;
            FoundIn::ToolCapture
        } else if context_provided.contains(&claim.text) {
            stats.found_in_context += 1;
            *found_counts.entry(claim.class).or_default() += 1;
            FoundIn::ContextProvided
        } else {
            stats.not_found += 1;
            FoundIn::NotFound
        };
        resolved.push((claim.clone(), found_in));
    }

    stats.extracted_by_class = counts.into_iter().collect();
    stats.per_class_found = found_counts.into_iter().collect();
    (resolved, stats)
}

/// Agreement reporting hook: returns the fraction of extracted claims that
/// the model's grounding dimension agrees were found.
#[must_use]
pub fn agreement_fraction(stats: &GroundingStats) -> f64 {
    let total = stats.found_in_tool_capture + stats.found_in_context + stats.not_found;
    if total == 0 {
        return 1.0;
    }
    let found = stats.found_in_tool_capture + stats.found_in_context;
    f64::from(found as u32) / f64::from(total as u32)
}
