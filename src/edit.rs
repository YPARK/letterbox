//! A text buffer edited as Emacs edits one, enough of the keys to write a
//! message: moving by character, line and word, killing and yanking, the
//! region from the mark, and searching as you type. The arrows do the same
//! as the movement keys, so a message can be written without them.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a key did to the buffer, or asked of the message being written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Done {
    Stay,
    Send,
    Cancel,
    /// `C-c C-a`: a file to be attached is asked for.
    Attach,
    /// `M-$`, Emacs's key to check spelling: the grammar of what is written
    /// checked.
    Check,
    /// `C-c` or `C-x` and a key neither does anything with.
    Undefined,
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
    /// Where `C-SPC` set the mark: the region is from there to point. Any
    /// change to the text lets it go.
    mark: Option<(usize, usize)>,
    /// The search being typed, if one is.
    search: Option<Search>,
    /// What was last searched for, for `C-s C-s` to search for again.
    last_search: String,
    /// The text and point before each step of changes, the last last, at
    /// most `STEPS` of them: `C-/` puts it back. An undo is a step too, so
    /// that after any other key it is undone in turn, as in Emacs.
    undo: Vec<(Vec<String>, usize, usize)>,
    /// While undoing, the step last put back: the next undo goes on from
    /// before it. Any other key ends it.
    undoing: Option<usize>,
    /// Whether the step being made is saved already: more changes by the
    /// same key, or typed in a row, are of it.
    stepped: bool,
    /// Where point was as the step began, saved with the text.
    step_at: (usize, usize),
    /// How many characters were typed in a row into the step, up to
    /// `TYPED`.
    typed: usize,
}

/// How many characters typed in a row are undone as one, as Emacs has it.
const TYPED: usize = 20;
/// How many steps are kept to undo, each the whole text as it was.
const STEPS: usize = 100;

/// A search as you type: what is typed, where it began, which way it
/// goes, where what it last found starts, and whether the last look failed.
/// What is typed has no line's end, as `⏎` ends the search: what it finds
/// is on one line.
struct Search {
    query: String,
    origin: (usize, usize),
    backward: bool,
    found: Option<(usize, usize)>,
    failing: bool,
}

