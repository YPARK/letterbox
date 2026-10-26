//! A message's pages, typeset by ohp and drawn in the terminal, where it
//! draws images: letterbox built `--features pages`. The pages are drawn
//! off the screen's thread, and shown one at a time, each whole in the
//! pane.

use image::{DynamicImage, RgbaImage};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::path::Path;

/// Where a message is: its account, folder and UID, as one alone is
/// only one folder's.
pub type Place = (usize, String, u32);

/// The pages of the message at `place`, the one shown, and each as it is
/// drawn for the terminal, once it has been: a page is kept once, as drawn.
pub struct Paged {
    pub place: Place,
    pub at: usize,
    pages: Vec<Page>,
}

enum Page {
    Typeset(DynamicImage),
    Drawn(Box<StatefulProtocol>),
    /// Between the two.
    Moving,
}

impl Paged {
    pub fn new(place: Place, pages: Vec<RgbaImage>) -> Paged {
        Paged {
            place,
            at: 0,
            pages: pages
                .into_iter()
                .map(|p| Page::Typeset(DynamicImage::ImageRgba8(p)))
                .collect(),
        }
    }

    /// How many pages there are.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Go `by` pages on, or back, as far as there are.
    pub fn step(&mut self, by: isize) {
        let last = self.pages.len().saturating_sub(1) as isize;
        self.at = (self.at as isize + by).clamp(0, last) as usize;
    }

    /// The page shown, as `picker` draws it for the terminal: made the
    /// first time it is shown, and kept.
    pub fn drawn(&mut self, picker: &Picker) -> Option<&mut StatefulProtocol> {
        let page = self.pages.get_mut(self.at)?;
        if let Page::Typeset(_) = page
            && let Page::Typeset(image) = std::mem::replace(page, Page::Moving)
        {
            *page = Page::Drawn(Box::new(picker.new_resize_protocol(image)));
        }
        match page {
            Page::Drawn(p) => Some(p),
            _ => None,
        }
    }
}

/// The pages of the page written at `path`, `width` pixels wide.
pub fn typeset(path: &Path, width: u32) -> anyhow::Result<Vec<RgbaImage>> {
    ohp::page::pages(path, width)
}

/// `pages`, each made smaller where it is taller than `height` pixels or
/// wider than `width`, to be seen whole; none made larger.
pub fn fit(pages: Vec<RgbaImage>, width: u32, height: u32) -> Vec<RgbaImage> {
    pages
        .into_iter()
        .map(|p| {
            let (w, h) = p.dimensions();
            let scale = (width as f32 / w.max(1) as f32).min(height as f32 / h.max(1) as f32);
            if scale >= 1.0 {
                return p;
            }
            let (w, h) = (
                ((w as f32 * scale).round() as u32).max(1),
                ((h as f32 * scale).round() as u32).max(1),
            );
            image::imageops::resize(&p, w, h, image::imageops::FilterType::Triangle)
        })
        .collect()
}

/// Whether the terminal, as `env` says, is one that draws images: kitty,
/// WezTerm, Ghostty or iTerm2, and not seen through tmux or screen, which
/// pass none on though their variables say what is outside them. Only such
/// a one is asked how: a terminal that does not answer keeps the asking
/// reading the keys typed after it.
pub fn draws_images(env: impl Fn(&str) -> Option<String>) -> bool {
    let has = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    let term = env("TERM").unwrap_or_default().to_lowercase();
    let program = env("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    if has("TMUX") || has("STY") || term.starts_with("screen") || term.starts_with("tmux") {
        return false;
    }
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
