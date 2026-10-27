//! The map: every message of every account in the middle pane, in its
//! cluster, near what it is about and who it is from, the clusters a
//! mosaic, each gone into as a mosaic of its parts. What is drawn is a
//! snapshot of the index, made off the screen's thread and made again as
//! the index changes: clusters follow each change, and the layout, which
//! places each block by those it lies by, moves only when asked to.

mod looking;
mod snapshot;
pub use self::looking::*;
pub use self::snapshot::*;

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