/// Where a search looks from: where it began, past what it found, or at
/// what it found, as when it turns about.
#[derive(Clone, Copy)]
enum From {
    Origin,
    Past,
    At,
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
            mark: None,
            search: None,
            last_search: String::new(),
            undo: Vec::new(),
            undoing: None,
            stepped: false,
            step_at: (0, 0),
            typed: 0,
        }
    }

    /// The region, from its start to its end, if the mark is set.
    pub fn region(&self) -> Option<((usize, usize), (usize, usize))> {
        let mark = self.mark?;
        let point = (self.row, self.col);
        Some((mark.min(point), mark.max(point)))
    }

    /// The columns of line `row` in the region: past its last character
    /// when the region goes on past its end, as what `C-w` cuts takes the
    /// line's end too.
    pub fn marked(&self, row: usize) -> std::ops::Range<usize> {
        match self.region() {
            Some(((r0, c0), (r1, c1))) if (r0..=r1).contains(&row) => {
                let start = if row == r0 { c0 } else { 0 };
                let end = if row == r1 {
                    c1
                } else {
                    self.lines[row].chars().count() + 1
                };
                start..end
            }
            _ => 0..0,
        }
    }

    /// What the search being typed says of itself, as Emacs says it.
    pub fn searching(&self) -> Option<String> {
        let s = self.search.as_ref()?;
        let failing = if s.failing { "Failing " } else { "" };
        let way = if s.backward {
            "I-search backward"
        } else {
            "I-search"
        };
        Some(format!("{failing}{way}: {}", s.query))
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

    /// To the field `by` after the one the cursor is in, or before, round:
    /// each line of the head, To, Cc, Subject and the rest but its files'
    /// `Attach` lines, then what is written after it; at the end of what a
    /// field says, and of the first line of what is written.
    pub fn field(&mut self, by: isize) {
        let head = self.head();
        let mut stops: Vec<usize> = (0..head)
            .filter(|&r| !self.lines[r].starts_with("Attach:"))
            .collect();
        if head + 1 < self.lines.len() {
            stops.push(head + 1);
        }
        if stops.is_empty() {
            return;
        }
        // Anywhere in what is written is there; the line between, the last
        // field's.
        let here = stops.iter().rposition(|&r| r <= self.row).unwrap_or(0);
        let next = (here as isize + by).rem_euclid(stops.len() as isize) as usize;
        self.row = stops[next];
        self.col = self.lines[self.row].chars().count();
        self.preferred = self.col;
        self.killing = false;
    }

    /// The characters `start` to `end` of the row `row` made `with`, and
    /// the cursor past it.
    pub fn replace(&mut self, row: usize, start: usize, end: usize, with: &str) {
        let Some(line) = self.lines.get(row) else {
            return;
        };
        let (a, b) = (byte_at(line, start), byte_at(line, end));
        self.edited();
        self.lines[row].replace_range(a..b, with);
        self.row = row;
        self.col = start + with.chars().count();
        self.preferred = self.col;
    }

    /// `new` as lines above the row `at`, point staying on its text.
    pub fn insert_lines(&mut self, at: usize, new: &[String]) {
        self.begin_step();
        self.edited();
        let at = at.min(self.lines.len());
        self.lines.splice(at..at, new.iter().cloned());
        if self.row >= at {
            self.row += new.len();
        }
    }

    /// `text` put in at point, as typed, its lines broken where it breaks.
    pub fn paste(&mut self, text: &str) {
        self.begin_step();
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                self.newline();
            }
            self.insert_str(part);
        }
    }

    /// How many lines the head is: those before the first blank one.
    fn head(&self) -> usize {
        self.lines
            .iter()
            .position(|l| l.trim().is_empty())
            .unwrap_or(self.lines.len())
    }

    fn line(&self) -> &str {
        &self.lines[self.row]
    }

    /// The text changes: the mark is let go, as Emacs lets it go, and
    /// the text as it was saved, once a step.
    fn edited(&mut self) {
        self.mark = None;
        self.undoing = None;
        if !self.stepped {
            let (row, col) = self.step_at;
            self.save((self.lines.clone(), row, col));
            self.stepped = true;
        }
    }

    /// `step` kept to undo, the oldest let go past `STEPS`: whether one
    /// was.
    fn save(&mut self, step: (Vec<String>, usize, usize)) -> bool {
        self.undo.push(step);
        let over = self.undo.len() > STEPS;
        if over {
            self.undo.remove(0);
        }
        over
    }

    /// A step of changes begun: a key's, a paste, or the model's fixes put
    /// in, one or all of them.
    pub fn begin_step(&mut self) {
        // The step before changed nothing, as a backspace at the start:
        // nothing to undo.
        if self.stepped && self.undo.last().is_some_and(|s| s.0 == self.lines) {
            self.undo.pop();
        }
        self.stepped = false;
        self.typed = 0;
        self.step_at = (self.row, self.col);
    }

    /// `C-/`, `C-_` or `C-x u`: the text and point as they were before the
    /// last step that changed it, or, undoing still, the one before that.
    /// A search under way ends first, as the text it looked in goes.
    fn undo(&mut self) {
        self.search = None;
        let from = self.undoing.unwrap_or(self.undo.len());
        let Some(at) = (0..from).rev().find(|&i| self.undo[i].0 != self.lines) else {
            return;
        };
        let (lines, row, col) = self.undo[at].clone();
        let here = std::mem::replace(&mut self.lines, lines);
        let at = at - usize::from(self.save((here, self.row, self.col)));
        self.place(row, col);
        self.mark = None;
        self.undoing = Some(at);
    }

    /// `killed`, as what `C-y` puts back: added to the last kill when that
    /// was a kill too.
    fn add_kill(&mut self, killed: String, append: bool) {
        if append {
            self.kill.push_str(&killed);
        } else {
            self.kill = killed;
        }
    }

    fn insert(&mut self, c: char) {
        self.edited();
        let col = self.col;
        let line = &mut self.lines[self.row];
        let at = byte_at(line, col);
        line.insert(at, c);
        self.col += 1;
        self.preferred = self.col;
    }

    fn newline(&mut self) {
        self.edited();
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
        self.edited();
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
        self.edited();
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

    /// `C-a`: the line's start; in the head, past the field's name, as
    /// message-mode's C-a goes, and the line's start from there.
    fn line_start(&mut self) {
        self.col = match self.name_end() {
            Some(end) if self.col != end => end,
            _ => 0,
        };
        self.preferred = self.col;
    }

    /// In the head, where the field's name ends on point's line: past its
    /// colon and the space after it.
    fn name_end(&self) -> Option<usize> {
        if self.row >= self.head() {
            return None;
        }
        let line = self.line();
        let (name, rest) = line.split_once(':')?;
        if name.is_empty() || name.contains(char::is_whitespace) {
            return None;
        }
        Some(name.chars().count() + 1 + usize::from(rest.starts_with(' ')))
    }

    fn line_end(&mut self) {
        self.col = self.line().chars().count();
        self.preferred = self.col;
    }

    /// `C-k`: from point to the end of the line, or the newline when point
    /// is already there, added to the last kill when that was a kill too.
    fn kill_line(&mut self, append: bool) {
        self.edited();
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
        self.add_kill(killed, append);
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
        self.edited();
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
        if !killed.is_empty() {
            self.add_kill(killed, append);
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
        self.edited();
        let killed = self.text_between(start, end);
        let ((r0, c0), (r1, c1)) = (start, end);
        let tail = self.lines[r1][byte_at(&self.lines[r1], c1)..].to_string();
        let at = byte_at(&self.lines[r0], c0);
        self.lines[r0].truncate(at);
        self.lines[r0].push_str(&tail);
        self.lines.drain(r0 + 1..=r1);
        self.place(r0, c0);
        killed
    }

    /// The text from `start` to `end`, its lines' ends as newlines.
    fn text_between(&self, (r0, c0): (usize, usize), (r1, c1): (usize, usize)) -> String {
        let (first, last) = (&self.lines[r0], &self.lines[r1]);
        if r0 == r1 {
            return first[byte_at(first, c0)..byte_at(first, c1)].to_string();
        }
        let mut text = first[byte_at(first, c0)..].to_string();
        for line in &self.lines[r0 + 1..r1] {
            text.push('\n');
            text.push_str(line);
        }
        text.push('\n');
        text.push_str(&last[..byte_at(last, c1)]);
        text
    }

    /// `C-w`: the region, killed, added to the last kill when that was a
    /// kill too; nothing without a mark.
    fn kill_region(&mut self, append: bool) {
        if let Some((start, end)) = self.region() {
            let killed = self.take(start, end);
            self.add_kill(killed, append);
            self.mark = None;
        }
    }

    /// `M-w`: the region, copied to be yanked; the mark let go.
    fn copy_region(&mut self) {
        if let Some((start, end)) = self.region() {
            self.kill = self.text_between(start, end);
            self.mark = None;
        }
    }

    /// Begin searching as you type, forward or `backward`.
    fn begin_search(&mut self, backward: bool) {
        self.search = Some(Search {
            query: String::new(),
            origin: (self.row, self.col),
            backward,
            found: None,
            failing: false,
        });
    }

    /// `C-g`: the search let go, point back where it began; else the mark.
    /// Whether there was either to let go.
    fn quit(&mut self) -> bool {
        if let Some(s) = self.search.take() {
            self.place(s.origin.0, s.origin.1);
            return true;
        }
        self.mark.take().is_some()
    }

    /// Find the query, looking `from` where it says, the way the search
    /// goes. Point goes to the end of what is found going forward, its
    /// start going back; failing, it stays.
    fn find(&mut self, from: From) {
        let Some(s) = &mut self.search else { return };
        if s.query.is_empty() {
            (s.found, s.failing) = (None, false);
            let (row, col) = s.origin;
            self.place(row, col);
            return;
        }
        let n = s.query.chars().count();
        // Where a match may start: going forward, at or after; going back,
        // at or before, none on that line when the column is none.
        let (row, col) = match (from, s.found) {
            (From::Past, Some((r, c))) if s.backward => (r, c.checked_sub(1)),
            (From::Past, Some((r, c))) => (r, Some(c + 1)),
            (From::At, Some((r, c))) => (r, Some(c)),
            _ if s.backward => (s.origin.0, s.origin.1.checked_sub(n)),
            _ => (s.origin.0, Some(s.origin.1)),
        };
        let (backward, query) = (s.backward, s.query.clone());
        let hit = self.locate(&query, row, col, backward);
        let s = self.search.as_mut().expect("searching");
        s.failing = hit.is_none();
        if let Some((r, c)) = hit {
            s.found = hit;
            self.place(r, if backward { c } else { c + n });
        }
    }

    /// Where `query` starts, the nearest to line `row`, column `col`, the
    /// way the search goes: at or after it going forward, at or before it
    /// going back. A capital is looked for as it is; otherwise case is let go.
    fn locate(
        &self,
        query: &str,
        row: usize,
        col: Option<usize>,
        backward: bool,
    ) -> Option<(usize, usize)> {
        let fold = !query.chars().any(char::is_uppercase);
        let norm = |c: char| {
            if fold {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                c
            }
        };
        let q: Vec<char> = query.chars().map(norm).collect();
        let starts = |r: usize| -> Vec<usize> {
            let line: Vec<char> = self.lines[r].chars().map(norm).collect();
            (0..line.len())
                .filter(|&i| line[i..].starts_with(&q))
                .collect()
        };
        if backward {
            col.and_then(|c| starts(row).into_iter().rev().find(|&i| i <= c))
                .map(|i| (row, i))
                .or_else(|| (0..row).rev().find_map(|r| Some((r, *starts(r).last()?))))
        } else {
            let c = col.unwrap_or(0);
            starts(row)
                .into_iter()
                .find(|&i| i >= c)
                .map(|i| (row, i))
                .or_else(|| {
                    (row + 1..self.lines.len()).find_map(|r| Some((r, *starts(r).first()?)))
                })
        }
    }

    /// A key while searching: whether it was the search's. A key that is
    /// not ends the search where it is, and is then what it is.
    fn search_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let Some(s) = &mut self.search else {
            return false;
        };
        match key.code {
            // Some terminals say a letter with Ctrl in upper case.
            KeyCode::Char(c @ ('s' | 'S' | 'r' | 'R')) if ctrl => {
                let backward = c.eq_ignore_ascii_case(&'r');
                let turned = s.backward != backward;
                s.backward = backward;
                let from = if s.query.is_empty() {
                    s.query = self.last_search.clone();
                    From::Origin
                } else if turned {
                    From::At
                } else {
                    From::Past
                };
                self.find(from);
            }
            KeyCode::Enter => self.end_search(),
            KeyCode::Backspace => {
                s.query.pop();
                self.find(From::Origin);
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                s.query.push(c);
                self.find(From::Origin);
            }
            _ => {
                self.end_search();
                return false;
            }
        }
        true
    }

    fn end_search(&mut self) {
        if let Some(s) = self.search.take()
            && !s.query.is_empty()
        {
            self.last_search = s.query;
        }
    }
}

