pub mod select;
pub mod tui;

/// What the user decided to do with the chosen suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Run the suggestion at this index
    Run(usize),
    /// Insert it into the shell's prompt buffer without running it
    Buffer(usize),
    /// Copy to the clipboard and exit
    Copy(usize),
    /// The suggestions were printed with no way to pick one, because there was
    /// no terminal to pick with. Nothing ran, but nothing failed either.
    Listed,
    /// Cancel: do nothing
    Cancel,
}
