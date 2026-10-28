//! The map as it is looked at: what is shown, the message picked, a
//! cluster gone into, and what is searched for.
use super::*;

/// How long ago new mail is taken to have come, the first time the map
/// is looked at.
pub(super) const FIRST_LOOK: i64 = 24 * 60 * 60;

/// The map as it is looked at.
#[derive(Default)]
pub struct Map {
    pub mode: Mode,
    pub snapshot: Option<Arc<Snapshot>>,
    /// The message picked.
    pub cursor: Option<Key>,
    pub focused: Focused,
    pub lens: Lens,
    pub since: Since,
    pub hide_read: bool,
    pub hide_sent: bool,
    /// Whether only the mail not yet filed under a label is shown.
    pub unfiled_only: bool,
    /// Whether areas with nothing new are dimmed.
    pub new_only: bool,
    /// When the map was looked at before this look, in seconds since 1970.
    pub looked: Option<i64>,
    /// Whether a snapshot is being made.
    pub computing: bool,
    /// What is marked, to act on all of it.
    pub region: Option<Region>,
    /// What is being moved onto a cluster or label, while it is.
    pub moving: Option<Vec<Key>>,
    /// The label last filed under, whose suggestions are shown.
    pub suggesting: Option<String>,
    /// The map laid out again, shown to be kept or dropped: where each of
    /// the snapshot's messages would be.
    pub preview: Option<Vec<Point>>,
    /// Whether it is being laid out again.
    pub relaying: bool,
    /// The messages that hold what is searched for, as last found.
    pub hits: Option<Hits>,
    /// The blocks drawn: a cluster's number, or a part's place, their
    /// middle, across and down in cells, and half their width, in columns.
    pub blocks: Vec<(u32, [f32; 2], f32)>,
    /// The mosaic last tiled, and what of: the snapshot, the messages
    /// shown and the canvas's size; tiled again only when one changes.
    pub mosaic: Option<(Tiled, Arc<Mosaic>)>,
    /// A cluster opened from its block, shown as its tree.
    pub tree: Option<Tree>,
    /// The centres each cluster's parts were split by, and how many it
    /// held then, by its number: its parts hold while it holds about as
    /// many.
    pub parts: BTreeMap<u32, (usize, Vec<Vector>)>,
    /// The way to each branch of the tree drawn, by its row.
    pub branches: Vec<Vec<usize>>,
}

/// What a mosaic was tiled from: the snapshot, by where it is, the
/// messages tiled, the canvas, across and down, of a cluster gone into,
/// the cluster and the way to the parts tiled, and the map laid out again,
/// by where it is, while it is shown.
pub type Tiled = (
    usize,
    Vec<usize>,
    u16,
    u16,
    Option<(u32, Vec<usize>)>,
    Option<usize>,
);

/// A cluster looked at as its tree.
#[derive(Clone, Debug)]
pub struct Tree {
    pub id: u32,
    /// How far down the branch picked is: one for a part of the cluster.
    pub depth: usize,
    /// The first row of it drawn.
    pub top: usize,
    /// It as last grown, of which snapshot and how many messages shown.
    pub(super) grown: Option<(usize, usize, Arc<Branch>)>,
}

/// The messages of a snapshot that hold every word searched for.
pub struct Hits {
    pub(super) query: String,
    /// The snapshot they were found in.
    pub(super) of: usize,
    /// Whether each message holds them, by its place.
    pub matched: Vec<bool>,
}

/// What is marked on the map: the messages of a cluster.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region {
    pub keys: Vec<Key>,
}

impl Map {
    /// All of the map: every cluster, out of any gone into.
    pub fn zoom_all(&mut self) {
        self.tree = None;
        self.focused = Focused::All;
    }

    /// Open the cluster `id` from its block, as its tree, the part picked
    /// that the message picked is in, if it is in it; else its newest's.
    pub fn open_block(&mut self, id: u32) {
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let Some(g) = s.group(id) else {
            return;
        };
        self.focused = Focused::Cluster(id);
        self.tree = Some(Tree {
            id,
            depth: 1,
            top: 0,
            grown: None,
        });
        if self.picked().is_some_and(|i| g.members.contains(&i)) {
            return;
        }
        let newest = g.members.iter().copied().max_by_key(|&i| {
            let d = &s.dots[i];
            (d.date.unwrap_or(d.arrived), i)
        });
        self.cursor = newest.map(|i| s.dots[i].key.clone());
    }

    /// Back from a cluster's tree to the clusters, its block among them.
    pub fn close_tree(&mut self) {
        self.tree = None;
        self.focused = Focused::All;
    }

