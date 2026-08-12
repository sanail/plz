pub mod openai;
pub mod presets;

use anyhow::Result;

use crate::context::Context;
use crate::suggestion::Suggestion;

/// A source of command suggestions.
///
/// The trait exists so that the TUI and the CLI mode need not know which
/// endpoint is configured, and so tests can substitute a stub for the network.
pub trait Provider {
    /// Suggest up to `count` commands for the task described by `task`.
    fn suggest(&self, ctx: &Context, task: &str, count: usize) -> Result<Vec<Suggestion>>;
}
