//! The message read as its pages, typeset by ohp.
use super::*;

impl App {
    /// Where the message read is.
    #[cfg(feature = "pages")]
    pub fn read_place(&self) -> Option<crate::pages::Place> {
        let r = self.reading.as_ref()?;
        Some((r.account, r.folder.clone(), r.uid))
    }

    /// Show the message read as its pages, typeset off the screen's thread;
    /// shown so, its text again.
    #[cfg(feature = "pages")]
    pub(super) fn toggle_pages(&mut self) {
        let place = self.read_place();
        // Another message's pages are let go, not taken for its own.
        if let Some(paged) = self.paged.take()
            && Some(&paged.place) == place.as_ref()
        {
            self.status = "its text".to_string();
            return;
        }
        if self
            .typesetting
            .take()
            .is_some_and(|t| Some(&t) == place.as_ref())
        {
            self.status = "its text: its pages let go".to_string();
            return;
        }
        self.typeset(false);
    }

    /// Show the message read as its pages with its pictures from the web
    /// fetched, off the screen's thread; shown so already, again.
    #[cfg(feature = "pages")]
    pub(super) fn pages_with_images(&mut self) {
        self.typeset(true);
    }

    /// Typeset the message read as its pages, its pictures from the web
    /// fetched first if `images`, and show them once they are.
    #[cfg(feature = "pages")]
    fn typeset(&mut self, images: bool) {
        let place = self.read_place();
        let (Some(m), Some(place)) = (self.message().cloned(), place) else {
            self.status = "nothing to show: ⏎ reads the message picked".to_string();
            return;
        };
        // Asked as it is first wanted, between keys, none being read.
        let picker = self.picker.get_or_insert_with(crate::pages::ask_terminal);
        let Some(picker) = picker else {
            self.status =
                "this terminal draws no images: O hands the message to ohp instead".to_string();
            return;
        };
        let (cols, rows) = match self.page_room {
            (0, _) | (_, 0) => (80, 40),
            room => room,
        };
        let font = picker.font_size();
        let width = u32::from(cols) * u32::from(font.width.max(1));
        let height = u32::from(rows) * u32::from(font.height.max(1));
        self.status = if images {
            "fetching its pictures, then typesetting its pages…"
        } else {
            "typesetting its pages…"
        }
        .to_string();
        self.typesetting = Some(place.clone());
        let replies = self.replies_to.clone();
        self.pending += 1;
        std::thread::spawn(move || {
            let mut html = page_html(&m);
            if images {
                let agent = crate::net::agent();
                html = with_remote_images(&html, |url| crate::net::image(&agent, url));
            }
            let pages = write_html(m.uid, &html)
                .and_then(|path| crate::pages::typeset(&path, width))
                .map(|pages| crate::pages::fit(pages, width, height))
                .map_err(worker::text);
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Paged { place, pages },
            });
        });
    }

    #[cfg(not(feature = "pages"))]
    pub(super) fn toggle_pages(&mut self) {
        self.status = "letterbox was built without pages: cargo install letterbox --features pages; O hands it to ohp".to_string();
    }

    #[cfg(not(feature = "pages"))]
    pub(super) fn pages_with_images(&mut self) {
        self.toggle_pages();
    }

    /// A key while the message read is shown as its pages: they are turned;
    /// whether it was its to do.
    #[cfg(feature = "pages")]
    pub(super) fn pages_act(&mut self, act: Act) -> bool {
        let place = self.read_place();
        let Some(paged) = self
            .paged
            .as_mut()
            .filter(|p| Some(&p.place) == place.as_ref())
        else {
            return false;
        };
        // n and p still go to the next message and the one before.
        match act {
            Act::Down | Act::ScreenDown | Act::PageDown => paged.step(1),
            Act::Up | Act::ScreenUp | Act::PageUp => paged.step(-1),
            Act::Top => paged.at = 0,
            Act::End => paged.step(isize::MAX / 2),
            _ => return false,
        }
        true
    }
}
