//! The map: every message of every account in the middle pane, in its
//! cluster, near what it is about and who it is from, the clusters a
//! mosaic, each gone into as a mosaic of its parts. What is drawn is a
//! snapshot of the index, made off the screen's thread and made again as
//! the index changes: clusters follow each change, and the layout, which
//! places each block by those it lies by, moves only when asked to.

use crate::forest::{self, Branch, Level};
use crate::index::labels::System;
use crate::index::record::Place;
use crate::index::{Index, Key, Lens, Record};
use crate::mail::Flags;
use crate::model::{self, Clustering, Corpus, Model, Point, Vector};
use crate::mosaic::Mosaic;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The seed every layout and clustering is made with.
pub const SEED: u64 = 1;
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
    /// The addresses it went to.
    pub to: Vec<String>,
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
    /// How likely it is to be thrown away, as learnt from what you threw
    /// away, if it is in an Inbox and that is sure enough to say.
    pub thrown: Option<f32>,
    /// Whether it was kept, as chosen, filed or flagged: looked at before.
    pub looked: bool,
    /// What it answers, by message IDs, and Gmail's thread of it.
    pub parents: Vec<String>,
    pub thread: Option<u64>,
}

/// A cluster as the map and the sidebar show it.
#[derive(Clone, Debug)]
pub struct Group {
    pub id: u32,
    pub members: Vec<usize>,
    /// Its largest part, where its anchor is: a label's mail may lie in
    /// several parts of the map.
    pub core: Vec<usize>,
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
    pub model: Arc<Model>,
    pub clustering: Clustering,
    /// The clusters, as shown: one a label, its parts together.
    pub groups: Vec<Group>,
    /// The group each message is in, by its place.
    pub group_at: Vec<usize>,
    /// Each message's hue, in degrees.
    pub hues: Vec<f32>,
    /// Where each message is, once laid out.
    pub at: Option<Vec<Point>>,
    pub faithfulness: Option<f32>,
    /// A layout made for it, to keep.
    pub fresh: Option<Vec<(Key, f32, f32)>>,
    /// Each message and label, folded, it was said not to be.
    pub not_it: std::collections::BTreeSet<(Key, String)>,
    /// Where each message filed under no label would go, as the filed mail
    /// nearest it votes, none turned down for it.
    pub voted: Vec<Option<String>>,
    /// What the model has read, to be handed to the next snapshot.
    pub corpus: Option<Corpus>,
}

impl Snapshot {
    /// The message known by `key`, by its place.
    pub fn find(&self, key: &Key) -> Option<usize> {
        self.model.place.get(key).copied()
    }

    /// The group the message `i` is in.
    pub fn group_of(&self, i: usize) -> Option<&Group> {
        self.groups.get(self.group_at.get(i).copied()?)
    }

    /// The messages of the group `id`, as it is picked: those clustered
    /// with it, and, a label's, every message that has the label, wherever
    /// it lies.
    pub fn picked_by(&self, id: u32, i: usize) -> bool {
        if self.group_of(i).is_some_and(|g| g.id == id) {
            return true;
        }
        match self.group(id) {
            Some(Group { label: Some(l), .. }) => {
                let l = crate::index::fold(l);
                self.dots[i]
                    .labels
                    .iter()
                    .any(|x| crate::index::fold(x) == l)
            }
            _ => false,
        }
    }

    /// The message of the group at `at` nearest its middle by angle.
    pub fn central(&self, at: usize) -> Option<usize> {
        let g = self.groups.get(at)?;
        let v = &self.model.vectors;
        let mut middle: model::Vector = [0.0; model::DIM];
        for &i in &g.members {
            for (m, x) in middle.iter_mut().zip(&v[i]) {
                *m += x;
            }
        }
        g.members
            .iter()
            .max_by(|&&a, &&b| {
                model::dot(&v[a], &middle)
                    .total_cmp(&model::dot(&v[b], &middle))
                    .then(b.cmp(&a))
            })
            .copied()
    }

