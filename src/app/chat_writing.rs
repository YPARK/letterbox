//! A Slack message written in letterbox: to the conversation read, or in
//! the thread read, with any files attached, as mail's are, by C-c C-a or
//! dropped onto the terminal. It goes as it is sent, C-⏎: Slack is for
//! quick messages.
use super::chat::Writing;
use super::*;
use crate::slack::worker::Ask as SlackAsk;

impl App {
    /// Write to the conversation read; or, from the thread read, in it.
    pub(super) fn write_talk(&mut self, in_thread: bool) {
        let Some(open) = &self.chat.open else {
            return;
        };
        let thread = in_thread
            .then(|| open.thread.as_ref().map(|t| t.ts.clone()))
            .flatten();
        self.chat.writing = Some(Writing {
            space: open.space,
            channel: open.id.clone(),
            name: open.name.clone(),
            thread,
            buffer: crate::edit::Buffer::from_text(""),
            prefix: None,
            files: Vec::new(),
            sending: false,
            error: None,
        });
        self.status =
            "C-⏎ sends it (M-⏎ or C-c C-c too) · C-c C-a attaches · esc lets it go".to_string();
    }

    /// A key while a message is written: to it, or sending it.
    pub(super) fn chat_write_key(&mut self, key: KeyEvent) {
        let Some(writing) = &mut self.chat.writing else {
            return;
        };
        if writing.sending {
            self.status = "sending…".to_string();
            return;
        }
        // Ctrl+Enter, as chat apps send with, where the terminal tells it from
        // Enter; Alt+Enter, which every terminal does; and C-c C-c, as mail's.
        let send_key = key.code == KeyCode::Enter
            && key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && writing.prefix.is_none();
        let done = if send_key {
            Done::Send
        } else {
            crate::edit::handle(&mut writing.buffer, &mut writing.prefix, key)
        };
        match done {
            Done::Send if writing.buffer.text().trim().is_empty() && writing.files.is_empty() => {
                self.status = "nothing written to send".to_string();
            }
            Done::Send => {
                writing.sending = true;
                writing.error = None;
                let ask = SlackAsk::Post {
                    channel: writing.channel.clone(),
                    thread: writing.thread.clone(),
                    // The line the buffer ends with is not sent.
                    text: writing.buffer.text().trim_end().to_string(),
                    files: writing.files.clone(),
                };
                let space = writing.space;
                self.status = format!("sending to {}…", writing.name);
                self.chat.ask(space, ask);
            }
            Done::Cancel => {
                self.chat.writing = None;
                self.status = "let go, not sent".to_string();
            }
            Done::Attach => {
                let dir = files::start_dir(self.attach_dir.clone());
                self.dialog = Dialog::Attach(Box::new(crate::pick::Picker::open(dir)));
            }
            _ => {}
        }
    }

    /// Attach `paths` to the Slack message being written, each once;
    /// whether one is being written.
    pub(super) fn chat_attach(&mut self, paths: &[PathBuf]) -> bool {
        let Some(writing) = self.chat.writing.as_mut().filter(|w| !w.sending) else {
            return false;
        };
        let new: Vec<PathBuf> = paths
            .iter()
            .filter(|p| !writing.files.contains(p))
            .cloned()
            .collect();
        self.status = match new.as_slice() {
            [] => "attached already".to_string(),
            [one] => format!("attached {}", compose::named(one)),
            more => format!("attached {} files", more.len()),
        };
        writing.files.extend(new);
        true
    }

    /// What is pasted, into the message being written: a file dropped onto
    /// the terminal, attached.
    pub(super) fn chat_paste(&mut self, text: &str) -> bool {
        match &mut self.chat.writing {
            Some(w) if !w.sending => {
                let paths = input::dropped(text);
                if paths.is_empty() {
                    w.buffer.paste(text);
                } else {
                    self.chat_attach(&paths);
                }
                true
            }
            Some(_) => true,
            None => false,
        }
    }
}
