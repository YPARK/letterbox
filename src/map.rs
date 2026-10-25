//! The map: every message of every account laid out in the middle pane,
//! near what it is about and who it is from, in clusters, coloured by how
//! they are related. What is drawn is a snapshot of the index, made off
//! the screen's thread and made again as the index changes: clusters
//! follow each change, and the layout moves only when asked to.

use crate::index::labels::System;
use crate::index::record::Place;
use crate::index::{Index, Key, Lens, Record};
use crate::mail::Flags;
use crate::model::{self, Clustering, Model, Point, Vector};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The seed every layout and clustering is made with.
pub const SEED: u64 = 1;
/// How old read mail is to be drawn faintly, in seconds.
const OLD: i64 = 30 * 24 * 60 * 60;
/// How long ago new mail is taken to have come, the first time the map
/// is looked at.
const FIRST_LOOK: i64 = 24 * 60 * 60;

/// What the middle pane shows: the list, as it was, or the map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    List,
    Map,
}

/// How far back the map goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Since {
    #[default]
    All,
    Week,
    Today,
}

impl Since {
    pub fn next(self) -> Since {
        match self {
            Since::All => Since::Week,
            Since::Week => Since::Today,
            Since::Today => Since::All,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Since::All => "all of it",
            Since::Week => "this week",
            Since::Today => "today",
        }
    }

    /// The oldest a message may be dated and be shown, at `now`.
    fn from(self, now: i64) -> i64 {
        match self {
            Since::All => i64::MIN,
            Since::Week => now - 7 * 24 * 60 * 60,
            Since::Today => now - 24 * 60 * 60,
        }
    }
}

/// What a message's colour says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Colour {
    /// What it is about: the hue of its angle.
    #[default]
    Angle,
    /// How near the message picked it is, by angle.
    Cursor,
    /// Whether it is unread, flagged, or read.
    State,
}

impl Colour {
    pub fn next(self) -> Colour {
        match self {
            Colour::Angle => Colour::Cursor,
            Colour::Cursor => Colour::State,
            Colour::State => Colour::Angle,
        }
    }

    /// What the line under the map says of it.
    pub fn explain(self) -> &'static str {
        match self {
            Colour::Angle => "colour is what it is about: near hues, near subjects",
            Colour::Cursor => "colour is how near the message picked: strong is near, faint is far",
            Colour::State => "colour is its state: unread, flagged, or read",
        }
    }
}

/// What the map is narrowed to: all of it, a cluster, or the messages
/// that fit nowhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focused {
    #[default]
    All,
    Cluster(u32),
    Loners,
}

/// A message, as the map draws it.
#[derive(Clone, Debug)]
pub struct Dot {
    pub key: Key,
    pub from: String,
    pub address: String,
    pub subject: String,
    pub preview: String,
    pub message_id: String,
    pub date: Option<i64>,
    /// When it came: its date, or when it was first seen if that was
    /// earlier, as for mail read in since.
    pub arrived: i64,
    pub size: u32,
    pub flags: Flags,
    pub labels: Vec<String>,
    /// Whether it has a label letterbox gave it, or one waiting on its
    /// server.
    pub mine: bool,
    pub system: System,
    pub places: Vec<Place>,
    /// Whether another account has it too, by its `Message-ID`.
    pub twin: bool,
}

/// A cluster as the map and the sidebar show it.
#[derive(Clone, Debug)]
pub struct Group {
    pub id: u32,
    pub members: Vec<usize>,
    pub label: Option<String>,
    pub words: Vec<String>,
    pub hue: f32,
    pub unread: usize,
}

impl Group {
    /// What it is called: its label, else its words.
    pub fn name(&self) -> String {
        match &self.label {
            Some(l) => l.clone(),
            None if self.words.is_empty() => format!("cluster {}", self.id),
            None => self.words.join(" "),
        }
    }
}

