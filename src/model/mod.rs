//! The map's model, as arithmetic alone: no file, no server, no clock. A
//! message is read as tokens, its sender, the sender's domain, its list,
//! whom else it went to, and the words of its subject and of its start,
//! each hashed by FNV-1a to a bucket and weighted by how rare it is. Its
//! nearest are the messages whose tokens are most alike, by cosine, found
//! through which messages have each token. It becomes a direction in 64
//! dimensions, each bucket a vector of signs made from its hash and a
//! fixed seed, summed; and, since 64 dimensions blur, the direction is
//! taken half from its nearest. Labels teach it: a label is the mean of
//! its messages, a message not yet labelled is pulled toward a label by
//! the share of its tokens seen in that label's mail, and pushed away from
//! one it was said not to be. Clusters are found by angle, a cluster
//! holding two labels' mail is split between them, and the map is laid
//! out by UMAP over each message's nearest. Every step is seeded, so the
//! same mail gives the same map, bit for bit, however many threads it is
//! made on.

mod cluster;
mod corpus;
mod layout;
mod rarity;
mod tokens;
mod voting;
pub use self::cluster::*;
pub use self::corpus::*;
pub use self::layout::*;
pub use self::rarity::*;
pub use self::tokens::*;
pub use self::voting::*;

use crate::index::record::{Record, fnv};
use crate::index::{Key, Lens, fold};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// The model's version: vectors are made again when it changes.
pub const VERSION: u32 = 1;
pub const DIM: usize = 64;
/// How many buckets tokens are hashed to.
const BUCKETS: u64 = 1 << 20;
const SEED: u64 = 0x6c65_7474_6572_626f;
/// How far a message is pulled toward a label by the share of its tokens
/// in that label's mail; how far a labelled one is held to its label; how
/// far one is pushed from a label it is not.
const PULL: f32 = 2.0;
const HOLD: f32 = 0.6;
const PUSH: f32 = 0.8;
/// How near to its cluster's centre, by cosine, a message is to be of it,
/// and not a loner.
const LONER: f32 = 0.2;
/// How many neighbours each message has in the graph the map is laid out
/// from: UMAP's `n_neighbors`, less the message itself.
const NEIGHBOURS: usize = 15;
/// How many others each end of an edge is pushed from, each time the edge
/// is drawn.
const NEGATIVE: usize = 5;
/// Of the messages that share a token with one, how many it is compared
/// with by that token, at most; and how many of the likeliest of them are
/// compared with it token by token.
const WINDOW: usize = 512;
const SHORTLIST: usize = 48;
/// How many rounds new messages are settled among the rest for.
const SETTLE: usize = 100;
/// How much nearer two messages are laid out for sharing a label.
const SHARED: f32 = 0.5;
/// How near a label, in degrees, a message is to be suggested for it.
pub const NEAR: f32 = 30.0;

pub type Vector = [f32; DIM];
pub type Point = [f32; 2];

/// The messages, as the model has them, for one lens.
pub struct Model {
    pub lens: Lens,
    pub keys: Vec<Key>,
    /// Where each message is, by its key.
    pub place: HashMap<Key, usize>,
    /// Each message's direction, labels' teaching taken in.
    pub vectors: Vec<Vector>,
    /// Each message's nearest by its tokens themselves, as (which,
    /// `1 − cos`), nearest first: what the map is laid out from.
    pub near: Vec<Vec<(usize, f32)>>,
    /// Each message's words, by what they weigh in it: to name clusters by.
    pub words: Vec<Arc<[(String, f32)]>>,
    /// Each message's sender, as named.
    pub senders: Vec<Arc<str>>,
    /// Each message's labels, as given.
    pub labels: Vec<Vec<String>>,
    /// Each label, folded, its name as first given, and the mean of its
    /// messages.
    pub label_vectors: BTreeMap<String, (String, Vector)>,
    /// The plane hues are read in.
    pub axes: Axes,
}

/// One message, as the model is given it: its record, its labels, and
/// whether it was sent from here.
pub type Given<'a> = (&'a Record, Vec<String>, bool);

