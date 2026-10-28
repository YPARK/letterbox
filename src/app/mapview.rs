//! The map, looked at: shown in the list's place, what of it is shown and
//! how, the message picked on it, moved between and read.
use super::*;

/// The map's index, its key made if it was not, where letterbox keeps it.
pub(super) fn open_map_index() -> Result<Index, String> {
    Index::open(
        &crate::index::key::dir(),
        crate::secret::keepers,
        crate::secret::find,
        crate::secret::store,
    )
}

impl App {
    /// Do what a key does in the map, with the focus on it; whether it
    /// was the map's to do.
    pub(super) fn map_act(&mut self, act: Act) -> bool {
        if self.map.preview.is_some() {
            match act {
                Act::Open => {
                    self.keep_relayout();
                    return true;
                }
                Act::Back => {
                    self.map.preview = None;
                    self.status = "the map as it was".to_string();
                    return true;
                }
                _ => {}
            }
        }
        if act == Act::Relayout {
            self.relayout();
            return true;
        }
        // The map is looked at: what moves mail, or marks it to, is done
        // down at a message in a cluster's tree, reading it, or in the
        // Inbox looked through; never on the clusters, where a key is so
        // easily pressed on the wrong one.
        let moves = matches!(
            act,
            Act::Trash
                | Act::Archive
                | Act::Move
                | Act::NewLabel
                | Act::FileFirst
                | Act::FileAll
                | Act::NotIt
                | Act::Mark
        );
        let now = jiff::Timestamp::now().as_second();
        if moves && self.map.moving.is_none() && (act == Act::Mark || !self.map.at_message(now)) {
            self.status = "the map is looked at: ⏎ goes down to a message, or i looks through the Inbox, to act on mail".to_string();
            return true;
        }
        if self.map.moving.is_some() {
            match act {
                Act::Open => {
                    let id = self
                        .map
                        .picked()
                        .and_then(|i| self.map.snapshot.as_ref()?.group_of(i).map(|g| g.id));
                    if let Some(id) = id {
                        self.move_onto(id);
                    }
                    return true;
                }
                Act::NewLabel => {
                    self.ask_new_label();
                    return true;
                }
                Act::Back => {
                    self.map.moving = None;
                    self.status = "not moved".to_string();
                    // Picking where a message looked through goes: back to it.
                    if let Some(r) = &mut self.review
                        && r.relabel.take().is_some()
                    {
                        self.focus = Focus::Reader;
                    }
                    return true;
                }
                _ => {}
            }
        }
        match act {
            Act::Move => {
                self.start_moving();
                return true;
            }
            Act::FileFirst => {
                self.file_suggested(false);
                return true;
            }
            Act::FileAll => {
                self.file_suggested(true);
                return true;
            }
            Act::NotIt => {
                self.not_it();
                return true;
            }
            Act::Rename => {
                self.ask_rename_label();
                return true;
            }
            Act::Back if !self.query.is_empty() && self.map.region.is_none() => {
                self.query.clear();
                self.found = None;
                self.map.hits = None;
                self.narrow();
                self.status = "the search let go".to_string();
                return true;
            }
            Act::ZoomIn if self.map.tree.is_some() => {
                self.map.tree_in(jiff::Timestamp::now().as_second());
                return true;
            }
            // From all of it, z goes into the cluster picked, as ⏎ does.
            Act::ZoomIn => {
                self.open_picked_cluster();
                return true;
            }
            Act::ZoomOut if self.map.tree.is_some() => {
                self.map.tree_out();
                return true;
            }
            Act::ZoomOut => {
                self.status = "all of it: every cluster, as far out as the map goes".to_string();
                return true;
            }
            Act::ZoomAll => {
                self.map.zoom_all();
                return true;
            }
            // From all of it, the block of the cluster picked is gone into.
            Act::Open if self.map.tree.is_none() => {
                self.open_picked_cluster();
                return true;
            }
            // In a cluster's tree, ⏎ goes into the branch picked, or, a
            // message, reads it.
            Act::Open if self.map.tree.is_some() => {
                if !self.map.tree_in(jiff::Timestamp::now().as_second()) {
                    self.read_dot();
                }
                return true;
            }
            Act::NextMessage | Act::PreviousMessage if !self.query.is_empty() => {
                let by = if act == Act::NextMessage { 1 } else { -1 };
                let now = jiff::Timestamp::now().as_second();
                self.map.next_found(&self.query.clone(), by, now);
                if self.side_by_side {
                    self.due = Some((Instant::now(), Due::Message));
                }
                return true;
            }
            Act::Back if self.map.suggesting.is_some() && self.map.region.is_none() => {
                self.map.suggesting = None;
                return true;
            }
            // In a cluster gone into, esc goes out a level.
            Act::Back if self.map.tree.is_some() && self.map.region.is_none() => {
                self.map.tree_out();
                return true;
            }
            _ => {}
        }
        if self.map.region.is_some() {
            match act {
                Act::Seen => return self.region_flag(Flag::Seen),
                Act::Flag => return self.region_flag(Flag::Flagged),
                Act::Trash => return self.ask_bulk(BulkAct::Trash),
                Act::Archive => return self.ask_bulk(BulkAct::Archive),
                Act::Back => {
                    self.map.region = None;
                    self.status = "let go".to_string();
                    return true;
                }
                _ => {}
            }
        }
        match act {
            Act::Down => self.map_step(0, 1),
            Act::Up => self.map_step(0, -1),
            Act::Left => self.map_step(-1, 0),
            Act::Right => self.map_step(1, 0),
            Act::Open => self.read_dot(),
            Act::Seen => self.dot_flag(Flag::Seen),
            Act::Flag => self.dot_flag(Flag::Flagged),
            Act::Trash => self.delete_dot(),
            Act::Archive => {
                if let Some(dot) = self.dot() {
                    self.mark(vec![dot.key]);
                    self.region_archive(true);
                }
            }
            Act::Back if self.map.focused != Focused::All => self.map.focused = Focused::All,
            Act::Back
            | Act::Top
            | Act::End
            | Act::PageDown
            | Act::PageUp
            | Act::ScreenDown
            | Act::ScreenUp
            | Act::NextMessage
            | Act::PreviousMessage => {}
            _ => return false,
        }
        true
    }