/// What the map is drawn from: the index as it was, by one lens.
pub struct Snapshot {
    pub version: u64,
    pub lens: Lens,
    pub dots: Vec<Dot>,
    pub model: Model,
    pub clustering: Clustering,
    pub groups: Vec<Group>,
    /// Each message's hue, in degrees.
    pub hues: Vec<f32>,
    /// Where each message is, once laid out.
    pub at: Option<Vec<Point>>,
    pub faithfulness: Option<f32>,
    /// A layout made for it, to keep.
    pub fresh: Option<Vec<(Key, f32, f32)>>,
    /// Each message and label, folded, it was said not to be.
    pub not_it: std::collections::BTreeSet<(Key, String)>,
}

impl Snapshot {
    /// The message known by `key`, by its place.
    pub fn find(&self, key: &Key) -> Option<usize> {
        self.model.keys.binary_search(key).ok()
    }

    /// The group the message `i` is in.
    pub fn group_of(&self, i: usize) -> Option<&Group> {
        self.groups.get(self.clustering.of.get(i).copied()?)
    }

    pub fn group(&self, id: u32) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The messages that fit nowhere.
    pub fn loners(&self) -> Vec<usize> {
        (0..self.dots.len())
            .filter(|&i| self.clustering.loner[i])
            .collect()
    }
}

/// What a snapshot is made from, taken from the index under its lock.
pub struct Input {
    version: u64,
    lens: Lens,
    records: Vec<(Record, Vec<String>, bool, System)>,
    not_it: std::collections::BTreeSet<(Key, String)>,
    layout: Option<BTreeMap<Key, (f32, f32)>>,
    before: Vec<(u32, Vector, Vec<Key>)>,
    layout_wanted: bool,
}

/// What a snapshot of `index` by `lens` is made from; begun from the
/// clusters of `last`, and laid out if `layout` and it never was.
pub fn input(index: &Index, lens: Lens, last: Option<&Snapshot>, layout: bool) -> Input {
    let records = index
        .records
        .values()
        .map(|r| {
            let mine = index.keep.labels.contains_key(&r.key)
                || index
                    .keep
                    .pending
                    .range((r.key.clone(), String::new())..)
                    .take_while(|((k, _), _)| *k == r.key)
                    .any(|(_, w)| w.on);
            (r.clone(), index.labels(&r.key), mine, index.system(&r.key))
        })
        .collect();
    let before = last
        .filter(|s| s.lens == lens)
        .map(|s| {
            s.clustering
                .clusters
                .iter()
                .map(|c| {
                    let keys = c.members.iter().map(|&i| s.model.keys[i].clone()).collect();
                    (c.id, c.centre, keys)
                })
                .collect()
        })
        .unwrap_or_default();
    Input {
        version: index.version,
        lens,
        records,
        not_it: index.keep.not_it.clone(),
        layout: index.keep.layouts.get(&lens).cloned(),
        before,
        layout_wanted: layout,
    }
}