    /// Where the message `i`, filed under no label, would go: the label
    /// the filed mail nearest it votes for, unless it was said not to be one
    /// for it.
    pub fn suggestion(&self, i: usize) -> Option<&str> {
        let d = self.dots.get(i)?;
        if !d.labels.is_empty() || d.system.sent {
            return None;
        }
        self.voted.get(i)?.as_deref()
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
    last: Option<Last>,
    corpus: Option<Corpus>,
    /// How likely each message in an Inbox is to be thrown away, where it
    /// is sure enough to say.
    thrown: BTreeMap<Key, f32>,
    /// The messages taught as kept: looked at before.
    looked: std::collections::BTreeSet<Key>,
}

/// A message as it is drawn from: its record, its labels, whether it has
/// one of letterbox's, its state, and whether another account has it.
type Drawn = (Record, Vec<String>, bool, System, bool);

/// What the last snapshot made, to be kept where nothing it was made from
/// changed.
struct Last {
    model: Arc<Model>,
    clustering: Clustering,
    hues: Vec<f32>,
    at: Option<Vec<Point>>,
    faithfulness: Option<f32>,
    not_it: std::collections::BTreeSet<(Key, String)>,
}

/// What a snapshot of `index` by `lens` is made from: begun from `last`,
/// its clusters and where its messages were, and from what `corpus` has
/// read; laid out if `layout` and it never was.
pub fn input(
    index: &Index,
    lens: Lens,
    last: Option<&Snapshot>,
    corpus: Option<Corpus>,
    layout: bool,
) -> Input {
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
    let known = index.known();
    let thrown = index
        .records
        .values()
        .filter(|r| index.system(&r.key).inbox)
        .filter_map(|r| Some((r.key.clone(), index.filter.would_throw(r, &known)?)))
        .collect();
    let looked = index
        .filter
        .taught
        .iter()
        .filter(|t| !t.thrown)
        .map(|t| t.key.clone())
        .collect();
    let last = last.filter(|s| s.lens == lens);
    let before = last
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
        last: last.map(|s| Last {
            model: s.model.clone(),
            clustering: s.clustering.clone(),
            hues: s.hues.clone(),
            at: s.at.clone(),
            faithfulness: s.faithfulness,
            not_it: s.not_it.clone(),
        }),
        corpus,
        thrown,
        looked,
    }
}