    /// A key in the map's view menu: what it shows changed, the menu kept
    /// open to change more; or the map laid out again; any other key
    /// closes it.
    pub(super) fn view_key(&mut self, key: KeyEvent) {
        // M-l, C-n and the like are not the menu's: it closes, and they do
        // what they do.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            self.dialog = Dialog::Closed;
            return self.key(key);
        }
        let row = match key.code {
            KeyCode::Char(c) => keymap::VIEW.iter().find(|v| v.0 == c),
            _ => None,
        };
        match row {
            Some(&(_, act, _)) => self.view(act),
            None => self.dialog = Dialog::Closed,
        }
    }

    /// Change what the map shows, or how, as `act` says.
    pub(super) fn view(&mut self, act: Act) {
        self.status = match act {
            Act::Lens => {
                self.map.lens = match self.map.lens {
                    crate::index::Lens::Both => crate::index::Lens::Words,
                    crate::index::Lens::Words => crate::index::Lens::People,
                    crate::index::Lens::People => crate::index::Lens::Both,
                };
                format!("the map laid out by {}", self.map.lens.name())
            }
            Act::Since => {
                self.map.since = self.map.since.next();
                format!("the map of {}", self.map.since.name())
            }
            Act::ShowRead => {
                self.map.hide_read = !self.map.hide_read;
                if self.map.hide_read {
                    "read mail hidden"
                } else {
                    "read mail shown"
                }
                .to_string()
            }
            Act::ShowSent => {
                self.map.hide_sent = !self.map.hide_sent;
                if self.map.hide_sent {
                    "sent mail hidden"
                } else {
                    "sent mail shown"
                }
                .to_string()
            }
            Act::Unfiled => {
                self.map.unfiled_only = !self.map.unfiled_only;
                if self.map.unfiled_only {
                    "only the mail not yet filed under a label"
                } else {
                    "filed mail shown too"
                }
                .to_string()
            }
            Act::NewOnly => {
                self.map.new_only = !self.map.new_only;
                if self.map.new_only {
                    "areas with nothing new dimmed"
                } else {
                    "every area as bright as it is"
                }
                .to_string()
            }
            Act::Relayout => {
                self.dialog = Dialog::Closed;
                return self.relayout();
            }
            _ => return,
        };
    }

    /// Move the map's cursor that way, and read what it picks once moving
    /// pauses, where the message read is beside the map.
    pub(super) fn map_step(&mut self, dx: i32, dy: i32) {
        let now = jiff::Timestamp::now().as_second();
        let parts = matches!(
            self.map.level(now),
            Some((_, _, crate::forest::Level::Mosaic(_)))
        );
        if parts {
            self.map.step_part(dx, dy, now);
        } else if self.map.tree.is_some() {
            match dx.signum() {
                1 => {
                    self.map.tree_in(now);
                }
                -1 => self.map.tree_out(),
                _ => self.map.tree_along(dy as isize, now),
            }
        } else {
            self.map.step_block(dx, dy);
        }
        if self.side_by_side {
            self.due = Some((Instant::now(), Due::Message));
        }
    }

    /// Begin the map, as was agreed: its key made and its index opened, and
    /// each account's sync set going; then show it. What stops it is said.
    pub(super) fn start_map(&mut self) {
        self.dialog = Dialog::Closed;
        self.map_later = false;
        let index = match (self.open_map)() {
            Ok(index) => index,
            Err(why) => {
                self.status = format!("no map: {why}");
                self.map_off = Some(why);
                return;
            }
        };
        *self.index.lock().unwrap_or_else(|e| e.into_inner()) = index;
        for a in 0..self.twins.len() {
            if let Some(twin) = self.twins[a].take() {
                self.syncs[a] = Some(sync::spawn(
                    a,
                    twin,
                    self.index.clone(),
                    self.replies_to.clone(),
                ));
                self.map_pass(a);
            }
        }
        self.show_map();
    }

    /// Open as the mail was last looked at: the map, from as far as its
    /// clusters if they were shown, else the list, and the message read as
    /// wide as it was made. Never looked at, the list, and the map, when it
    /// is, from its clusters.
    pub fn recall_look(&mut self) {
        let Some(said) = self
            .look_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
        else {
            return;
        };
        let mut lines = said.lines();
        let look = lines.next().unwrap_or_default();
        for line in lines {
            if let Some(w) = line.strip_prefix("reader ") {
                self.reader_width = w.trim().parse().ok();
            }
            if let Some(dir) = line.strip_prefix("save ") {
                self.save_dir = Some(PathBuf::from(dir));
            }
        }
        if look.split_whitespace().next() != Some("map") || self.map_off.is_some() || self.map_later
        {
            return;
        }
        self.show_map();
    }

    /// Write down how the mail is looked at, to open so again: the list or
    /// the map, how wide the message read was made, if it was, and where
    /// its files were last saved.
    pub fn remember_look(&self) {
        let Some(path) = &self.look_file else {
            return;
        };
        let look = match self.map.mode {
            Mode::Map => "map",
            Mode::List => "list",
        };
        let wide = self
            .reader_width
            .map_or(String::new(), |w| format!("reader {w}\n"));
        // A folder named across lines is not kept: the line after it would
        // be read as more.
        let save = self
            .save_dir
            .as_ref()
            .and_then(|d| d.to_str())
            .filter(|d| !d.contains('\n'))
            .map_or(String::new(), |d| format!("save {d}\n"));
        // Only how it was left; nothing lost if it cannot be kept.
        let _ = std::fs::write(path, format!("{look}\n{wide}{save}"));
    }

    /// Show the map in the middle pane, mail new since it was last looked
    /// at marked.
    pub(super) fn show_map(&mut self) {
        if let Some(why) = &self.map_off {
            self.status = format!("no map: {why}");
            return;
        }
        if self.map_later {
            self.dialog = Dialog::StartMap;
            return;
        }
        if self.map.mode != Mode::Map {
            let looked = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keep
                .looked;
            self.map.looked = (looked != 0).then_some(looked);
        }
        self.map.mode = Mode::Map;
        self.searching = false;
        self.focus = Focus::List;
        self.map.pick_first(jiff::Timestamp::now().as_second());
        self.map_due();
    }

    /// Show the list again; the map was looked at until now.
    pub(super) fn show_list(&mut self) {
        if self.map.mode == Mode::Map {
            self.looked_now();
            // What the map was narrowed to is what is listed.
            let narrowed = match self.map.focused {
                Focused::Cluster(id) => Some(Row::Cluster(id)),
                Focused::Loners => Some(Row::Loners),
                Focused::All => None,
            };
            if let Some(i) = narrowed.and_then(|n| self.rows.iter().position(|r| *r == n)) {
                self.row = i;
            }
        }
        // A move or a mark begun on the map is the map's: let go; one made
        // in the list, kept.
        if self.map.mode == Mode::Map {
            let moving = self.map.moving.take().is_some();
            let marked = self.map.region.take().is_some();
            if moving || marked {
                self.status = "what was moving or marked on the map, let go".to_string();
            }
        }
        if let Some(r) = &mut self.review {
            r.relabel = None;
            // The Inbox looked through is the map's: back to the list.
            if self.focus == Focus::Reader {
                self.focus = Focus::List;
            }
        }
        self.map.mode = Mode::List;
        if self.focus == Focus::Reader && !self.side_by_side {
            self.focus = Focus::List;
        }
        if matches!(
            self.rows.get(self.row),
            Some(Row::Map | Row::Cluster(_) | Row::Loners)
        ) {
            self.open_folder();
        }
    }

    /// Write down that the map was looked at now: what comes after is new.
    pub(super) fn looked_now(&mut self) {
        let at = jiff::Timestamp::now().as_second();
        let kept = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep(vec![crate::index::Kept::Looked { at }]);
        if let Err(e) = kept {
            self.status = format!("the map cannot keep when it was looked at: {e:#}");
        }
        self.map.looked = None;
    }

    /// The message read let go once it is out of what is shown, beside
    /// the list or the map it was not read from: in the list, once it is
    /// not listed; on the map, in a cluster gone into, once it is not in
    /// the part or the threads drawn.
    pub fn keep_reading_in_view(&mut self) {
        let Some(r) = &self.reading else {
            return;
        };
        let (account, folder, uid) = (r.account, r.folder.clone(), r.uid);
        let kept = match self.map.mode {
            Mode::List => !r.on_map && self.shown.iter().any(|l| l.is(account, &folder, uid)),
            Mode::Map => {
                r.on_map && (self.map.tree.is_none() || self.in_tree(account, &folder, uid))
            }
        };
        if !kept {
            self.reading = None;
            if self.focus == Focus::Reader {
                self.focus = Focus::List;
            }
        }
    }

    /// Whether the message `uid` of `folder` is in the part or the threads
    /// drawn of the cluster gone into; else, it cannot be told, and it is.
    fn in_tree(&mut self, account: usize, folder: &str, uid: u32) -> bool {
        let now = jiff::Timestamp::now().as_second();
        let Some((root, _, level)) = self.map.level(now) else {
            return true;
        };
        let (crate::forest::Level::Mosaic(path) | crate::forest::Level::Tree(path)) = &level;
        let Some(shown) = root.at(path) else {
            return true;
        };
        let key = self.accounts.get(account).and_then(|a| {
            self.index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .key_at(&a.ident, folder, uid)
        });
        let s = self.map.snapshot.as_ref();
        key.and_then(|k| s?.find(&k))
            .is_some_and(|i| shown.messages.contains(&i))
    }

    /// The message picked on the map.
    pub(super) fn dot(&self) -> Option<map::Dot> {
        let s = self.map.snapshot.as_ref()?;
        Some(s.dots[self.map.picked()?].clone())
    }

    /// Read the message picked on the map.
    pub(super) fn read_dot(&mut self) {
        let Some(dot) = self.dot() else {
            self.status = "no message picked: the arrows or hjkl pick one".to_string();
            return;
        };
        let Some((a, folder, uid)) = self.place_of(&dot) else {
            self.status = "that message is on no account here".to_string();
            return;
        };
        self.due = None;
        self.scroll = 0;
        let mark = self.mark_read && !dot.flags.seen;
        let body = match self.read.get(&(a, folder.clone(), uid)) {
            Some(m) => {
                if mark {
                    self.marked(a, &folder, uid, Flag::Seen, true);
                    self.ask(
                        a,
                        Ask::Set {
                            folder: folder.clone(),
                            uid,
                            flag: Flag::Seen,
                            on: true,
                        },
                    );
                }
                Body::Ready(m)
            }
            None => {
                self.ask(
                    a,
                    Ask::Open {
                        folder: folder.clone(),
                        uid,
                        mark,
                    },
                );
                Body::Loading
            }
        };
        self.reading = Some(Reading {
            account: a,
            folder,
            uid,
            body,
            on_map: true,
        });
        if !self.side_by_side {
            self.focus = Focus::Reader;
        }
    }

    /// Mark the message picked on the map, or not, as `flag` says.
    pub(super) fn dot_flag(&mut self, flag: Flag) {
        let Some(dot) = self.dot() else {
            return;
        };
        let Some((a, folder, uid)) = self.place_of(&dot) else {
            return;
        };
        let on = match flag {
            Flag::Seen => !dot.flags.seen,
            Flag::Flagged => !dot.flags.flagged,
            Flag::Answered => !dot.flags.answered,
        };
        self.marked(a, &folder, uid, flag, on);
        self.ask(
            a,
            Ask::Set {
                folder,
                uid,
                flag,
                on,
            },
        );
    }

    /// Move the message picked on the map to Trash.
    pub(super) fn delete_dot(&mut self) {
        let Some(dot) = self.dot() else {
            return;
        };
        let Some(listed) = self.listed_dot(&dot) else {
            return;
        };
        let (a, folder, summary) = (listed.account, listed.folder, listed.summary);
        match self.trash_of(a) {
            Some(trash) => self.move_to_trash(a, folder, summary.uid, summary.message_id, trash),
            None => self.ask_which_trash(a, folder, summary),
        }
    }

    /// Show a message marked, or not, on the map too: in the index, as its
    /// server will say at the next pass.
    pub(super) fn index_flag(
        &mut self,
        account: usize,
        folder: &str,
        uid: u32,
        flag: Flag,
        on: bool,
    ) {
        let Some(ident) = self.accounts.get(account).map(|a| a.ident.clone()) else {
            return;
        };
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let found = index
            .records
            .values()
            .find(|r| r.key.account == ident && r.place(folder).is_some_and(|p| p.uid == uid))
            .map(|r| (r.key.clone(), r.flags));
        if let Some((key, mut flags)) = found {
            match flag {
                Flag::Seen => flags.seen = on,
                Flag::Flagged => flags.flagged = on,
                Flag::Answered => flags.answered = on,
            }
            let _ = index.cache(vec![crate::index::Cached::Flags { key, flags }]);
        }
    }

    /// Make the map's snapshot again, off this thread, if the index has
    /// changed since, or the map is to be laid out and never was.
    pub(super) fn map_due(&mut self) {
        if self.map.computing || self.map_off.is_some() || self.map_later {
            return;
        }
        let layout = self.map.mode == Mode::Map;
        let shared = self.index.clone();
        let index = shared.lock().unwrap_or_else(|e| e.into_inner());
        let stale = match &self.map.snapshot {
            None => layout || !index.records.is_empty(),
            Some(s) => {
                s.version != index.version || s.lens != self.map.lens || (layout && s.at.is_none())
            }
        };
        if !stale {
            return;
        }
        let input = map::input(
            &index,
            self.map.lens,
            self.map.snapshot.as_deref(),
            self.map_corpus.take(),
            layout,
        );
        drop(index);
        self.map.computing = true;
        self.pending += 1;
        let replies = self.replies_to.clone();
        std::thread::spawn(move || {
            let snapshot = map::compute(input);
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Drawn(Box::new(snapshot)),
            });
        });
    }

    /// Take the map's new snapshot: keep a layout made for it, keep the
    /// message picked if it is still there, and list again what the map
    /// lists.
    pub(super) fn drawn_map(&mut self, mut snapshot: map::Snapshot) {
        self.map.computing = false;
        self.map_corpus = snapshot.corpus.take();
        if let Some(fresh) = &snapshot.fresh {
            let kept = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keep(vec![crate::index::Kept::Layout {
                    lens: snapshot.lens,
                    at: fresh.clone(),
                }]);
            if let Err(e) = kept {
                self.status = format!("the map cannot keep its layout: {e:#}");
            }
        }
        if self
            .map
            .cursor
            .as_ref()
            .is_some_and(|k| snapshot.find(k).is_none())
        {
            self.map.cursor = None;
        }
        if let Focused::Cluster(id) = self.map.focused
            && snapshot.group(id).is_none()
        {
            self.map.focused = Focused::All;
        }
        // A layout shown to be kept or dropped follows the map, by key.
        if let (Some(p), Some(old)) = (self.map.preview.take(), self.map.snapshot.clone()) {
            self.map.preview = (old.lens == snapshot.lens)
                .then(|| map::onto(&snapshot, &old.model.keys, &p))
                .flatten();
        }
        self.map.snapshot = Some(Arc::new(snapshot));
        if self.map.mode == Mode::Map {
            self.map.pick_first(jiff::Timestamp::now().as_second());
        }
        self.rebuild();
        if self.listing.as_ref().is_some_and(|l| {
            matches!(
                l.view,
                View::Map | View::Cluster(_) | View::Loners | View::Found(..)
            )
        }) {
            self.list_map();
            self.narrow();
        }
        // What the index holds changed: new mail may be sure to go.
        self.throw_away_on_its_own();
        self.map_due();
    }

    /// A click on a branch of a cluster's tree picks it; on the one
    /// picked, goes into it, or reads it; on the cluster, goes back to
    /// the clusters.
    pub(super) fn branch_click(&mut self, row: usize) {
        self.focus = Focus::List;
        let Some(path) = self.map.branches.get(row).cloned() else {
            return;
        };
        // The cluster itself: back to the clusters.
        if path.is_empty() {
            return self.map.close_tree();
        }
        let now = jiff::Timestamp::now().as_second();
        let picked = self.map.tree_picked(now).map(|(_, p)| p);
        if picked.as_ref() == Some(&path) {
            self.map_act(Act::Open);
        } else {
            self.map.pick_branch(&path, now);
            if self.side_by_side {
                self.due = Some((Instant::now(), Due::Message));
            }
        }
    }

    /// Go into the block of the cluster the message picked is in.
    pub(super) fn open_picked_cluster(&mut self) {
        let id = self
            .map
            .picked()
            .and_then(|i| self.map.snapshot.as_ref()?.group_of(i).map(|g| g.id));
        if let Some(id) = id {
            self.map.open_block(id);
        }
    }

    /// The wheel on the map, over `at`: in the threads of a part, along
    /// them; else up, into the block under it, and down, out a level, as
    /// z and Z. A swipe says the same many times: a level each while it
    /// goes on is enough.
    pub(super) fn map_wheel(&mut self, at: Target, by: isize) {
        let now = jiff::Timestamp::now().as_second();
        if matches!(
            self.map.level(now),
            Some((_, _, crate::forest::Level::Tree(_)))
        ) {
            return self.map_step(0, by.signum() as i32);
        }
        if self
            .wheeled
            .is_some_and(|(when, was)| was == by.signum() && when.elapsed() < WHEEL_SWIPE)
        {
            return;
        }
        self.wheeled = Some((Instant::now(), by.signum()));
        self.focus = Focus::List;
        if by > 0 {
            if self.map.tree.is_some() {
                self.map.tree_out();
            }
            return;
        }
        match at {
            Target::Anchor(id) => self.map.open_block(id),
            Target::Branch(row) => {
                if let Some(path) = self.map.branches.get(row).cloned() {
                    self.map.pick_branch(&path, now);
                    self.map.tree_in(now);
                }
            }
            _ => {}
        }
    }

    pub(super) fn anchor_click(&mut self, id: u32) {
        self.focus = Focus::List;
        if self.map.moving.is_some() {
            return self.move_onto(id);
        }
        self.map.open_block(id);
    }
}
