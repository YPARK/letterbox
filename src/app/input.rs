//! Keys, the mouse and what is pasted: what each does where it is.
use super::*;

pub(super) const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How long a swipe of the wheel on the map is taken to go on: one level
/// in or out for it.
pub(super) const WHEEL_SWIPE: Duration = Duration::from_millis(300);

/// `word` with a capital, as a label is given one.
pub(super) fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Edit a line typed in, a field's or the search's, by `key`: a character
/// typed, `⌫` the last taken back, `C-u` all of it. Whether it changed.
pub(super) fn edit_line(line: &mut String, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Backspace => line.pop().is_some(),
        KeyCode::Char('u') if ctrl => {
            let had = !line.is_empty();
            line.clear();
            had
        }
        KeyCode::Char(c) if !ctrl => {
            line.push(c);
            true
        }
        _ => false,
    }
}

/// `key` as the table reads it: a letter said in lower case with Shift
/// alone, as some terminals say it once keys are told apart, in upper case,
/// as the rest say it.
pub(super) fn capital(key: KeyEvent) -> KeyEvent {
    match key.code {
        KeyCode::Char(c) if c.is_lowercase() && key.modifiers == KeyModifiers::SHIFT => {
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) => KeyEvent {
                    code: KeyCode::Char(u),
                    ..key
                },
                _ => key,
            }
        }
        _ => key,
    }
}

/// The files `text` names, as a terminal pastes those dropped on it:
/// their paths, each quoted or its spaces escaped, or their `file://`
/// addresses; none unless each is a file named from the root or home.
pub(super) fn dropped(text: &str) -> Vec<PathBuf> {
    let windows = cfg!(windows);
    let paths: Vec<PathBuf> = dropped_words(text, windows)
        .iter()
        .map(|w| match w.strip_prefix("file://") {
            Some(rest) => PathBuf::from(file_address(rest, windows)),
            None => compose::path_of(w),
        })
        .collect();
    if paths.iter().all(|p| p.is_absolute() && p.is_file()) {
        paths
    } else {
        Vec::new()
    }
}

/// The words of `text` as a terminal pastes paths: each quoted, or its
/// spaces escaped by a backslash; on `windows`, where a backslash parts
/// folders, quoted only.
pub(crate) fn dropped_words(text: &str, windows: bool) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = text.trim().chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '\\' && !windows => word.extend(chars.next()),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            None => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// The path of a `file://` address, after its `file://`: its escapes
/// read; on `windows`, the drive's, without the root's slash before it,
/// its folders parted by backslashes.
pub(crate) fn file_address(rest: &str, windows: bool) -> String {
    let path = unescaped(rest);
    if !windows {
        return path;
    }
    let b = path.as_bytes();
    let drive = b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':';
    let path = if drive { &path[1..] } else { &path[..] };
    path.replace('/', "\\")
}