/// The snapshot `input` makes: the model, its clusters and hues, and where
/// each message is. What the model has read follows the index: new mail
/// is read alone, and where no message came, went or changed its labels,
/// the last snapshot's model, clusters and map are kept as they were.
/// Messages stay where the layout kept, or the last snapshot, has them,
/// and new mail is settled among its nearest; a layout is made now if one
/// is wanted and none was kept.
pub fn compute(input: Input) -> Snapshot {
    let given: Vec<model::Given> = input
        .records
        .iter()
        .map(|(r, labels, _, system)| (r, labels.clone(), system.sent))
        .collect();
    let (mut corpus, changed) = match input.corpus {
        Some(mut c) if c.lens == input.lens => {
            let changed = c.follow(&given);
            (c, changed)
        }
        _ => (Corpus::read(&given, input.lens), true),
    };
    let unchanged = input.last.as_ref().filter(|l| {
        !changed
            && l.not_it == input.not_it
            && l.model.len() == given.len()
            && given.iter().all(|(r, labels, _)| {
                l.model
                    .place
                    .get(&r.key)
                    .is_some_and(|&i| l.model.labels[i] == *labels)
            })
    });
    let n = corpus.len();
    let (model, clustering, hues) = match unchanged {
        Some(l) => (l.model.clone(), l.clustering.clone(), l.hues.clone()),
        None => {
            let model = Arc::new(corpus.model(&given, &input.not_it));
            let clustering = model::cluster(&model, &input.before, SEED);
            let hues = model.hues();
            (model, clustering, hues)
        }
    };
    drop(given);
    let (at, fresh, faithfulness) = if let Some(kept) = &input.layout {
        // Where the layout kept has each message, else where the last
        // snapshot had it; the rest settled among their nearest.
        let mut known: Vec<Option<Point>> = model
            .keys
            .iter()
            .map(|k| kept.get(k).map(|&(x, y)| [x, y]))
            .collect();
        if let Some(l) = &input.last
            && let Some(old) = &l.at
        {
            for (i, k) in model.keys.iter().enumerate() {
                if known[i].is_none()
                    && let Some(&j) = l.model.place.get(k)
                {
                    known[i] = old.get(j).copied();
                }
            }
        }
        let new: Vec<usize> = (0..n).filter(|&i| known[i].is_none()).collect();
        let mut at: Vec<Point> = known.iter().map(|p| p.unwrap_or([0.0, 0.0])).collect();
        model::settle(&model, &mut at, &new, SEED);
        // As faithful as it was, where nothing moved or changed.
        let faith = match unchanged {
            Some(l) if l.at.as_ref() == Some(&at) => l.faithfulness,
            _ => None,
        }
        .unwrap_or_else(|| model::faithfulness(&model, &at));
        (Some(at), None, Some(faith))
    } else if input.layout_wanted && n > 0 {
        let at = model::layout(&model, None, SEED);
        let fresh = model
            .keys
            .iter()
            .zip(&at)
            .map(|(k, p)| (k.clone(), p[0], p[1]))
            .collect();
        let faith = model::faithfulness(&model, &at);
        (Some(at), Some(fresh), Some(faith))
    } else {
        (None, None, None)
    };
    let mut ids: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (r, _, _, _) in &input.records {
        if !r.message_id.is_empty() {
            ids.entry(r.message_id.as_str())
                .or_default()
                .push(r.key.account.as_str());
        }
    }
    let twins: Vec<bool> = input
        .records
        .iter()
        .map(|(r, _, _, _)| {
            ids.get(r.message_id.as_str())
                .is_some_and(|a| a.iter().any(|x| *x != r.key.account))
        })
        .collect();
    drop(ids);
    // The dots, in the model's order, made of the records themselves.
    let mut records: Vec<Option<Drawn>> = input
        .records
        .into_iter()
        .zip(twins)
        .map(|((r, labels, mine, system), twin)| Some((r, labels, mine, system, twin)))
        .collect();
    let mut order = vec![0usize; n];
    for (i, slot) in records.iter().enumerate() {
        if let Some((r, ..)) = slot
            && let Some(&at) = model.place.get(&r.key)
        {
            order[at] = i;
        }
    }
    let dots: Vec<Dot> = order
        .into_iter()
        .map(|i| {
            let (r, labels, mine, system, twin) =
                records[i].take().expect("each record a dot once");
            let thrown = input.thrown.get(&r.key).copied();
            let looked = input.looked.contains(&r.key);
            Dot {
                key: r.key,
                from: r.from,
                address: r.address,
                to: r.to,
                subject: r.subject,
                preview: r.preview,
                message_id: r.message_id,
                date: r.date,
                arrived: r.date.unwrap_or(r.first_seen).min(r.first_seen),
                size: r.size,
                flags: r.flags,
                labels,
                mine,
                system,
                places: r.places,
                twin,
                thrown,
                looked,
                parents: r.parents,
                thread: r.thread,
            }
        })
        .collect();
    // One group a label, however many clusters its mail is split between:
    // the first cluster's number, the parts' messages together, and its
    // largest part where its anchor is.
    let mut groups: Vec<Group> = Vec::new();
    let mut of_label: BTreeMap<String, usize> = BTreeMap::new();
    for c in &clustering.clusters {
        let merged = c
            .label
            .as_ref()
            .and_then(|l| of_label.get(&crate::index::fold(l)).copied());
        match merged {
            Some(g) => {
                let g = &mut groups[g];
                if c.members.len() > g.core.len() {
                    g.core = c.members.clone();
                }
                g.members.extend(&c.members);
                g.members.sort_unstable();
            }
            None => {
                if let Some(l) = &c.label {
                    of_label.insert(crate::index::fold(l), groups.len());
                }
                groups.push(Group {
                    id: c.id,
                    members: c.members.clone(),
                    core: c.members.clone(),
                    label: c.label.clone(),
                    words: c.words.clone(),
                    hue: 0.0,
                    unread: 0,
                });
            }
        }
    }
    let mut group_at = vec![0usize; dots.len()];
    for (g, group) in groups.iter_mut().enumerate() {
        group.unread = group
            .members
            .iter()
            .filter(|&&i| !dots[i].flags.seen)
            .count();
        for &i in &group.members {
            group_at[i] = g;
        }
    }
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

    let voted = model.voted(|i, l| {
        input
            .not_it
            .contains(&(model.keys[i].clone(), l.to_string()))
    });
    Snapshot {
        version: input.version,
        lens: input.lens,
        voted,
        dots,
        model,
        clustering,
        groups,
        group_at,
        hues,
        at,
        faithfulness,
        fresh,
        not_it: input.not_it,
        corpus: Some(corpus),
    }
}

