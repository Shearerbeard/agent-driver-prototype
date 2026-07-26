//! The corpus pins one configuration: MCP-less, persistence-disabled,
//! `AURA_ESCAPE_HATCH` unset. The golden harness asserts this guardrail
//! before any snapshot runs so an env-shaped envelope can never be
//! snapshotted silently.

/// Panic when `AURA_ESCAPE_HATCH` is set: the corpus pins the default
/// preamble branch.
pub fn assert_escape_hatch_unset() {
    assert!(
        std::env::var_os("AURA_ESCAPE_HATCH").is_none(),
        "AURA_ESCAPE_HATCH is set: the corpus pins the default preamble branch; \
         unset it before running the golden-frame tests"
    );
}
