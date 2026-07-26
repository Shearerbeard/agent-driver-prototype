//! The corpus pins one configuration: MCP-less, persistence-disabled,
//! `AURA_ESCAPE_HATCH` unset. The golden harness asserts this guardrail
//! before any envelope is composed so an env-shaped envelope can never be
//! snapshotted silently.
//!
//! Only the escape-hatch pin is runtime-checkable; the other two are
//! structural properties of the port and hold by construction:
//!
//! - MCP-less: `producers::resolve_worker_tools` binds `all_tools` to an
//!   empty vec (the exact value aura's `get_all_tool_names` returns when
//!   `mcp_manager` is `None`), and `producers::get_all_tool_descriptions`
//!   omits the MCP manager block entirely.
//! - Persistence-disabled: `src/persistence.rs` carries data types only;
//!   no file IO exists in the crate.

/// Assert the full corpus configuration guardrail. Currently the only
/// runtime-checkable pin is the escape hatch; the MCP-less and
/// persistence-disabled pins are structural (module docs).
pub fn assert_corpus_configuration() {
    assert_escape_hatch_unset();
}

/// Panic (fail loud, never fall back) when `AURA_ESCAPE_HATCH` is set:
/// the corpus pins the default preamble branch.
pub fn assert_escape_hatch_unset() {
    assert!(
        std::env::var_os("AURA_ESCAPE_HATCH").is_none(),
        "AURA_ESCAPE_HATCH is set: the corpus pins the default preamble branch; \
         unset it before running the golden-frame tests"
    );
}