/// The snapshot `input` makes: the model, its clusters and hues, and where
/// each message is, by the layout kept, new mail placed among its
/// nearest; or by a layout made now, if one is wanted and none was kept.
pub fn compute(input: Input) -> Snapshot {
    let given: Vec<model::Given> = input
        .records
        .iter()
        .map(|(r, labels, _, _)| (r, labels.clone()))
        .collect();
    let model = Model::build(&given, &input.not_it, input.lens);
    let clustering = model::cluster(&model, &input.before, SEED);
    let hues = model::hues(&model.vectors);
    let mut ids: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (r, _, _, _) in &input.records {
        if !r.message_id.is_empty() {
            ids.entry(r.message_id.as_str())
                .or_default()
                .push(r.key.account.as_str());
        }
    }
    let dots: Vec<Dot> = input
        .records
        .iter()
        .map(|(r, labels, mine, system)| {
            let twin = ids
                .get(r.message_id.as_str())
                .is_some_and(|a| a.iter().any(|x| *x != r.key.account));
            Dot {
                key: r.key.clone(),
                from: r.from.clone(),
                address: r.address.clone(),
                subject: r.subject.clone(),
                preview: r.preview.clone(),
                message_id: r.message_id.clone(),
                date: r.date,
                arrived: r.date.unwrap_or(r.first_seen).min(r.first_seen),
                size: r.size,
                flags: r.flags,
                labels: labels.clone(),
                mine: *mine,
                system: *system,
                places: r.places.clone(),
                twin,
            }
        })
        .collect();
    let groups = clustering
        .clusters
        .iter()
        .map(|c| Group {
            id: c.id,
            members: c.members.clone(),
            label: c.label.clone(),
            words: c.words.clone(),
            hue: model::hues(&[c.centre]).first().copied().unwrap_or(0.0),
            unread: c.members.iter().filter(|&&i| !dots[i].flags.seen).count(),
        })
        .collect::<Vec<_>>();
    // A cluster's hue is its members' mean, by angle on the hue circle.
    let groups = groups
        .into_iter()
        .map(|mut g| {
            let (mut x, mut y) = (0f32, 0f32);
            for &i in &g.members {
                x += hues[i].to_radians().cos();
                y += hues[i].to_radians().sin();
            }
            g.hue = y.atan2(x).to_degrees().rem_euclid(360.0);
            g
        })
        .collect();
    let (at, fresh) = match &input.layout {
        Some(kept) => {
            let known: Vec<(usize, Point)> = model
                .keys
                .iter()
                .enumerate()
                .filter_map(|(i, k)| kept.get(k).map(|&(x, y)| (i, [x, y])))
                .collect();
            let mut at: Vec<Option<Point>> = vec![None; model.len()];
            for &(i, p) in &known {
                at[i] = Some(p);
            }
            let at: Vec<Point> = (0..model.len())
                .map(|i| at[i].unwrap_or_else(|| model::place(&model, i, &known)))
                .collect();
            (Some(at), None)
        }
        None if input.layout_wanted && model.len() > 0 => {
            let at = model::layout(&model, None, SEED);
            let fresh = model
                .keys
                .iter()
                .zip(&at)
                .map(|(k, p)| (k.clone(), p[0], p[1]))
                .collect();
            (Some(at), Some(fresh))
        }
        None => (None, None),
    };
    let faithfulness = at.as_ref().map(|a| model::faithfulness(&model, a));
    Snapshot {
        version: input.version,
        lens: input.lens,
        dots,
        model,
        clustering,
        groups,
        hues,
        at,
        faithfulness,
        fresh,
        not_it: input.not_it,
    }
}

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
    pub colour: Colour,
    pub hide_read: bool,
    /// Whether areas with nothing new are dimmed.
    pub new_only: bool,
    /// When the map was looked at before this look, in seconds since 1970.
    pub looked: Option<i64>,
    /// Whether a snapshot is being made.
    pub computing: bool,
    /// Where each message was drawn, by its place, as cells of the screen.
    pub cells: BTreeMap<usize, (u16, u16)>,
    /// What each cell drawn holds, by its place.
    pub drawn: BTreeMap<(u16, u16), Vec<usize>>,
    /// The anchors drawn, where they are, by their group's number.
    pub anchors: Vec<(u16, u16, u16, u32)>,
    /// The cell last clicked, and which of its messages it picked.
    pub clicked: Option<((u16, u16), usize)>,
    /// What is marked, to act on all of it.
    pub region: Option<Region>,
    /// A box being drawn with the mouse: where it was begun, and where the
    /// mouse is now.
    pub drag: Option<((u16, u16), (u16, u16))>,
    /// What is being moved onto a cluster or label, while it is.
    pub moving: Option<Vec<Key>>,
    /// The label last filed under, whose suggestions are shown.
    pub suggesting: Option<String>,
}

/// What is marked on the map: the messages of a cluster, or those in a
/// box drawn, and the box, by its corners, if it was drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Region {
    pub keys: Vec<Key>,
    pub boxed: Option<((u16, u16), (u16, u16))>,
}