/// `s` with each `%` and two hex digits the byte they stand for.
pub(super) fn unescaped(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A key as `letterbox screen` is given it: `<enter>`, `<esc>`, `<tab>`,
/// `<space>`, `<up>`, `<down>`, `<left>`, `<right>`, `<backspace>`, and
/// with control, as `<c-u>`, Alt, as `<m-g>`, or Super, as `<s-g>`;
/// anything else, each character typed.
pub fn keys(spec: &str) -> Vec<KeyEvent> {
    // `<c-s-r>` is `C-S-r`, `<s-tab>` is `S-tab`, `<enter>` is `enter`, as
    // the foot names them.
    if let Some(name) = spec.strip_prefix('<').and_then(|s| s.strip_suffix('>'))
        && let Some(key) = keymap::key_named(&match name {
            "s-tab" => "S-tab".to_string(),
            "s-up" => "S-up".to_string(),
            "s-down" => "S-down".to_string(),
            _ => name
                .replace("c-s-", "C-S-")
                .replace("c-", "C-")
                .replace("m-", "M-"),
        })
    {
        return vec![key];
    }
    spec.chars()
        .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        .collect()
}

/// What `v` says: several are selected with the mouse.
pub(super) const SELECT_BY_MOUSE: &str =
    "select with the mouse: drag over rows, or Ctrl and a click on one; S-↑↓ too";

impl App {
    pub fn key(&mut self, key: KeyEvent) {
        let key = capital(key);
        if self.compose.is_some() {
            if matches!(self.dialog, Dialog::Attach(_) | Dialog::Help(_)) {
                return self.dialog_key(key);
            }
            // F1, as Emacs has it: every key, over the message.
            if key.code == KeyCode::F(1) {
                self.dialog = Dialog::Help(0);
                return;
            }
            return self.compose_key(key);
        }
        // C-x C-c quits, as Emacs does; C-x and anything else, nothing,
        // and C-c alone, nothing: compose's C-c C-c, pressed once more than
        // it needed, quits no more.
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if std::mem::take(&mut self.c_x) {
            if ctrl && key.code == KeyCode::Char('c') {
                self.quit = true;
            } else if !ctrl && key.code == KeyCode::Char('u') {
                self.undo_last();
            } else {
                self.status = format!("{} quits", keymap::QUIT);
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            self.status = format!("{} quits", keymap::QUIT);
            return;
        }
        if ctrl
            && key.code == KeyCode::Char('x')
            && !self.dialog.is_open()
            && self.menu.is_none()
            && !self.searching
        {
            self.c_x = true;
            self.status = "C-x-".to_string();
            return;
        }
        if self.dialog.is_open() {
            return self.dialog_key(key);
        }
        if self.menu.is_some() {
            return self.menu_key(key);
        }
        if self.searching {
            // esc, or C-g as Emacs has it, lets it go.
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let off = key.code == KeyCode::Esc || ctrl && key.code == KeyCode::Char('g');
            match key.code {
                _ if off => {
                    self.query.clear();
                    self.found = None;
                    self.map.hits = None;
                    self.searching = false;
                }
                // On the map, what is found is lit there; nothing is asked of
                // the servers.
                KeyCode::Enter if self.map.mode == Mode::Map => {
                    self.searching = false;
                    let now = jiff::Timestamp::now().as_second();
                    let found = self.map.seek(&self.query, now);
                    let of = match self.map.searched_in() {
                        Some(name) => format!(" in {name}, ] [ the clusters beside it"),
                        None => String::new(),
                    };
                    self.status =
                        format!("{found} found{of} · n next, p the one before, esc lets it go");
                }
                KeyCode::Enter => {
                    self.searching = false;
                    self.search();
                }
                KeyCode::Down | KeyCode::Up => self.searching = false,
                _ => {
                    if edit_line(&mut self.query, key) {
                        self.found = None;
                    }
                }
            }
            self.narrow();
            if self.map.mode == Mode::Map {
                self.map
                    .seek(&self.query, jiff::Timestamp::now().as_second());
            }
            return;
        }
        if let Some(act) = keymap::act(key) {
            self.act(act);
        } else if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::NONE {
            // q quit once: said how it is done now.
            self.status = format!("{} quits", keymap::QUIT);
        }
    }

    /// Do what a key in the table does, where the focus is.
    pub(super) fn act(&mut self, act: Act) {
        // Shift and the arrows mark on from where they began; any other key
        // begins anew.
        if !matches!(act, Act::MarkDown | Act::MarkUp) {
            self.marking = None;
            if let Some(r) = &mut self.review {
                r.anchor = None;
            }
        }
        if self.review_act(act) {
            return;
        }
        if self.map.mode == Mode::Map && self.focus == Focus::List && self.map_act(act) {
            return;
        }
        if self.map.mode == Mode::List && self.focus != Focus::Side && self.list_marked_act(act) {
            return;
        }
        #[cfg(feature = "pages")]
        if self.focus == Focus::Reader && self.pages_act(act) {
            return;
        }
        let page = self.height.saturating_sub(2).max(1) as isize;
        match act {
            Act::Help => self.dialog = Dialog::Help(0),
            Act::Review => self.open_review(),
            Act::Apply => {
                self.status =
                    "$ does what was chosen in the Inbox looked through: i opens it".to_string()
            }
            Act::Search => {
                self.searching = true;
                if self.focus == Focus::Side {
                    self.focus = Focus::List;
                }
            }
            Act::Back if self.focus == Focus::Reader => self.focus = Focus::List,
            Act::Back if !self.query.is_empty() => {
                self.query.clear();
                self.found = None;
                self.narrow();
            }
            // Out of what was found, back to what was listed before.
            Act::Back
                if self
                    .listing
                    .as_ref()
                    .is_some_and(|l| matches!(l.view, View::Found(..))) =>
            {
                if let Some(view) = self.before_found.take() {
                    self.open_view(view);
                    self.status.clear();
                }
            }
            Act::Back => {}
            // The map has no sidebar: between it and the message read.
            Act::NextPane | Act::PreviousPane if self.map.mode == Mode::Map => {
                self.focus = if self.focus == Focus::List
                    && (self.reading.is_some() || self.review.is_some())
                {
                    Focus::Reader
                } else {
                    Focus::List
                };
            }
            Act::NextPane => {
                self.focus = match self.focus {
                    Focus::Side => Focus::List,
                    Focus::List if self.reading.is_some() => Focus::Reader,
                    Focus::List | Focus::Reader => Focus::Side,
                }
            }
            Act::PreviousPane => {
                self.focus = match self.focus {
                    Focus::Side if self.reading.is_some() => Focus::Reader,
                    Focus::Side | Focus::Reader => Focus::List,
                    Focus::List => Focus::Side,
                }
            }
            Act::Left => {
                self.focus = match self.focus {
                    Focus::Reader => Focus::List,
                    _ => Focus::Side,
                }
            }
            Act::Right => match self.focus {
                Focus::Side => {
                    self.open_row(true);
                    self.focus = Focus::List;
                }
                _ => {
                    self.read_picked();
                    if self.reading.is_some() {
                        self.focus = Focus::Reader;
                    }
                }
            },
            Act::Open => match self.focus {
                Focus::Side if self.rows.get(self.row) == Some(&Row::Add) => self.open_form(),
                Focus::Side => {
                    self.due = None;
                    self.open_row(true);
                    self.focus = Focus::List;
                }
                _ => {
                    self.due = None;
                    self.read_picked();
                    self.focus = Focus::Reader;
                }
            },
            Act::Down => self.step(1),
            Act::Up => self.step(-1),
            Act::MarkDown | Act::MarkUp
                if self.map.mode == Mode::List && self.focus == Focus::List =>
            {
                self.list_extend(if act == Act::MarkDown { 1 } else { -1 })
            }
            Act::PageDown if self.focus == Focus::Side => self.step(5),
            Act::PageUp if self.focus == Focus::Side => self.step(-5),
            Act::ScreenDown | Act::ScreenUp if self.focus == Focus::Side => {}
            Act::PageDown | Act::ScreenDown => self.scroll_by(page),
            Act::PageUp | Act::ScreenUp => self.scroll_by(-page),
            Act::Top => self.jump(false),
            Act::End => self.jump(true),
            Act::ReplyAll => self.respond(Respond::All),
            Act::Reply => self.respond(Respond::Reply),
            Act::Forward => self.respond(Respond::Forward),
            Act::NewMail => {
                self.refresh();
                self.status = "looking for new mail…".to_string();
            }
            Act::NextMessage => self.next(1),
            Act::PreviousMessage => self.next(-1),
            Act::Seen => self.toggle(Flag::Seen),
            Act::Flag => self.toggle(Flag::Flagged),
            Act::Lens
            | Act::Since
            | Act::NewOnly
            | Act::ShowRead
            | Act::ShowSent
            | Act::Unfiled => self.view(act),
            Act::View if self.map.mode == Mode::Map => self.dialog = Dialog::View,
            // Several are selected with the mouse, never by a key pressed
            // alone: one is too easily pressed on the wrong one.
            Act::Mark => self.status = SELECT_BY_MOUSE.to_string(),
            // Elsewhere, Shift and the arrows move, as the arrows do.
            Act::MarkDown => self.step(1),
            Act::MarkUp => self.step(-1),
            Act::Move | Act::NewLabel | Act::FileFirst | Act::NotIt
                if self.map.mode == Mode::List =>
            {
                self.list_file(act)
            }
            Act::Move | Act::NewLabel | Act::FileFirst | Act::NotIt => {
                self.status =
                    "on the map, tab to it: m, N, y and - act on the message picked there"
                        .to_string();
            }
            Act::FileAll if self.map.mode == Mode::List => self.list_file_all(),
            Act::FileAll
            | Act::View
            | Act::ZoomIn
            | Act::ZoomOut
            | Act::ZoomAll
            | Act::Relayout => {
                self.status = "that is the map's: M or M-g shows it".to_string();
            }
            Act::Wider | Act::Narrower if self.read_width > 0 => {
                let by: i16 = if act == Act::Wider { 4 } else { -4 };
                self.reader_width = Some(self.read_width.saturating_add_signed(by));
                self.remember_look();
            }
            Act::Wider | Act::Narrower => {}
            Act::MapView => self.show_map(),
            Act::SwitchView if self.map.mode == Mode::Map => self.show_list(),
            Act::SwitchView => self.show_map(),
            Act::ListView => self.show_list(),
            Act::PreviousCluster => self.map.cycle(-1),
            Act::NextCluster => self.map.cycle(1),
            Act::Files => self.list_files(),
            Act::Save => self.list_to_save(),
            Act::SaveAll => {
                self.list_to_save();
                self.pick_save_dir(true);
            }
            Act::Debrief => self.debrief(),
            Act::Junk => self.show_junk(),
            Act::Papers => self.list_papers(),
            Act::Pages => self.toggle_pages(),
            Act::Present => self.present(),
            Act::Archive => self.archive(),
            Act::AddAccount => self.open_form(),
            Act::Remove => self.ask_to_remove(),
            Act::Trash => self.delete(),
            Act::Compose => self.compose_new(),
            Act::Rename => self.ask_to_rename(),
        }
    }

    /// Take a click or a turn of the wheel at where it lands.
    pub fn mouse(&mut self, m: MouseEvent) {
        let at = self
            .targets
            .iter()
            .rev()
            .find(|(r, _)| r.contains(Position::new(m.column, m.row)))
            .map(|&(_, t)| t);
        // Writing, its keys are clicked, and nothing behind it; the files
        // being picked, or the help, as they are elsewhere.
        if self.compose.is_some() && !matches!(self.dialog, Dialog::Attach(_) | Dialog::Help(_)) {
            if m.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(key @ (Target::Key(_) | Target::Chord(..))) = at
            {
                self.click(Some(key));
            }
            return;
        }
        let by = match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if at == Some(Target::Divider) && !self.dialog.is_open() {
                    self.resizing = true;
                    return;
                }
                // The labels dropped down: a click elsewhere puts them away.
                if self.menu.is_some() && !matches!(at, Some(Target::MenuItem(_))) {
                    self.menu = None;
                    return;
                }
                // A row of the list or of the Inbox looked through: Ctrl
                // and a click add it or take it away, Shift and a click
                // select as far as it; pressed on, it may be dragged over.
                if let Some(row @ (Target::Message(_) | Target::ReviewRow(_))) = at
                    && !self.dialog.is_open()
                {
                    if m.modifiers.contains(KeyModifiers::CONTROL) {
                        return self.select_one(row);
                    }
                    if m.modifiers.contains(KeyModifiers::SHIFT) {
                        return self.select_as_far_as(row);
                    }
                    self.dragged_from = Some(row);
                }
                return self.click(at);
            }
            MouseEventKind::Drag(MouseButton::Left) if self.resizing => {
                self.reader_width = Some(self.panes_right.saturating_sub(m.column));
                return;
            }
            MouseEventKind::Up(MouseButton::Left) if self.resizing => {
                self.resizing = false;
                self.remember_look();
                return;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let (Some(from), Some(to)) = (self.dragged_from, at) {
                    self.select_between(from, to);
                }
                return;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.dragged_from = None;
                return;
            }
            // A right-click on a cluster's anchor: what may be done with it.
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(Target::Anchor(id)) = at
                    && !self.dialog.is_open()
                {
                    self.cluster_menu(id);
                }
                return;
            }
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => return,
        };
        // A dialog open, nothing behind it scrolls.
        if self.dialog.is_open() {
            return self.dialog.step(by);
        }
        match at {
            Some(Target::Side | Target::Row(_)) => {
                let before = self.focus;
                self.focus = Focus::Side;
                self.step(by);
                self.focus = before;
            }
            Some(Target::List | Target::Message(_)) => self.pick(by),
            Some(Target::Reader) => self.scroll_by(3 * by),
            // The wheel on the Inbox looked through goes along it.
            Some(Target::ReviewRow(_) | Target::Choose(..)) => self.review_step(by),
            Some(t @ (Target::Map | Target::Anchor(_) | Target::Branch(_))) => {
                self.map_wheel(t, by)
            }
            _ => {}
        }
    }

    pub(super) fn click(&mut self, at: Option<Target>) {
        if self.dialog.is_open() {
            return self.dialog_click(at);
        }
        self.searching = false;
        match at {
            Some(Target::Key(key)) => self.key(key),
            Some(Target::Chord(a, b)) => {
                self.key(a);
                self.key(b);
            }
            Some(Target::Search) => self.searching = true,
            Some(Target::Side) => self.focus = Focus::Side,
            Some(Target::Row(i)) if self.rows.get(i) == Some(&Row::Add) => {
                self.row = i;
                self.open_form();
            }
            Some(Target::Row(i)) => {
                self.focus = Focus::Side;
                if matches!(
                    self.rows.get(i),
                    Some(Row::Folder(..) | Row::All(_) | Row::Map | Row::Cluster(_) | Row::Loners)
                ) {
                    self.row = i;
                    self.open_row(true);
                    self.focus = Focus::List;
                }
            }
            Some(Target::List | Target::Map) => self.focus = Focus::List,
            Some(Target::Anchor(id)) => self.anchor_click(id),
            Some(Target::Branch(row)) => self.branch_click(row),
            Some(Target::ReviewRow(row)) => self.review_click(row),
            Some(Target::Choose(row, which)) => self.choice_click(row, which),
            Some(Target::MenuItem(at)) => self.pick_item(at),
            Some(Target::Message(i)) => {
                self.focus = Focus::List;
                self.list.select(Some(i));
                let again = self
                    .clicked
                    .is_some_and(|(when, j)| j == i && when.elapsed() < DOUBLE_CLICK);
                self.clicked = (!again).then(|| (Instant::now(), i));
                self.due = None;
                if self.side_by_side || again {
                    self.read_picked();
                }
                if again || !self.side_by_side {
                    self.focus = Focus::Reader;
                }
                self.more();
            }
            Some(Target::Reader) if self.reading.is_some() => self.focus = Focus::Reader,
            _ => {}
        }
    }
}
