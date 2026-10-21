//! A text buffer edited as Emacs edits one, enough of the keys to write a
//! message: moving by character, line and word, killing and yanking. The
//! arrows do the same as the movement keys, so a message can be written
//! without them.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a key did to the buffer, or asked of the message being written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Done {
    Stay,
    Send,
    Cancel,
}

pub struct Buffer {
    lines: Vec<String>,
    row: usize,
    col: usize,
    /// The column `C-n` and `C-p` aim for, past a shorter line.
    preferred: usize,
    kill: String,
    /// The last key killed text, so a second `C-k` adds to the same kill.
    killing: bool,
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(str::to_string).collect()
        };
        Buffer {
            lines,
            row: 0,
            col: 0,
            preferred: 0,
            kill: String::new(),
            killing: false,
        }
    }

    pub fn text(&self) -> String {
        let mut s = self.lines.join("\n");
        s.push('\n');
        s
    }

    pub fn row(&self) -> usize {
        self.row
    }

    pub fn col(&self) -> usize {
        self.col
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn place(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len().saturating_sub(1));
        let n = self.lines[self.row].chars().count();
        self.col = col.min(n);
        self.preferred = self.col;
    }

    fn line(&self) -> &str {
        &self.lines[self.row]
    }

    fn insert(&mut self, c: char) {
        let col = self.col;
        let line = &mut self.lines[self.row];
        let at = byte_at(line, col);
        line.insert(at, c);
        self.col += 1;
        self.preferred = self.col;
    }

    fn newline(&mut self) {
        let col = self.col;
        let line = &mut self.lines[self.row];
        let at = byte_at(line, col);
        let rest = line[at..].to_string();
        line.truncate(at);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
        self.preferred = 0;
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
            self.delete_char();
            self.preferred = self.col;
            return;
        }
        if self.row == 0 {
            return;
        }
        let line = self.lines.remove(self.row);
        self.row -= 1;
        self.col = self.lines[self.row].chars().count();
        self.lines[self.row].push_str(&line);
        self.preferred = self.col;
    }

    /// `C-d`: the character after point, or the line's end when there.
    fn delete_forward(&mut self) {
        if self.col < self.line().chars().count() {
            self.delete_char();
            return;
        }
        if self.row + 1 >= self.lines.len() {
            return;
        }
        let next = self.lines.remove(self.row + 1);
        self.lines[self.row].push_str(&next);
    }

    fn delete_char(&mut self) {
        let col = self.col;
        let line = &mut self.lines[self.row];
        let start = byte_at(line, col);
        let end = byte_at(line, col + 1);
        line.replace_range(start..end, "");
    }

    fn move_char(&mut self, by: isize) {
        if by < 0 {
            if self.col > 0 {
                self.col -= 1;
            } else if self.row > 0 {
                self.row -= 1;
                self.col = self.line().chars().count();
            }
        } else if self.col < self.line().chars().count() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
        self.preferred = self.col;
    }

    fn move_line(&mut self, by: isize) {
        if by < 0 {
            if self.row == 0 {
                self.col = 0;
                return;
            }
            self.row -= 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
        } else {
            self.col = self.line().chars().count();
            return;
        }
        self.col = self.preferred.min(self.line().chars().count());
    }

    fn line_start(&mut self) {
        self.col = 0;
        self.preferred = 0;
    }

    fn line_end(&mut self) {
        self.col = self.line().chars().count();
        self.preferred = self.col;
    }

    /// `C-k`: from point to the end of the line, or the newline when point
    /// is already there, added to the last kill when that was a kill too.
    fn kill_line(&mut self, append: bool) {
        let col = self.col;
        let n = self.line().chars().count();
        let killed = if col < n {
            let line = &mut self.lines[self.row];
            let at = byte_at(line, col);
            let killed = line[at..].to_string();
            line.truncate(at);
            killed
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
            "\n".to_string()
        } else {
            return;
        };
        if append {
            self.kill.push_str(&killed);
        } else {
            self.kill = killed;
        }
    }

    fn yank(&mut self) {
        let text = self.kill.clone();
        if text.is_empty() {
            return;
        }
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                self.newline();
            }
            self.insert_str(part);
        }
    }

    fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let col = self.col;
        let line = &mut self.lines[self.row];
        let at = byte_at(line, col);
        line.insert_str(at, s);
        self.col += s.chars().count();
        self.preferred = self.col;
    }

    fn word(&mut self, by: isize) {
        if by > 0 {
            self.forward_word();
        } else {
            self.backward_word();
        }
        self.preferred = self.col;
    }

    fn forward_word(&mut self) {
        while self.char_at().is_some_and(|c| !is_word(c)) {
            if !self.step(1) {
                return;
            }
        }
        while self.char_at().is_some_and(is_word) {
            if !self.step(1) {
                return;
            }
        }
    }

    fn backward_word(&mut self) {
        if !self.step(-1) {
            return;
        }
        while self.char_at().is_some_and(|c| !is_word(c)) {
            if !self.step(-1) {
                return;
            }
        }
        while self.char_at().is_some_and(is_word) {
            if !self.step(-1) {
                self.col = 0;
                return;
            }
        }
        self.step(1);
    }

    /// `M-d`: from point through the next word.
    fn kill_word(&mut self, append: bool) {
        let start = (self.row, self.col);
        self.forward_word();
        let end = (self.row, self.col);
        let killed = self.take(start, end);
        if killed.is_empty() {
            return;
        }
        if append {
            self.kill.push_str(&killed);
        } else {
            self.kill = killed;
        }
    }

    /// The character at point, across the line's end as a newline.
    fn char_at(&self) -> Option<char> {
        self.line()
            .chars()
            .nth(self.col)
            .or_else(|| (self.row + 1 < self.lines.len()).then_some('\n'))
    }

    /// Move one character, onto the next line at its end; whether it could.
    fn step(&mut self, by: isize) -> bool {
        let before = (self.row, self.col);
        self.move_char(by);
        (self.row, self.col) != before
    }

    /// Take the text from `start` to `end` out of the buffer; point is left
    /// at `start`.
    fn take(&mut self, start: (usize, usize), end: (usize, usize)) -> String {
        if start >= end {
            return String::new();
        }
        let (r0, c0) = start;
        let (r1, c1) = end;
        if r0 == r1 {
            let line = &mut self.lines[r0];
            let a = byte_at(line, c0);
            let b = byte_at(line, c1);
            let killed = line[a..b].to_string();
            line.replace_range(a..b, "");
            self.row = r0;
            self.col = c0;
            return killed;
        }
        let mut killed = String::new();
        killed.push_str(&self.lines[r0][byte_at(&self.lines[r0], c0)..]);
        for line in &self.lines[r0 + 1..r1] {
            killed.push('\n');
            killed.push_str(line);
        }
        killed.push('\n');
        killed.push_str(&self.lines[r1][..byte_at(&self.lines[r1], c1)]);
        let tail = self.lines[r1][byte_at(&self.lines[r1], c1)..].to_string();
        let at = byte_at(&self.lines[r0], c0);
        self.lines[r0].truncate(at);
        self.lines[r0].push_str(&tail);
        self.lines.drain(r0 + 1..=r1);
        self.row = r0;
        self.col = c0;
        killed
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// The byte index of character `col` in `line`.
fn byte_at(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(i, _)| i)
        .unwrap_or(line.len())
}