    /// The tree of the cluster opened, grown again only when the snapshot
    /// or what of it is shown has changed; its parts as they were while
    /// it holds about as many.
    pub fn grown(&mut self, now: i64) -> Option<Arc<Branch>> {
        let s = self.snapshot.clone()?;
        let shown = self.shown(now);
        let of = Arc::as_ptr(&s) as usize;
        let id = self.tree.as_ref()?.id;
        if let Some((o, n, b)) = &self.tree.as_ref()?.grown
            && *o == of
            && *n == shown.len()
        {
            return Some(b.clone());
        }
        let held = s
            .group(id)?
            .members
            .iter()
            .filter(|i| shown.binary_search(i).is_ok())
            .count();
        let kept: Option<Vec<Vector>> = self
            .parts
            .get(&id)
            .filter(|(then, _)| {
                let (then, held) = (*then as f32, held as f32);
                held >= then * 0.8 && held <= then * 1.25
            })
            .map(|(_, c)| c.clone());
        let (branch, centres) = forest::grow(&s, id, &shown, kept.as_deref())?;
        if kept.is_none() {
            self.parts.insert(id, (held, centres));
        }
        let branch = Arc::new(branch);
        self.tree.as_mut()?.grown = Some((of, shown.len(), branch.clone()));
        Some(branch)
    }

    /// The tree, and the way to the branch picked in it: the one the
    /// message picked is under, as far down as it is looked at. The newest
    /// message is picked if none of it is.
    pub fn tree_picked(&mut self, now: i64) -> Option<(Arc<Branch>, Vec<usize>)> {
        // The message picked elsewhere, as by a search or the Inbox looked
        // through: its cluster gone into instead, never another picked.
        let s = self.snapshot.clone()?;
        let elsewhere = self
            .picked()
            .and_then(|i| s.group_of(i))
            .map(|g| g.id)
            .filter(|&id| self.tree.as_ref().is_some_and(|t| t.id != id));
        if let Some(id) = elsewhere {
            self.open_block(id);
        }
        let root = self.grown(now)?;
        let path = match self.picked().and_then(|i| root.path_to(i)) {
            Some(path) => path,
            None => {
                let i = root.newest()?;
                self.cursor = Some(self.snapshot.as_ref()?.dots[i].key.clone());
                root.path_to(i)?
            }
        };
        let t = self.tree.as_mut()?;
        t.depth = t.depth.clamp(1, path.len().max(1));
        Some((root.clone(), path[..t.depth.min(path.len())].to_vec()))
    }

    /// Pick the branch `by` after the one picked, as far down within the
    /// part shown as a tree, or before: the newest message under it.
    pub fn tree_along(&mut self, by: isize, now: i64) {
        let Some((root, path)) = self.tree_picked(now) else {
            return;
        };
        let part = match root.level(&path) {
            Level::Tree(part) => part,
            Level::Mosaic(_) => Vec::new(),
        };
        let Some(from) = root.at(&part) else {
            return;
        };
        let all: Vec<Vec<usize>> = from
            .at_depth(path.len() - part.len())
            .into_iter()
            .map(|p| [part.as_slice(), &p].concat())
            .collect();
        let at = all.iter().position(|p| *p == path).unwrap_or(0);
        let next = (at as isize + by).clamp(0, all.len() as isize - 1) as usize;
        if let Some(i) = all.get(next).and_then(|p| root.at(p)?.newest()) {
            self.cursor = self.snapshot.as_ref().map(|s| s.dots[i].key.clone());
        }
    }

    /// Whether a cluster's tree is gone down to a message: the one place on
    /// the map, besides reading it, where mail is acted on.
    pub fn at_message(&mut self, now: i64) -> bool {
        self.tree.is_some()
            && self
                .tree_picked(now)
                .is_some_and(|(root, path)| root.at(&path).is_some_and(|b| b.is_leaf()))
    }

    /// Go into the branch picked, to the one under it the message picked
    /// is under; whether there was one to go into.
    pub fn tree_in(&mut self, now: i64) -> bool {
        let Some((root, path)) = self.tree_picked(now) else {
            return false;
        };
        let inner = root.at(&path).is_some_and(|b| !b.is_leaf());
        if let Some(t) = self.tree.as_mut().filter(|_| inner) {
            t.depth = path.len() + 1;
        }
        inner
    }

    /// Go out of the branch picked to the one it is under; out of the
    /// cluster's parts, back to the clusters.
    pub fn tree_out(&mut self) {
        match &mut self.tree {
            Some(t) if t.depth > 1 => t.depth -= 1,
            _ => self.close_tree(),
        }
    }

    /// Pick the branch the way `path` goes to: the newest message under it.
    pub fn pick_branch(&mut self, path: &[usize], now: i64) {
        let Some(root) = self.grown(now) else {
            return;
        };
        let Some(i) = root.at(path).and_then(Branch::newest) else {
            return;
        };
        self.cursor = self.snapshot.as_ref().map(|s| s.dots[i].key.clone());
        if let Some(t) = &mut self.tree {
            t.depth = path.len().max(1);
        }
    }