/// `key` named as Emacs names it: `C-z`, `M-f`, `SPC`, `RET`.
pub fn name(key: KeyEvent) -> String {
    let base = match key.code {
        KeyCode::Char(' ') => "SPC".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "RET".to_string(),
        KeyCode::Tab => "TAB".to_string(),
        KeyCode::Esc => "ESC".to_string(),
        KeyCode::Backspace => "DEL".to_string(),
        KeyCode::F(n) => format!("<f{n}>"),
        code => format!("<{code:?}>").to_lowercase(),
    };
    let mut named = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        named.push_str("C-");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        named.push_str("M-");
    }
    named + &base
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

/// Do what `key` does in `buf`. `prefix` is `c` once `C-c` is pressed, and
/// the next key sends, cancels, asks for a file, or is let go; `x` once
/// `C-x` is, and `u` undoes. Outside this buffer `C-c` quits; here it does
/// not.
pub fn handle(buf: &mut Buffer, prefix: &mut Option<char>, key: KeyEvent) -> Done {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    // A typed character goes on the step of those typed before it, up to
    // TYPED; any other key begins one.
    let typing = matches!(key.code, KeyCode::Char(_)) && !ctrl && !alt && prefix.is_none();
    if typing && (1..TYPED).contains(&buf.typed) {
        buf.typed += 1;
    } else {
        buf.begin_step();
        buf.typed = usize::from(typing);
    }
    // Any key but undo's, and C-x before its u, ends the undoing: undone
    // again after it, what was undone comes back, as in Emacs.
    let undoing = buf.undoing.take();
    if let Some(p) = prefix.take() {
        buf.killing = false;
        return match (p, key.code) {
            ('c', KeyCode::Char('c' | 'C')) => Done::Send,
            ('c', KeyCode::Char('k' | 'K')) => Done::Cancel,
            ('c', KeyCode::Char('a' | 'A')) => Done::Attach,
            // C-c C-s: the grammar checked, as M-$, no Meta needed.
            ('c', KeyCode::Char('s' | 'S')) => Done::Check,
            // C-x u undoes, as Emacs's.
            ('x', KeyCode::Char('u')) if !ctrl => {
                buf.undoing = undoing;
                buf.undo();
                Done::Stay
            }
            // C-c or C-x C-g, nothing; and anything else is undefined.
            (_, KeyCode::Char('g' | 'G')) if ctrl => Done::Stay,
            _ => Done::Undefined,
        };
    }
    if let KeyCode::Char(c @ ('c' | 'C' | 'x' | 'X')) = key.code
        && ctrl
    {
        *prefix = Some(c.to_ascii_lowercase());
        if *prefix == Some('x') {
            buf.undoing = undoing;
        }
        return Done::Stay;
    }
    // Undo's keys as the table has them, C-/ and C-_ as Emacs's.
    if crate::keymap::act(key) == Some(crate::keymap::Act::Undo) {
        buf.killing = false;
        buf.undoing = undoing;
        buf.undo();
        return Done::Stay;
    }
    // C-g lets the search go, else the mark, as Emacs's keyboard-quit
    // does; with nothing to let go, the message. esc as C-g, as it goes
    // back everywhere else.
    if ctrl && matches!(key.code, KeyCode::Char('g' | 'G')) || key.code == KeyCode::Esc {
        buf.killing = false;
        return if buf.quit() { Done::Stay } else { Done::Cancel };
    }
    if buf.search_key(key) {
        buf.killing = false;
        return Done::Stay;
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
        // C-SPC, which some terminals say as C-@.
        KeyCode::Char(' ' | '@') if ctrl => buf.mark = Some((buf.row, buf.col)),
        KeyCode::Char('w') if ctrl => {
            buf.kill_region(append);
            killing = true;
        }
        KeyCode::Char('w') if alt => buf.copy_region(),
        KeyCode::Char('s') if ctrl => buf.begin_search(false),
        KeyCode::Char('r') if ctrl => buf.begin_search(true),
        KeyCode::Char('$') if alt => return Done::Check,
        // Tab and Shift-Tab from field to field: To, Cc, Subject, the rest.
        KeyCode::Tab => buf.field(1),
        KeyCode::BackTab => buf.field(-1),
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
