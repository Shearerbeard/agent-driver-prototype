//! A deliberately minimal JSON-Schema validator: the `inputSchema`
//! subset the rig's discovered tools actually declare, and nothing
//! more.
//!
//! W1 narrowing, recorded in `DESIGN.md`: the card's scope names
//! `src/workflow/` only, so adding a schema crate to `Cargo.toml` is a
//! stop-and-report rather than a quiet new dependency. The subset below
//! covers the validation-relevant keywords the sidecar tools use —
//! `type` (with `string`, `number`, `integer`, `boolean`, `object`,
//! `array`, `null`), `properties`, `required`, `items`, and `enum`.
//! Metadata keywords (`title`, `description`, `$schema`, `$id`,
//! `default`, `examples`) are ignored. Any other validation-relevant
//! keyword fails loud as [`SchemaCheck::Unsupported`] rather than
//! passing silently: the approver-authorization rule (K3 finding 2)
//! makes unearned validation confidence the worst outcome of the three.
//!
//! Reference nodes are structural here: a `{"$from": ...}` object
//! occupies the position its property names, and its bounds (when
//! present) must be numbers, but its *type* is checked at resolve time
//! by the W3 executor against the value the referenced export actually
//! produced. Propose time checks what is knowable at propose time.

use serde_json::Value;

/// Why an inputSchema check refused.
///
/// The two cases map to different [`WorkflowError`](super::WorkflowError)
/// variants at the caller: an unsupported keyword indicts the *schema*
/// (the tool's declaration exceeds what this validator can honestly
/// check), while a mismatch indicts the *instance* (the model's
/// arguments). Conflating them would let a tool-limitation read as a
/// model mistake.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "W1 Layer 1: used when validate_step_args is filled"
)]
pub(super) enum SchemaCheck {
    /// The schema leans on a validation-relevant keyword outside the
    /// subset; `keyword` names it.
    Unsupported { keyword: String },
    /// The instance broke the rule `message` describes.
    Mismatch { message: String },
}

/// Validate `instance` against `schema`, both as raw JSON.
///
/// # Errors
///
/// Returns the first refusal per [`SchemaCheck`]: the caller (the spec
/// validator) owns the step and tool names needed to phrase either case
/// as a `WorkflowError`.
#[expect(
    dead_code,
    reason = "W1 Layer 1: called when validate_step_args is filled"
)]
pub(super) fn validate_instance(_schema: &Value, _instance: &Value) -> Result<(), SchemaCheck> {
    todo!()
}

/// Whether an argument node is a reference (`{"$from": ...}`), which
/// occupies a structural position in the instance tree without being
/// type-checked at propose time.
#[expect(
    dead_code,
    reason = "W1 Layer 1: called when the reference walk is filled"
)]
pub(super) fn is_reference_node(_node: &Value) -> bool {
    todo!()
}