    /// The tree of the cluster gone into, the way to the branch picked,
    /// and what is shown of it.
    pub fn level(&mut self, now: i64) -> Option<(Arc<Branch>, Vec<usize>, Level)> {
        let (root, path) = self.tree_picked(now)?;
        let level = root.level(&path);
        Some((root, path, level))
    }

    /// In a cluster's parts, pick the part nearest that way, `dx` across
    /// and `dy` down, among those drawn: the newest message under it.
    pub fn step_part(&mut self, dx: i32, dy: i32, now: i64) {
        let Some((_, path, Level::Mosaic(above))) = self.level(now) else {
            return;
        };
        let here = path.last().map(|&k| k as u32);
        if let Some(k) = self.nearest_block(here, dx, dy) {
            self.pick_branch(&[above.as_slice(), &[k as usize]].concat(), now);
        }
    }

    /// Pick the block nearest that way, `dx` across and `dy` down, from the
    /// one the message picked is in: its message nearest its middle.
    pub fn step_block(&mut self, dx: i32, dy: i32) {
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let here = self.picked().and_then(|i| s.group_of(i)).map(|g| g.id);
        let next = self.nearest_block(here, dx, dy);
        let at = next.and_then(|id| s.groups.iter().position(|g| g.id == id));
        if let Some(i) = at.and_then(|at| s.central(at)) {
            self.cursor = Some(s.dots[i].key.clone());
        }
    }

    /// The block drawn nearest that way, `dx` across and `dy` down, from
    /// the block `here`; the first, if none is.
    pub(super) fn nearest_block(&self, here: Option<u32>, dx: i32, dy: i32) -> Option<u32> {
        let here = here.and_then(|id| self.blocks.iter().find(|n| n.0 == id).copied());
        match here {
            None => self.blocks.first().map(|n| n.0),
            Some((id, [x0, y0], _)) => self
                .blocks
                .iter()
                .filter(|n| n.0 != id)
                .filter_map(|&(other, [x, y], _)| {
                    // A cell is about twice as tall as it is wide.
                    let (ex, ey) = (x - x0, 2.0 * (y - y0));
                    let along = ex * dx as f32 + ey * dy as f32;
                    let across = (ex * dy as f32 - ey * dx as f32).abs();
                    (along > 0.0).then_some((along + 2.0 * across, other))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, other)| other),
        }
    }

    /// Which messages hold every word of `query`, in their subject, sender,
    /// address, first lines or labels, case let go: none for no words.
    /// Found again only once the query or the snapshot changes.
    pub fn hits(&mut self, query: &str) -> Option<&Hits> {
        use rayon::prelude::*;
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let s = self.snapshot.clone()?;
        if words.is_empty() {
            self.hits = None;
            return None;
        }
        let of = Arc::as_ptr(&s) as usize;
        if !self
            .hits
            .as_ref()
            .is_some_and(|h| h.query == query && h.of == of)
        {
            let matched: Vec<bool> = s
                .dots
                .par_iter()
                .map(|d| {
                    let text = format!(
                        "{} {} {} {} {}",
                        d.from,
                        d.address,
                        d.subject,
                        d.preview,
                        d.labels.join(" ")
                    )
                    .to_lowercase();
                    words.iter().all(|w| text.contains(w.as_str()))
                })
                .collect();
            self.hits = Some(Hits {
                query: query.to_string(),
                of,
                matched,
            });
        }
        self.hits.as_ref()
    }

    /// The messages drawn that hold what is searched for, of the cluster
    /// the map is narrowed to if it is, newest first.
    pub fn found(&mut self, query: &str, now: i64) -> Vec<usize> {
        let shown = self.shown(now);
        let (Some(hits), Some(s)) = (
            self.hits(query).map(|h| h.matched.clone()),
            self.snapshot.clone(),
        ) else {
            return Vec::new();
        };
        let mut found: Vec<usize> = shown
            .into_iter()
            .filter(|&i| hits[i] && self.in_focus(&s, i))
            .collect();
        found.sort_by_key(|&i| (std::cmp::Reverse(s.dots[i].date), i));
        found
    }

    /// Whether the message `i` is of what the map is narrowed to.
    fn in_focus(&self, s: &Snapshot, i: usize) -> bool {
        match self.focused {
            Focused::All => true,
            Focused::Cluster(id) => s.picked_by(id, i),
            Focused::Loners => s.clustering.loner[i],
        }
    }

