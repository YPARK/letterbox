//! A message's pages, typeset by ohp and drawn in the terminal, where it
//! draws images: letterbox built `--features pages`. The pages are drawn
//! off the screen's thread, and shown one at a time, as wide as the pane.

use image::{DynamicImage, RgbaImage};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::path::Path;

/// The pages of the message `uid`, the one shown, and how it is drawn for
/// the terminal, once it has been.
pub struct Paged {
    pub uid: u32,
    pub pages: Vec<RgbaImage>,
    pub at: usize,
    drawn: Option<(usize, StatefulProtocol)>,
}

impl Paged {
    pub fn new(uid: u32, pages: Vec<RgbaImage>) -> Paged {
        Paged {
            uid,
            pages,
            at: 0,
            drawn: None,
        }
    }

    /// Go `by` pages on, or back, as far as there are.
    pub fn step(&mut self, by: isize) {
        let last = self.pages.len().saturating_sub(1) as isize;
        self.at = (self.at as isize + by).clamp(0, last) as usize;
    }

    /// The page shown, as `picker` draws it for the terminal: made again
    /// only when another page is shown.
    pub fn drawn(&mut self, picker: &Picker) -> Option<&mut StatefulProtocol> {
        if self.drawn.as_ref().is_none_or(|(at, _)| *at != self.at) {
            let page = self.pages.get(self.at)?.clone();
            let protocol = picker.new_resize_protocol(DynamicImage::ImageRgba8(page));
            self.drawn = Some((self.at, protocol));
        }
        self.drawn.as_mut().map(|(_, p)| p)
    }
}

/// The pages of the page written at `path`, `width` pixels wide.
pub fn typeset(path: &Path, width: u32) -> anyhow::Result<Vec<RgbaImage>> {
    ohp::page::pages(path, width)
}

/// Whether the terminal, as `env` says, is one that draws images: kitty,
/// WezTerm, Ghostty or iTerm2. Only such a one is asked how: a terminal
/// that does not answer keeps the asking reading the keys typed after it.
pub fn draws_images(env: impl Fn(&str) -> Option<String>) -> bool {
    let has = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    let term = env("TERM").unwrap_or_default().to_lowercase();
    let program = env("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    has("KITTY_WINDOW_ID")
        || has("WEZTERM_EXECUTABLE")
        || has("GHOSTTY_RESOURCES_DIR")
        || ["kitty", "ghostty", "wezterm"]
            .iter()
            .any(|t| term.contains(t))
        || ["wezterm", "ghostty", "iterm.app"]
            .iter()
            .any(|p| program == *p)
}

/// What the terminal draws images with, asked of it, if it says it draws
/// them: none where it does not, or draws only coarse blocks.
pub fn ask_terminal() -> Option<Picker> {
    if !draws_images(|k| std::env::var(k).ok()) {
        return None;
    }
    let picker = Picker::from_query_stdio().ok()?;
    (picker.protocol_type() != ratatui_image::picker::ProtocolType::Halfblocks).then_some(picker)
}
