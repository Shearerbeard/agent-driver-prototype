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
    /// Claims grounded in the tool capture only via value-equivalence (a bare
    /// number or decimal-equivalent form), not an exact text match.
    #[serde(default)]
    pub normalized_matches: usize,
    pub regions_total: usize,
    pub regions_degraded: usize,
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

/// A claim resolved against the capture, carrying all hit positions.
#[derive(Debug, Clone)]
pub struct ClaimResolution {
    pub claim: ExtractedClaim,
    pub found_in: FoundIn,
    /// Character start positions of each occurrence in `full_capture`.
    pub positions: Vec<usize>,
}

fn find_all_positions(haystack: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let hay_chars: Vec<char> = haystack.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();
    let mut positions = Vec::new();
    for (i, window) in hay_chars.windows(needle_chars.len()).enumerate() {
        if window == needle_chars.as_slice() {
            positions.push(i);
        }
    }
    positions
}

/// The bare numeric value of a `<number><unit>` claim, with the unit
/// stripped (`"51.1%"` -> `51.1`). Returns `None` when the leading token is
/// not a number.
fn bare_value(text: &str) -> Option<f64> {
    let number_end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '+' || c == '-'))
        .unwrap_or(text.len());
    text[..number_end].parse::<f64>().ok()
}

/// True when the character bordering a numeric token is neither alphanumeric
/// nor a decimal point, so `51.1` does not match inside `151.1` or `51.12`.
fn numeric_boundary(c: Option<&char>) -> bool {
    c.is_none_or(|c| !(c.is_alphanumeric() || *c == '.'))
}

/// Positions of numeric tokens in `haystack` whose value equals `value`.
///
/// A token is a maximal decimal literal (`[+-]?digits(.digits)?`), so
/// `: 51.1`, `:51.1`, and `51.10` all match `51.1` while `51.12` and `151.1`
/// do not. Comparison is exact decimal equivalence, not a rounding tolerance.
fn find_value_positions(haystack: &str, value: f64) -> Vec<usize> {
    let chars: Vec<char> = haystack.chars().collect();
    let mut positions = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        if (chars[index] == '-' || chars[index] == '+')
            && chars.get(index + 1).is_some_and(char::is_ascii_digit)
        {
            index += 1;
        }
        let digits_start = index;
        while chars.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        if index > digits_start
            && chars.get(index) == Some(&'.')
            && chars.get(index + 1).is_some_and(char::is_ascii_digit)
        {
            index += 1;
            while chars.get(index).is_some_and(char::is_ascii_digit) {
                index += 1;
            }
        }
        if index == start {
            index += 1;
            continue;
        }
        let before = start.checked_sub(1).map(|i| &chars[i]);
        if index > digits_start && numeric_boundary(before) && numeric_boundary(chars.get(index)) {
            let token: String = chars[start..index].iter().collect();
            if let Ok(parsed) = token.parse::<f64>()
                && parsed == value
            {
                positions.push(start);
            }
        }
    }
    positions
}

/// Resolve each claim against the full capture and context-provided frame,
/// returning every occurrence position in the capture.
#[must_use]
pub fn resolve_claim_positions(
    claims: &[ExtractedClaim],
    full_capture: &str,
    context_provided: &str,
) -> (Vec<ClaimResolution>, GroundingStats) {
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
        let exact_positions = find_all_positions(full_capture, &claim.text);
        let (positions, normalized) = if !exact_positions.is_empty() {
            (exact_positions, false)
        } else if claim.class == ClaimClass::ValueWithUnit {
            let normalized_positions = bare_value(&claim.text)
                .map(|value| find_value_positions(full_capture, value))
                .unwrap_or_default();
            let matched = !normalized_positions.is_empty();
            (normalized_positions, matched)
        } else {
            (Vec::new(), false)
        };
        let found_in = if !positions.is_empty() {
            stats.found_in_tool_capture += 1;
            if normalized {
                stats.normalized_matches += 1;
            }
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
        resolved.push(ClaimResolution {
            claim: claim.clone(),
            found_in,
            positions,
        });
    }

    stats.extracted_by_class = counts.into_iter().collect();
    stats.per_class_found = found_counts.into_iter().collect();
    (resolved, stats)
}