    /// What a search on the map is of, if not all of it: the cluster it is
    /// narrowed to, by name, or what fits nowhere.
    pub fn searched_in(&self) -> Option<String> {
        let s = self.snapshot.as_ref()?;
        match self.focused {
            Focused::All => None,
            Focused::Cluster(id) => s.group(id).map(|g| g.name()),
            Focused::Loners => Some("what fits nowhere".to_string()),
        }
    }

    /// Every message that holds every word of `query`, whatever the map
    /// shows: of every folder of every account it knows, newest first.
    pub fn found_all(&mut self, query: &str) -> Vec<usize> {
        let (Some(hits), Some(s)) = (
            self.hits(query).map(|h| h.matched.clone()),
            self.snapshot.clone(),
        ) else {
            return Vec::new();
        };
        let mut found: Vec<usize> = (0..s.dots.len()).filter(|&i| hits[i]).collect();
        found.sort_by_key(|&i| (std::cmp::Reverse(s.dots[i].date), i));
        found
    }

    /// Pick the newest message found, unless the one picked is found too;
    /// how many are.
    pub fn seek(&mut self, query: &str, now: i64) -> usize {
        let found = self.found(query, now);
        if !self.picked().is_some_and(|p| found.contains(&p))
            && let (Some(&first), Some(s)) = (found.first(), &self.snapshot)
        {
            self.cursor = Some(s.dots[first].key.clone());
        }
        found.len()
    }

    /// Pick the next message found, newest first, or the one before; round.
    pub fn next_found(&mut self, query: &str, by: isize, now: i64) {
        let found = self.found(query, now);
        if found.is_empty() {
            return;
        }
        let at = self
            .picked()
            .and_then(|p| found.iter().position(|&i| i == p));
        let n = found.len() as isize;
        let next = match at {
            Some(at) => (at as isize + by).rem_euclid(n),
            None => 0,
        } as usize;
        if let Some(s) = &self.snapshot {
            self.cursor = Some(s.dots[found[next]].key.clone());
        }
    }

    /// When mail is new since: the map's last look, or a day ago.
    pub fn since_look(&self, now: i64) -> i64 {
        self.looked.unwrap_or(now - FIRST_LOOK)
    }

    /// Whether the message `i` came since the map was last looked at and
    /// is still in the Inbox.
    pub fn is_new(&self, dot: &Dot, now: i64) -> bool {
        dot.system.inbox && dot.arrived > self.since_look(now)
    }

    /// The messages drawn: all, but the read ones if they are hidden, the
    /// sent ones if they are, those filed under a label, and the sent, if
    /// only what is not yet filed is, and those older than how far back the
    /// map goes.
    pub fn shown(&self, now: i64) -> Vec<usize> {
        let Some(s) = &self.snapshot else {
            return Vec::new();
        };
        let from = self.since.from(now);
        (0..s.dots.len())
            .filter(|&i| {
                let d = &s.dots[i];
                !(self.hide_read && d.flags.seen)
                    && !(self.hide_sent && d.system.sent)
                    && !(self.unfiled_only && (!d.labels.is_empty() || d.system.sent))
                    && d.date.unwrap_or(d.arrived) >= from
            })
            .collect()
    }

    /// Pick a message where none is: the newest of what is new, else the
    /// newest shown.
    pub fn pick_first(&mut self, now: i64) {
        if self.picked().is_some() {
            return;
        }
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let shown = self.shown(now);
        let newest = |only_new: bool| {
            shown
                .iter()
                .copied()
                .filter(|&i| !only_new || self.is_new(&s.dots[i], now))
                .max_by_key(|&i| (s.dots[i].date, std::cmp::Reverse(i)))
        };
        self.cursor = newest(true)
            .or_else(|| newest(false))
            .map(|i| s.dots[i].key.clone());
    }

    /// The message picked, by its place in the snapshot.
    pub fn picked(&self) -> Option<usize> {
        self.snapshot.as_ref()?.find(self.cursor.as_ref()?)
    }

    /// Narrow the map to the cluster after the one it is narrowed to, or
    /// before, and pick its message nearest the centre.
    pub fn cycle(&mut self, by: isize) {
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        if s.groups.is_empty() {
            return;
        }
        let at = match self.focused {
            Focused::Cluster(id) => s.groups.iter().position(|g| g.id == id),
            _ => None,
        };
        let n = s.groups.len() as isize;
        let next = match at {
            Some(a) => (a as isize + by).rem_euclid(n) as usize,
            None if by > 0 => 0,
            None => (n - 1) as usize,
        };
        self.focused = Focused::Cluster(s.groups[next].id);
        if let Some(i) = s.central(next) {
            self.cursor = Some(s.dots[i].key.clone());
        }
        // Looked at as a tree, the next cluster's tree.
        if self.tree.is_some() {
            self.open_block(s.groups[next].id);
        }
    }
}