/// The words of a message's tokens, each by what it weighs.
fn words_of(tokens: &[(u64, String, f32)], rarity: &Rarity) -> Arc<[(String, f32)]> {
    tokens
        .iter()
        .filter_map(|(b, s, w)| Some((s.strip_prefix("w:")?.to_string(), w * rarity.idf(*b))))
        .collect()
}

impl Model {
    /// The model of `messages`, in the order given (the index's, by key),
    /// as `lens` reads them, taught by their labels and by `not_it`, each
    /// message and label, folded, it was said not to be.
    #[cfg(test)]
    pub fn build(messages: &[Given], not_it: &BTreeSet<(Key, String)>, lens: Lens) -> Model {
        Corpus::read(messages, lens).model(messages, not_it)
    }

    /// Each message's hue, in degrees: the angle of its direction in the
    /// plane of the axes.
    pub fn hues(&self) -> Vec<f32> {
        self.vectors.par_iter().map(|v| self.axes.hue(v)).collect()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Each message's nearest as the map is laid out from them: by their
    /// tokens, and nearer by half where they share a label.
    fn ties(&self) -> Vec<Vec<(usize, f32)>> {
        let folded: Vec<Vec<String>> = self
            .labels
            .iter()
            .map(|ls| ls.iter().map(|l| fold(l)).collect())
            .collect();
        self.near
            .par_iter()
            .enumerate()
            .map(|(i, list)| {
                list.iter()
                    .map(|&(j, d)| {
                        let shared = folded[i].iter().any(|l| folded[j].contains(l));
                        (j, if shared { d * SHARED } else { d })
                    })
                    .collect()
            })
            .collect()
    }

    /// Where each message filed under no label would go: the label its
    /// `VOTERS` nearest filed messages vote for, the nearer the more; none
    /// that `skip` turns down, by its place and the label folded. Graded on
    /// real mail, filed mail was put back so three times as often as by
    /// the label whose mean is nearest.
    pub fn voted(&self, skip: impl Fn(usize, &str) -> bool + Sync) -> Vec<Option<String>> {
        let labels: Vec<BTreeSet<String>> = self
            .labels
            .iter()
            .map(|ls| ls.iter().map(|l| fold(l)).collect())
            .collect();
        let filed: Vec<usize> = (0..self.len()).filter(|&i| !labels[i].is_empty()).collect();
        (0..self.len())
            .into_par_iter()
            .map(|i| {
                if !labels[i].is_empty() {
                    return None;
                }
                ranked_votes(&labels, &self.vectors, &filed, i, VOTERS, VOTED_WITHIN)
                    .into_iter()
                    .find(|l| !skip(i, l))
                    .map(|l| {
                        self.label_vectors
                            .get(&l)
                            .map_or(l.clone(), |(name, _)| name.clone())
                    })
            })
            .collect()
    }

    /// The `n` messages nearest `i` by angle, nearest first, and how far
    /// each is, in degrees.
    pub fn nearest(&self, i: usize, n: usize) -> Vec<(usize, f32)> {
        let mut all: Vec<(usize, f32)> = (0..self.len())
            .filter(|&j| j != i)
            .map(|j| (j, degrees(&self.vectors[i], &self.vectors[j])))
            .collect();
        all.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        all.truncate(n);
        all
    }

    /// The messages that would go to `label` too: those without it, not
    /// said not to be, within `NEAR` degrees of it, nearest first.
    pub fn suggestions(&self, label: &str, not_it: &BTreeSet<(Key, String)>) -> Vec<(usize, f32)> {
        let folded = fold(label);
        let Some((_, l)) = self.label_vectors.get(&folded) else {
            return Vec::new();
        };
        let mut out: Vec<(usize, f32)> = (0..self.len())
            .filter(|&i| !self.labels[i].iter().any(|x| fold(x) == folded))
            .filter(|&i| !not_it.contains(&(self.keys[i].clone(), folded.clone())))
            .map(|i| (i, degrees(&self.vectors[i], l)))
            .filter(|&(_, d)| d <= NEAR)
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        out
    }
}
