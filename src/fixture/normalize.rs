//! Test-side normalization of the two nondeterminism sources in the
//! envelope, and the snapshot assertion entry point.
//!
//! Ported from `crates/aura/src/orchestration/context_fixture/normalize.rs`.
//! The only adaptation: `message_role_and_text` uses `crate::message::Message`
//! (`User(String)`/`Assistant(String)`) instead of rig's content-parts model;
//! the single-text-part panic discipline is preserved.

use super::envelope::RequestEnvelope;

const TIMESTAMP_LABEL: &str = "Current time: ";
const TIMESTAMP_STAND_IN: &str = "Current time: <TIMESTAMP>";
const RFC3339_LEN: usize = 20;
const ROSTER_MARKER: &str = "AVAILABLE WORKERS:";
const VALID_NAMES_MARKER: &str = "Valid worker names: ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedSnapshot(String);

impl NormalizedSnapshot {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

enum TimestampScrub {
    Scrubbed(String),
    Absent,
}

/// The role label and text body of one envelope message. The conversation
/// at `9df96382` holds only single-part text turns; anything else is
/// builder drift and panics.
fn message_role_and_text(message: &crate::message::Message) -> (&'static str, String) {
    match message {
        crate::message::Message::User(text) => ("user", text.clone()),
        crate::message::Message::Assistant(text) => ("assistant", text.clone()),
    }
}

pub(crate) fn normalize(envelope: &RequestEnvelope) -> NormalizedSnapshot {
    audit_normalization_markers(envelope);

    let mut document = String::new();
    document.push_str("================ SYSTEM ================\n");
    document.push_str(&envelope.system);
    document.push_str("\n\n================ MESSAGES ================\n");

    let mut first_user_seen = false;
    for (index, message) in envelope.messages.iter().enumerate() {
        let (role, text) = message_role_and_text(message);
        let mut body = text;
        if role == "user" {
            if let TimestampScrub::Scrubbed(scrubbed) = scrub_wrapper_timestamp(&body) {
                body = scrubbed;
            }
            if !first_user_seen {
                first_user_seen = true;
                if body.contains(ROSTER_MARKER) || body.contains(VALID_NAMES_MARKER) {
                    body = canonicalize_worker_order(&body);
                }
            }
        }
        document.push_str(&format!("---- [{index}] {role} ----\n{body}\n\n"));
    }

    document.push_str("================ TOOLS ================\n");
    document.push_str(
        &serde_json::to_string_pretty(&envelope.tools_json()).expect("tools JSON renders"),
    );
    document.push('\n');

    NormalizedSnapshot(document)
}

fn has_full_timestamp_prefix(body: &str) -> bool {
    let Some(rest) = body.strip_prefix(TIMESTAMP_LABEL) else {
        return false;
    };
    let Some(stamp) = rest.get(..RFC3339_LEN) else {
        return false;
    };
    let bytes = stamp.as_bytes();
    bytes.iter().enumerate().all(|(i, b)| match i {
        4 | 7 => *b == b'-',
        10 => *b == b'T',
        13 | 16 => *b == b':',
        19 => *b == b'Z',
        _ => b.is_ascii_digit(),
    })
}

fn audit_normalization_markers(envelope: &RequestEnvelope) {
    let mut user_bodies = Vec::new();
    let mut assistant_bodies = Vec::new();
    for message in &envelope.messages {
        let (role, text) = message_role_and_text(message);
        if role == "user" {
            user_bodies.push(text);
        } else {
            assistant_bodies.push(text);
        }
    }

    let with_prefix = user_bodies
        .iter()
        .filter(|body| has_full_timestamp_prefix(body))
        .count();
    for body in &user_bodies {
        assert!(
            !body.starts_with(TIMESTAMP_LABEL) || has_full_timestamp_prefix(body),
            "normalization defect: user message starts with a malformed \
             'Current time: ' prefix: {:?}",
            &body[..body.len().min(60)]
        );
    }
    assert!(
        with_prefix == 0 || with_prefix == user_bodies.len(),
        "normalization defect: {} of {} user messages carry the timestamp prefix \
         (all-or-none expected; mixed prefixes mean builder drift)",
        with_prefix,
        user_bodies.len()
    );

    let count = |haystack: &str, needle: &str| haystack.matches(needle).count();
    let tools_json = serde_json::to_string(&envelope.tools_json()).expect("tools JSON renders");
    for marker in [ROSTER_MARKER, VALID_NAMES_MARKER] {
        assert_eq!(
            count(&envelope.system, marker),
            0,
            "normalization defect: {marker:?} found in the system preamble (payload collision)"
        );
        assert_eq!(
            count(&tools_json, marker),
            0,
            "normalization defect: {marker:?} found in the tools JSON (payload collision)"
        );
        for body in &assistant_bodies {
            assert_eq!(
                count(body, marker),
                0,
                "normalization defect: {marker:?} found in an assistant turn (payload collision)"
            );
        }
        for (index, body) in user_bodies.iter().enumerate() {
            let occurrences = count(body, marker);
            if index == 0 {
                assert!(
                    occurrences <= 1,
                    "normalization defect: {marker:?} appears {occurrences} times in the \
                     initial planning wrapper (at most once expected)"
                );
            } else {
                assert_eq!(
                    occurrences, 0,
                    "normalization defect: {marker:?} found in user message {index} \
                     (rosters render only in the initial planning wrapper)"
                );
            }
        }
    }
}

fn scrub_wrapper_timestamp(user_message: &str) -> TimestampScrub {
    if !has_full_timestamp_prefix(user_message) {
        return TimestampScrub::Absent;
    }
    let rest = &user_message[TIMESTAMP_LABEL.len() + RFC3339_LEN..];
    TimestampScrub::Scrubbed(format!("{TIMESTAMP_STAND_IN}{rest}"))
}

fn sort_span(message: &str, start: usize, end: usize, separator: &str) -> String {
    let mut entries: Vec<&str> = message[start..end].split(separator).collect();
    entries.sort_unstable();
    format!(
        "{}{}{}",
        &message[..start],
        entries.join(separator),
        &message[end..]
    )
}

fn canonicalize_worker_order(planning_wrapper: &str) -> String {
    let mut message = planning_wrapper.to_owned();

    if let Some(heading) = message.find(ROSTER_MARKER) {
        let after_heading = heading + ROSTER_MARKER.len() + 1;
        const NOTE_PREFIX: &str = "NOTE: ";
        const INLINE_TAIL: &str =
            "\n\nAssign tasks to the worker whose tools best match the required operations.";
        const NO_TOOLS_TAIL: &str = "\n\nEach worker has specialized capabilities. Assign tasks to the most appropriate worker.";
        if message[after_heading..].starts_with(NOTE_PREFIX) {
            let note_end = message[after_heading..]
                .find("\n\n")
                .map(|i| after_heading + i + 2)
                .expect("normalization defect: Summary/Full roster NOTE line has no terminator");
            let span_end = message[note_end..]
                .find(INLINE_TAIL)
                .map(|i| note_end + i)
                .expect("normalization defect: Summary/Full roster has no closing sentence");
            for block in message[note_end..span_end].split("\n\n") {
                assert!(
                    block.starts_with("## "),
                    "normalization defect: roster block does not start with '## ': {block:?}"
                );
            }
            message = sort_span(&message, note_end, span_end, "\n\n");
        } else {
            let span_end = message[after_heading..]
                .find(NO_TOOLS_TAIL)
                .map(|i| after_heading + i)
                .expect("normalization defect: None-visibility roster has no closing sentence");
            message = sort_span(&message, after_heading, span_end, "\n");
        }
    }

    if let Some(label) = message.find(VALID_NAMES_MARKER) {
        let names_start = label + VALID_NAMES_MARKER.len();
        let names_end = message[names_start..]
            .find('\n')
            .map(|i| names_start + i)
            .expect("normalization defect: valid-names line has no terminator");
        message = sort_span(&message, names_start, names_end, ", ");
    }

    message
}

/// Assert the envelope's normalized snapshot against the committed
/// snapshot named `name` (insta, strict).
#[cfg(test)]
pub(crate) fn assert_envelope_snapshot(name: &str, envelope: &RequestEnvelope) {
    let snapshot = normalize(envelope);
    insta::assert_snapshot!(name, snapshot.as_str());
}