impl Map {
    /// When mail is new since: the map's last look, or a day ago.
    pub fn since_look(&self, now: i64) -> i64 {
        self.looked.unwrap_or(now - FIRST_LOOK)
    }

    /// Whether the message `i` came since the map was last looked at and
    /// is still in the Inbox.
    pub fn is_new(&self, dot: &Dot, now: i64) -> bool {
        dot.system.inbox && dot.arrived > self.since_look(now)
    }

    /// Whether read mail more than a month old is the message `i`.
    pub fn is_old(dot: &Dot, now: i64) -> bool {
        dot.flags.seen && dot.date.is_some_and(|d| now - d > OLD)
    }

    /// The messages drawn: all, but the read ones if they are hidden, and
    /// those older than how far back the map goes.
    pub fn shown(&self, now: i64) -> Vec<usize> {
        let Some(s) = &self.snapshot else {
            return Vec::new();
        };
        let from = self.since.from(now);
        (0..s.dots.len())
            .filter(|&i| {
                let d = &s.dots[i];
                !(self.hide_read && d.flags.seen) && d.date.unwrap_or(d.arrived) >= from
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

    /// Whether the message `i` is in what the map is narrowed to.
    pub fn in_focus(&self, i: usize) -> bool {
        let Some(s) = &self.snapshot else {
            return true;
        };
        match self.focused {
            Focused::All => true,
            Focused::Cluster(id) => s.group_of(i).is_some_and(|g| g.id == id),
            Focused::Loners => s.clustering.loner.get(i).copied().unwrap_or(false),
        }
    }

    /// Move the cursor to the nearest message drawn that way, `dx` across
    /// and `dy` down; to the one nearest the middle if none is picked.
    pub fn step(&mut self, dx: i32, dy: i32) {
        let Some(s) = self.snapshot.clone() else {
            return;
        };
        let here = self.picked().and_then(|i| self.cells.get(&i).copied());
        let Some((x0, y0)) = here else {
            // None picked: the message nearest the middle of what is drawn.
            let n = self.cells.len().max(1) as f32;
            let (mx, my) = self.cells.values().fold((0.0, 0.0), |(a, b), &(x, y)| {
                (a + f32::from(x) / n, b + f32::from(y) / n)
            });
            let near = self
                .cells
                .iter()
                .min_by(|a, b| {
                    let d = |&(x, y): &(u16, u16)| {
                        (f32::from(x) - mx).powi(2) + (2.0 * (f32::from(y) - my)).powi(2)
                    };
                    d(a.1).total_cmp(&d(b.1)).then(a.0.cmp(b.0))
                })
                .map(|(&i, _)| i);
            self.cursor = near.map(|i| s.dots[i].key.clone());
            return;
        };
        let picked = self.picked();
        // A cell is about twice as tall as it is wide: rows count double.
        let best = self
            .cells
            .iter()
            .filter(|(i, _)| Some(**i) != picked)
            .filter_map(|(&i, &(x, y))| {
                let (ex, ey) = (
                    f32::from(x) - f32::from(x0),
                    2.0 * (f32::from(y) - f32::from(y0)),
                );
                let along = ex * dx as f32 + ey * dy as f32;
                let across = (ex * dy as f32 - ey * dx as f32).abs();
                let same = (x, y) == (x0, y0);
                (along > 0.0 || same && along >= 0.0).then_some((along + 2.0 * across, i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if let Some((_, i)) = best {
            self.cursor = Some(s.dots[i].key.clone());
        }
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
        let g = &s.groups[next];
        self.focused = Focused::Cluster(g.id);
        let centre = &s.clustering.clusters[next].centre;
        let first = g
            .members
            .iter()
            .max_by(|&&a, &&b| {
                model::dot(&s.model.vectors[a], centre)
                    .total_cmp(&model::dot(&s.model.vectors[b], centre))
                    .then(b.cmp(&a))
            })
            .copied();
        if let Some(i) = first {
            self.cursor = Some(s.dots[i].key.clone());
        }
    }
}