/// String-match each extracted claim against the full capture and the
/// context-provided frame. Returns the grounding stats.
#[must_use]
pub fn resolve_claims(
    claims: &[ExtractedClaim],
    full_capture: &str,
    context_provided: &str,
) -> (Vec<(ExtractedClaim, FoundIn)>, GroundingStats) {
    let (resolved, stats) = resolve_claim_positions(claims, full_capture, context_provided);
    let slim = resolved
        .into_iter()
        .map(|r| (r.claim, r.found_in))
        .collect();
    (slim, stats)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_one(text: &str, class: ClaimClass, capture: &str) -> ClaimResolution {
        let claims = vec![ExtractedClaim {
            text: text.to_owned(),
            class,
        }];
        let (mut resolved, _) = resolve_claim_positions(&claims, capture, "");
        resolved.pop().expect("one resolution")
    }

    fn resolve_stats(text: &str, class: ClaimClass, capture: &str) -> GroundingStats {
        let claims = vec![ExtractedClaim {
            text: text.to_owned(),
            class,
        }];
        resolve_claim_positions(&claims, capture, "").1
    }

    #[test]
    fn value_with_unit_matches_bare_json_number() {
        let capture = "{\"pct\": 51.1}";
        let resolution = resolve_one("51.1%", ClaimClass::ValueWithUnit, capture);
        assert_eq!(resolution.found_in, FoundIn::ToolCapture);
        assert_eq!(resolution.positions, vec![8]);
        assert_eq!(
            resolve_stats("51.1%", ClaimClass::ValueWithUnit, capture).normalized_matches,
            1
        );
    }

    #[test]
    fn value_with_unit_matches_compact_json_number() {
        let resolution = resolve_one("51.1%", ClaimClass::ValueWithUnit, "{\"pct\":51.1}");
        assert_eq!(resolution.found_in, FoundIn::ToolCapture);
        assert_eq!(resolution.positions, vec![7]);
    }

    #[test]
    fn decimal_equivalent_values_match() {
        let narrower = resolve_one("51.10%", ClaimClass::ValueWithUnit, "{\"pct\": 51.1}");
        assert_eq!(narrower.found_in, FoundIn::ToolCapture);
        let wider = resolve_one("51.1%", ClaimClass::ValueWithUnit, "{\"pct\": 51.10}");
        assert_eq!(wider.found_in, FoundIn::ToolCapture);
    }

    #[test]
    fn neighboring_numbers_do_not_match() {
        let capture = "{\"pct\": 51.2}";
        let resolution = resolve_one("51.1%", ClaimClass::ValueWithUnit, capture);
        assert_eq!(resolution.found_in, FoundIn::NotFound);
        assert_eq!(
            resolve_stats("51.1%", ClaimClass::ValueWithUnit, capture).normalized_matches,
            0
        );

        // `51.1` is not a token of a longer number.
        let longer = resolve_one("51.1%", ClaimClass::ValueWithUnit, "{\"pct\": 151.1}");
        assert_eq!(longer.found_in, FoundIn::NotFound);
    }

    #[test]
    fn exact_hits_are_not_counted_as_normalized() {
        let stats = resolve_stats("150ms", ClaimClass::ValueWithUnit, "latency 150ms");
        assert_eq!(stats.found_in_tool_capture, 1);
        assert_eq!(stats.normalized_matches, 0);
    }

    #[test]
    fn non_value_classes_stay_exact() {
        let capture = "{\"pct\": 51.1}";
        for class in [
            ClaimClass::QuotedString,
            ClaimClass::Id,
            ClaimClass::Timestamp,
            ClaimClass::HostnameOrPath,
        ] {
            let stats = resolve_stats("51.10", class, capture);
            assert_eq!(stats.not_found, 1, "{class:?} must not be normalized");
            assert_eq!(stats.normalized_matches, 0, "{class:?}");
        }
    }
}
