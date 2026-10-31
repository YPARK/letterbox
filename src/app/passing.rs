//! A passing word in the status line: said until what it waits for is
//! done, and cleared then by its kind, not by its wording, unless
//! something else was said since.
use super::App;

/// What a passing word in the status line waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transient {
    /// A message replied to, while it is read first.
    Reading,
    /// U, till all it asked is answered.
    LookingNew,
    /// \ put the message read out of sight, till one is read.
    Hidden,
    /// C-c or C-x, till the next key.
    Prefix,
}

/// A passing word, and what it said: the status line holds it while it
/// still says that.
#[derive(Clone, Debug)]
pub(crate) struct Passing {
    kind: Transient,
    said: String,
}

impl App {
    /// Say `text` in the status line, as a passing word of this kind.
    pub(super) fn say_passing(&mut self, kind: Transient, text: String) {
        self.status = text.clone();
        self.passing = Some(Passing { kind, said: text });
    }

    /// Whether the status line still says the passing word of this kind.
    pub(super) fn passing(&self, kind: Transient) -> bool {
        self.passing
            .as_ref()
            .is_some_and(|p| p.kind == kind && p.said == self.status)
    }

    /// Clear the passing word of this kind, if it is still said.
    pub(super) fn clear_passing(&mut self, kind: Transient) {
        if self.passing(kind) {
            self.status.clear();
            self.passing = None;
        }
    }
}