/// The map of `s` laid out again, begun from where each message is, and
/// laid back onto the old by Procrustes: by the labelled messages, the
/// labels' anchors, where there are three or more, else by all.
pub fn relayout(s: &Snapshot) -> Vec<Point> {
    let Some(old) = &s.at else {
        return model::layout(&s.model, None, SEED);
    };
    let start: Vec<Option<Point>> = old.iter().copied().map(Some).collect();
    let new = model::layout(&s.model, Some(&start), SEED);
    let labelled: Vec<usize> = (0..s.dots.len())
        .filter(|&i| !s.dots[i].labels.is_empty())
        .collect();
    let held: Vec<usize> = if labelled.len() >= 3 {
        labelled
    } else {
        (0..s.dots.len()).collect()
    };
    let a: Vec<Point> = held.iter().map(|&i| new[i]).collect();
    let b: Vec<Point> = held.iter().map(|&i| old[i]).collect();
    model::procrustes(&a, &b, &new)
}

/// `at`, where the messages `keys` would be, as where each message of `s`
/// would be: as `at` has it, or, come since, where `s` has it.
pub fn onto(s: &Snapshot, keys: &[Key], at: &[Point]) -> Option<Vec<Point>> {
    let now = s.at.as_ref()?;
    let by: std::collections::HashMap<&Key, Point> = keys.iter().zip(at.iter().copied()).collect();
    Some(
        s.model
            .keys
            .iter()
            .zip(now)
            .map(|(k, &p)| by.get(k).copied().unwrap_or(p))
            .collect(),
    )
}

/// How many messages of `s` would move noticeably, laid out at `at`: by
/// more than a twentieth of the map's breadth.
pub fn moved(s: &Snapshot, at: &[Point]) -> usize {
    let Some(old) = &s.at else {
        return at.len();
    };
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in old {
        for d in 0..2 {
            lo[d] = lo[d].min(p[d]);
            hi[d] = hi[d].max(p[d]);
        }
    }
    let breadth = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2)).sqrt();
    old.iter()
        .zip(at)
        .filter(|(p, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() > breadth / 20.0)
        .count()
}

/// Lay the map of `index` out again by every lens, from nothing, as the
/// seed has it, and keep the layouts; what to say of it.
pub fn rebuild(index: &mut Index) -> anyhow::Result<String> {
    let mut said = Vec::new();
    for lens in Lens::ALL {
        let mut input = input(index, lens, None, None, true);
        input.layout = None;
        let s = compute(input);
        if let Some(fresh) = s.fresh {
            index.keep(vec![crate::index::Kept::Layout { lens, at: fresh }])?;
        }
        said.push(format!(
            "{}, {:.0}% faithful",
            lens.name(),
            s.faithfulness.unwrap_or(1.0) * 100.0
        ));
    }
    index.flush()?;
    Ok(format!(
        "laid out {} messages by the model {}: {}",
        index.records.len(),
        model::VERSION,
        said.join("; ")
    ))
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
    grown: Option<(usize, usize, Arc<Branch>)>,
}

/// The messages of a snapshot that hold every word searched for.
pub struct Hits {
    query: String,
    /// The snapshot they were found in.
    of: usize,
    /// Whether each message holds them, by its place.
    pub matched: Vec<bool>,
    pub count: usize,
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
    fn nearest_block(&self, here: Option<u32>, dx: i32, dy: i32) -> Option<u32> {
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
            let count = matched.iter().filter(|&&m| m).count();
            self.hits = Some(Hits {
                query: query.to_string(),
                of,
                matched,
                count,
            });
        }
        self.hits.as_ref()
    }

    /// The messages drawn that hold what is searched for, newest first.
    fn found(&mut self, query: &str, now: i64) -> Vec<usize> {
        let shown = self.shown(now);
        let (Some(hits), Some(s)) = (
            self.hits(query).map(|h| h.matched.clone()),
            self.snapshot.clone(),
        ) else {
            return Vec::new();
        };
        let mut found: Vec<usize> = shown.into_iter().filter(|&i| hits[i]).collect();
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