/// Do what `key` does in `buf`. `prefix` is set once `C-c` is pressed, and
/// the next key sends, cancels, or is let go. Outside this buffer `C-c`
/// quits; here it does not.
pub fn handle(buf: &mut Buffer, prefix: &mut bool, key: KeyEvent) -> Done {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if *prefix {
        *prefix = false;
        buf.killing = false;
        return match key.code {
            KeyCode::Char('c' | 'C') => Done::Send,
            KeyCode::Char('k' | 'K') => Done::Cancel,
            _ => Done::Stay,
        };
    }
    if ctrl && matches!(key.code, KeyCode::Char('c' | 'C')) {
        *prefix = true;
        return Done::Stay;
    }
    if ctrl && matches!(key.code, KeyCode::Char('g' | 'G')) {
        buf.killing = false;
        return Done::Cancel;
    }
    let append = buf.killing;
    let mut killing = false;
    match key.code {
        KeyCode::Left | KeyCode::Char('b') if ctrl => buf.move_char(-1),
        KeyCode::Right | KeyCode::Char('f') if ctrl => buf.move_char(1),
        KeyCode::Up | KeyCode::Char('p') if ctrl => buf.move_line(-1),
        KeyCode::Down | KeyCode::Char('n') if ctrl => buf.move_line(1),
        KeyCode::Home | KeyCode::Char('a') if ctrl => buf.line_start(),
        KeyCode::End | KeyCode::Char('e') if ctrl => buf.line_end(),
        KeyCode::Char('d') if ctrl => buf.delete_forward(),
        KeyCode::Delete => buf.delete_forward(),
        KeyCode::Backspace => buf.backspace(),
        KeyCode::Enter => buf.newline(),
        KeyCode::Char('k') if ctrl => {
            buf.kill_line(append);
            killing = true;
        }
        KeyCode::Char('y') if ctrl => buf.yank(),
        KeyCode::Char('f') if alt => buf.word(1),
        KeyCode::Char('b') if alt => buf.word(-1),
        KeyCode::Char('d') if alt => {
            buf.kill_word(append);
            killing = true;
        }
        KeyCode::Left => buf.move_char(-1),
        KeyCode::Right => buf.move_char(1),
        KeyCode::Up => buf.move_line(-1),
        KeyCode::Down => buf.move_line(1),
        KeyCode::Home => buf.line_start(),
        KeyCode::End => buf.line_end(),
        KeyCode::Char(c) if !ctrl && !alt => buf.insert(c),
        _ => {}
    }
    buf.killing = killing;
    Done::Stay
}
