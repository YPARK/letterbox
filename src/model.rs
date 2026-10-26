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

const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "do", "for", "from",
    "has", "have", "here", "i", "if", "in", "is", "it", "its", "me", "my", "no", "not", "of", "on",
    "or", "our", "so", "that", "the", "this", "to", "up", "was", "we", "were", "will", "with",
    "would", "you", "your", "re", "fwd", "fw", "aw", "us", "all", "one", "now",
];

/// The words of `text`, as tokens are made of them: in lower case, split
/// where a letter or digit is not, without the commonest and the numbers,
/// a plural's `s` let go.
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !STOP.contains(w))
        .map(|w| match w.strip_suffix('s') {
            Some(stem) if stem.chars().count() >= 3 && !stem.ends_with('s') => stem.to_string(),
            _ => w.to_string(),
        })
        .collect()
}

/// A subject without the `Re:` and `Fwd:` it is answered or passed on with.
pub fn bare_subject(subject: &str) -> &str {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let Some(n) = ["re:", "fwd:", "fw:", "aw:", "re :"]
            .iter()
            .find(|p| lower.starts_with(**p))
            .map(|p| p.len())
        else {
            return s;
        };
        s = s[n..].trim_start();
    }
}

/// A message's tokens, and what each weighs, as `lens` reads it. The
/// people of mail sent from here are those it went to, as the sender is
/// of mail come in, so that an answer lies with the mail it answers.
pub fn tokens(r: &Record, sent: bool, lens: Lens) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = Vec::new();
    let mut add = |t: String, w: f32| {
        if !out.iter().any(|(x, _)| *x == t) {
            out.push((t, w));
        }
    };
    if !r.list.is_empty() {
        add(format!("list:{}", r.list), 3.0);
    }
    if lens != Lens::Words {
        let (from, copied): (&[String], Vec<&String>) = if sent {
            (&r.to, r.cc.iter().collect())
        } else {
            (
                std::slice::from_ref(&r.address),
                r.to.iter().chain(&r.cc).collect(),
            )
        };
        for a in from.iter().filter(|a| !a.is_empty()) {
            add(format!("from:{a}"), 2.5);
            if let Some((_, domain)) = a.rsplit_once('@') {
                add(format!("dom:{domain}"), 1.0);
            }
        }
        for a in copied {
            if *a != r.address {
                add(format!("to:{a}"), 0.8);
            }
        }
    }
    if lens != Lens::People {
        for w in words(bare_subject(&r.subject)) {
            add(format!("w:{w}"), 1.5);
        }
        for w in words(&r.preview).into_iter().take(20) {
            add(format!("w:{w}"), 0.7);
        }
    }
    out
}

fn bucket(token: &str) -> u64 {
    fnv(token.as_bytes()) % BUCKETS
}

/// The next of a sequence of numbers made from a seed, by SplitMix64.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Numbers that look random, the same for the same seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ SEED)
    }

    fn unit(&mut self) -> f64 {
        (splitmix(&mut self.0) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n.saturating_sub(1))
    }

    fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit().max(1e-12), self.unit());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

pub fn dot(a: &Vector, b: &Vector) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalised(mut v: Vector) -> Vector {
    let n = dot(&v, &v).sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// The angle between two unit vectors, in degrees.
pub fn degrees(a: &Vector, b: &Vector) -> f32 {
    dot(a, b).clamp(-1.0, 1.0).acos().to_degrees()
}

/// A message's tokens, each by its bucket and what it weighs before how
/// rare it is, in order of bucket.
pub type Bag = Vec<(u64, f32)>;

/// `tokens` as a bag: by bucket, two tokens of one bucket as one.
fn bag_of(tokens: &[(u64, String, f32)]) -> Bag {
    let mut bag: Bag = tokens.iter().map(|(b, _, w)| (*b, *w)).collect();
    bag.sort_by_key(|t| t.0);
    bag.dedup_by(|later, kept| {
        let same = later.0 == kept.0;
        if same {
            kept.1 += later.1;
        }
        same
    });
    bag
}

/// How many of the messages have each bucket, and of how many.
#[derive(Clone, Debug, Default)]
pub struct Rarity {
    seen: HashMap<u64, u32>,
    n: usize,
}

impl Rarity {
    fn of(bags: &[Bag]) -> Rarity {
        let mut r = Rarity::default();
        for bag in bags {
            r.add(bag);
        }
        r
    }

    fn add(&mut self, bag: &Bag) {
        self.n += 1;
        for &(b, _) in bag {
            *self.seen.entry(b).or_default() += 1;
        }
    }

    fn remove(&mut self, bag: &Bag) {
        self.n -= 1;
        for &(b, _) in bag {
            if let Some(seen) = self.seen.get_mut(&b) {
                *seen -= 1;
                if *seen == 0 {
                    self.seen.remove(&b);
                }
            }
        }
    }

    /// How rare a bucket is: the more messages have it, the less.
    fn idf(&self, b: u64) -> f32 {
        let n = self.n as f32;
        ((n + 1.0) / (self.seen.get(&b).copied().unwrap_or(0) as f32 + 1.0)).ln() + 1.0
    }

    /// What a token of the bucket `b` weighs, for each of its own weight:
    /// by how rare it is. A token of one message alone tells it from none,
    /// and one of most tells none apart: either is only noise, and weighs
    /// nothing.
    fn factor(&self, b: u64) -> f32 {
        let n = self.n as f32;
        let seen = self.seen.get(&b).copied().unwrap_or(0) as f32;
        let telling = n < 20.0 || (seen > 1.0 && seen <= n / 2.0);
        if telling { self.idf(b) } else { 0.0 }
    }

    /// The length of `bag`, each token weighed.
    fn norm(&self, bag: &Bag) -> f32 {
        bag.iter()
            .map(|&(b, w)| (w * self.factor(b)).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    /// The cosine of two bags, each token weighed, of the lengths given.
    fn cosine(&self, a: &Bag, b: &Bag, na: f32, nb: f32) -> f32 {
        if na <= 0.0 || nb <= 0.0 {
            return 0.0;
        }
        let (mut i, mut j, mut sum) = (0, 0, 0f32);
        while i < a.len() && j < b.len() {
            match a[i].0.cmp(&b[j].0) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    sum += a[i].1 * b[j].1 * self.factor(a[i].0).powi(2);
                    i += 1;
                    j += 1;
                }
            }
        }
        sum / (na * nb)
    }

    /// `bag` as a direction in `DIM` dimensions: each bucket a vector of
    /// signs made from its hash, weighed, summed and made of unit length.
    fn project(&self, bag: &Bag) -> Vector {
        let mut v = [0f32; DIM];
        for &(b, w) in bag {
            let mut state = b ^ SEED;
            let signs = splitmix(&mut state);
            let weight = w * self.factor(b);
            for (d, x) in v.iter_mut().enumerate() {
                *x += if signs >> d & 1 == 1 { weight } else { -weight };
            }
        }
        normalised(v)
    }
}

/// Which messages have each bucket, in their order, and what it weighs in
/// each before how rare it is.
pub type Postings = HashMap<u64, Vec<(u32, f32)>>;

fn postings(bags: &[Bag]) -> Postings {
    let mut out: Postings = HashMap::new();
    for (i, bag) in bags.iter().enumerate() {
        for &(b, w) in bag {
            out.entry(b).or_default().push((i as u32, w));
        }
    }
    out
}

/// The `k` messages nearest the message `i` by its tokens themselves, as
/// (which, `1 − cos`), nearest first. Those that share a token with it are
/// found by `postings`; of a token many messages have, only the `WINDOW`
/// nearest it in their order. The likeliest `SHORTLIST` are then compared
/// token by token. `acc` is a number for every message, all nought, and
/// left so.
fn kin(
    i: usize,
    k: usize,
    bags: &[Bag],
    norms: &[f32],
    rarity: &Rarity,
    postings: &Postings,
    acc: &mut [f32],
) -> Vec<(usize, f32)> {
    if norms[i] <= 0.0 {
        return Vec::new();
    }
    let mut touched: Vec<usize> = Vec::new();
    for &(b, w) in &bags[i] {
        let f = rarity.factor(b);
        let Some(post) = postings.get(&b).filter(|_| f > 0.0) else {
            continue;
        };
        let from = if post.len() <= WINDOW {
            0
        } else {
            let at = post.partition_point(|p| (p.0 as usize) < i);
            at.saturating_sub(WINDOW / 2).min(post.len() - WINDOW)
        };
        for &(j, wj) in &post[from..(from + WINDOW).min(post.len())] {
            let j = j as usize;
            let add = w * wj * f * f;
            if j == i || add <= 0.0 {
                continue;
            }
            if acc[j] == 0.0 {
                touched.push(j);
            }
            acc[j] += add;
        }
    }
    let by = |a: &(usize, f32), b: &(usize, f32)| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0));
    let mut short: Vec<(usize, f32)> = touched
        .iter()
        .map(|&j| (j, acc[j] / norms[j].max(1e-9)))
        .collect();
    for &j in &touched {
        acc[j] = 0.0;
    }
    if short.len() > SHORTLIST {
        short.select_nth_unstable_by(SHORTLIST, by);
        short.truncate(SHORTLIST);
    }
    let mut near: Vec<(usize, f32)> = short
        .into_iter()
        .map(|(j, _)| {
            let cos = rarity.cosine(&bags[i], &bags[j], norms[i], norms[j]);
            (j, (1.0 - cos).max(0.0))
        })
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    near.truncate(k);
    near
}

/// Each message's `k` nearest by its tokens, found in parallel; the same
/// on any number of threads.
fn kin_all(
    bags: &[Bag],
    norms: &[f32],
    rarity: &Rarity,
    postings: &Postings,
    k: usize,
) -> Vec<Vec<(usize, f32)>> {
    let n = bags.len();
    (0..n)
        .into_par_iter()
        .map_init(
            || vec![0f32; n],
            |acc, i| kin(i, k, bags, norms, rarity, postings, acc),
        )
        .collect()
}

/// Each of the messages `which`, with its `k` nearest by tokens, every
/// pair compared: what `kin` is checked against.
#[cfg(test)]
pub fn nearest_by_every_pair(
    messages: &[Given],
    lens: Lens,
    k: usize,
    which: impl Iterator<Item = usize>,
) -> Vec<(usize, Vec<(usize, f32)>)> {
    let bags: Vec<Bag> = messages
        .iter()
        .map(|(r, _, sent)| {
            let t: Vec<(u64, String, f32)> = tokens(r, *sent, lens)
                .into_iter()
                .map(|(t, w)| (bucket(&t), t, w))
                .collect();
            bag_of(&t)
        })
        .collect();
    let rarity = Rarity::of(&bags);
    let norms: Vec<f32> = bags.iter().map(|b| rarity.norm(b)).collect();
    which
        .map(|i| {
            let mut all: Vec<(usize, f32)> = (0..bags.len())
                .filter(|&j| j != i && norms[i] > 0.0)
                .map(|j| {
                    (
                        j,
                        1.0 - rarity.cosine(&bags[i], &bags[j], norms[i], norms[j]),
                    )
                })
                .filter(|&(_, d)| d < 1.0)
                .collect();
            all.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            all.truncate(k);
            (i, all)
        })
        .collect()
}

/// The direction of the message `i`, made less noisy: its own, and the
/// mean of its nearest by tokens, each by how near, in equal parts.
fn smoothed(projected: &[Vector], i: usize, near: &[(usize, f32)]) -> Vector {
    let mut v = projected[i];
    let total: f32 = near.iter().map(|&(_, d)| 1.0 - d).sum();
    if total > 0.0 {
        for &(j, d) in near {
            let w = (1.0 - d) / total;
            v.iter_mut()
                .zip(&projected[j])
                .for_each(|(x, y)| *x += w * y);
        }
    }
    normalised(v)
}

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

/// How much the mail may grow, or shrink, since it was last read whole,
/// before it is read whole again, rarity and all: by a quarter, or a fifth.
const GROWN: f32 = 1.25;
const SHRUNK: f32 = 0.8;

/// How well labels were put back: of so many filed, how many under one
/// of their own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LabelCheck {
    pub hits: usize,
    pub total: usize,
    /// How many were said to go somewhere at all.
    pub said: usize,
}

/// How many filed messages nearest one vote for where it would go, and
/// how near, in degrees, each is to be to vote at all: graded on real
/// mail, within 45° it said where for nine in ten filed messages, and was
/// right for four in five of them; nearer, it said less, and farther, it
/// was less often right.
pub const VOTERS: usize = 10;
pub const VOTED_WITHIN: f32 = 45.0;

/// How many messages hold each label.
fn label_counts(labels: &[BTreeSet<String>]) -> HashMap<String, usize> {
    let mut count = HashMap::new();
    for ls in labels {
        for l in ls {
            *count.entry(l.clone()).or_insert(0) += 1;
        }
    }
    count
}

/// The label the `k` messages of `filed` nearest `i` by `base` vote for,
/// each its labels, the nearer the more; `i` itself not among them.
fn vote(
    labels: &[BTreeSet<String>],
    base: &[Vector],
    filed: &[usize],
    i: usize,
    k: usize,
    within: f32,
) -> Option<String> {
    ranked_votes(labels, base, filed, i, k, within)
        .into_iter()
        .next()
}

/// The labels the `k` messages of `filed` nearest `i` vote for, most voted
/// first: of those within `within` degrees of it, if any are.
fn ranked_votes(
    labels: &[BTreeSet<String>],
    base: &[Vector],
    filed: &[usize],
    i: usize,
    k: usize,
    within: f32,
) -> Vec<String> {
    let least = within.to_radians().cos();
    let mut near: Vec<(usize, f32)> = filed
        .iter()
        .copied()
        .filter(|&j| j != i)
        .map(|j| (j, dot(&base[i], &base[j])))
        .filter(|&(_, cos)| cos >= least)
        .collect();
    let k = k.min(near.len());
    if k == 0 {
        return Vec::new();
    }
    near.select_nth_unstable_by(k - 1, |a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut votes: BTreeMap<&String, f32> = BTreeMap::new();
    for &(j, cos) in &near[..k] {
        for l in &labels[j] {
            *votes.entry(l).or_default() += cos.max(0.0) + 1e-3;
        }
    }
    let mut ranked: Vec<(&String, f32)> = votes.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    ranked.into_iter().map(|(l, _)| l.clone()).collect()
}

/// How well labels would be suggested by always the commonest: the
/// baseline any other is to beat.
pub fn commonest_label_check(messages: &[Given]) -> LabelCheck {
    let labels: Vec<BTreeSet<String>> = messages
        .iter()
        .map(|(_, l, _)| l.iter().map(|x| fold(x)).collect())
        .collect();
    let count = label_counts(&labels);
    let Some(commonest) = count
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(l, _)| l.clone())
    else {
        return LabelCheck::default();
    };
    let eligible: Vec<&BTreeSet<String>> = labels
        .iter()
        .filter(|ls| ls.iter().any(|l| count[l] >= 2))
        .collect();
    LabelCheck {
        hits: eligible.iter().filter(|ls| ls.contains(&commonest)).count(),
        said: eligible.len(),
        total: eligible.len(),
    }
}

/// What the model has read of each message, kept from one snapshot to the
/// next, so that new mail is read alone and the rest is not read again.
/// Each message's direction is made once, by how rare its tokens were
/// then; how near messages are is always by how rare they are now.
pub struct Corpus {
    pub lens: Lens,
    keys: Vec<Key>,
    place: HashMap<Key, usize>,
    sent: Vec<bool>,
    bags: Vec<Bag>,
    rarity: Rarity,
    postings: Postings,
    norms: Vec<f32>,
    projected: Vec<Vector>,
    /// Each direction made less noisy by its nearest, as it was read.
    base: Vec<Vector>,
    near: Vec<Vec<(usize, f32)>>,
    words: Vec<Arc<[(String, f32)]>>,
    senders: Vec<Arc<str>>,
    /// How many messages there were when all were last read together.
    whole: usize,
    /// The plane hues are read in, found once all are read together.
    axes: Option<Axes>,
}

/// A message read: its bag, its words and its sender.
fn read((r, _, sent): &Given, lens: Lens) -> (Bag, Vec<(u64, String, f32)>, Arc<str>) {
    let tokens: Vec<(u64, String, f32)> = tokens(r, *sent, lens)
        .into_iter()
        .map(|(t, w)| (bucket(&t), t, w))
        .collect();
    // Mail sent from here names no cluster by its sender.
    let sender: Arc<str> = if *sent {
        "".into()
    } else {
        r.from.as_str().into()
    };
    (bag_of(&tokens), tokens, sender)
}

impl Corpus {
    /// Every one of `messages` read, in the order given (the index's, by
    /// key), as `lens` reads them.
    pub fn read(messages: &[Given], lens: Lens) -> Corpus {
        let read: Vec<_> = messages.par_iter().map(|m| read(m, lens)).collect();
        let bags: Vec<Bag> = read.iter().map(|r| r.0.clone()).collect();
        let rarity = Rarity::of(&bags);
        let norms: Vec<f32> = bags.par_iter().map(|b| rarity.norm(b)).collect();
        let postings = postings(&bags);
        let near = kin_all(&bags, &norms, &rarity, &postings, NEIGHBOURS);
        let projected: Vec<Vector> = bags.par_iter().map(|b| rarity.project(b)).collect();
        let base: Vec<Vector> = (0..bags.len())
            .into_par_iter()
            .map(|i| smoothed(&projected, i, &near[i]))
            .collect();
        let words = read.iter().map(|(_, t, _)| words_of(t, &rarity)).collect();
        let keys: Vec<Key> = messages.iter().map(|(r, _, _)| r.key.clone()).collect();
        Corpus {
            lens,
            place: keys
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, k)| (k, i))
                .collect(),
            keys,
            sent: messages.iter().map(|m| m.2).collect(),
            bags,
            rarity,
            postings,
            norms,
            projected,
            base,
            near,
            words,
            senders: read.into_iter().map(|r| r.2).collect(),
            whole: messages.len(),
            axes: None,
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Follow `messages`, as they are now: read those new, let go those
    /// gone; or, if the mail has grown or shrunk too far since it was read
    /// whole, read it whole again. Whether anything changed.
    pub fn follow(&mut self, messages: &[Given]) -> bool {
        let now: HashMap<&Key, bool> = messages.iter().map(|(r, _, s)| (&r.key, *s)).collect();
        let gone: Vec<usize> = (0..self.len())
            .filter(|&i| now.get(&self.keys[i]) != Some(&self.sent[i]))
            .collect();
        let new: Vec<usize> = (0..messages.len())
            .filter(|&m| {
                let (r, _, sent) = &messages[m];
                self.place
                    .get(&r.key)
                    .is_none_or(|&i| self.sent[i] != *sent)
            })
            .collect();
        if gone.is_empty() && new.is_empty() {
            return false;
        }
        let whole = self.whole.max(1) as f32;
        let kept = self.len() - gone.len();
        if messages.len() as f32 > GROWN * whole || (kept as f32) < SHRUNK * whole {
            *self = Corpus::read(messages, self.lens);
            return true;
        }
        let mut short = if gone.is_empty() {
            Vec::new()
        } else {
            self.forget(&gone)
        };
        let first = self.len();
        self.take(&new.iter().map(|&m| &messages[m]).collect::<Vec<_>>());
        // The nearest of those that lost some, found again, now that the
        // new are there too.
        short.retain(|&i| i < first);
        let mut acc = vec![0f32; self.len()];
        for i in short {
            self.near[i] = kin(
                i,
                NEIGHBOURS,
                &self.bags,
                &self.norms,
                &self.rarity,
                &self.postings,
                &mut acc,
            );
        }
        true
    }

    /// Let the messages `gone`, by place, go; the places of those whose
    /// nearest lost some.
    fn forget(&mut self, gone: &[usize]) -> Vec<usize> {
        let mut to: Vec<Option<usize>> = vec![None; self.len()];
        let mut kept = 0;
        let mut g = gone.iter().peekable();
        for (i, slot) in to.iter_mut().enumerate() {
            if g.peek() == Some(&&i) {
                g.next();
                self.rarity.remove(&self.bags[i]);
            } else {
                *slot = Some(kept);
                kept += 1;
            }
        }
        fn keep<T>(v: &mut Vec<T>, to: &[Option<usize>]) {
            let mut i = 0;
            v.retain(|_| {
                i += 1;
                to[i - 1].is_some()
            });
        }
        keep(&mut self.keys, &to);
        keep(&mut self.sent, &to);
        keep(&mut self.bags, &to);
        keep(&mut self.projected, &to);
        keep(&mut self.base, &to);
        keep(&mut self.near, &to);
        keep(&mut self.words, &to);
        keep(&mut self.senders, &to);
        let mut short = Vec::new();
        for (i, list) in self.near.iter_mut().enumerate() {
            let before = list.len();
            list.retain_mut(|(j, _)| match to[*j] {
                Some(k) => {
                    *j = k;
                    true
                }
                None => false,
            });
            if list.len() < before {
                short.push(i);
            }
        }
        self.place = self
            .keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| (k, i))
            .collect();
        self.postings = postings(&self.bags);
        short
    }

    /// Read `messages`, new, after the rest.
    fn take(&mut self, messages: &[&Given]) {
        let first = self.len();
        let read: Vec<_> = messages.par_iter().map(|m| read(m, self.lens)).collect();
        for (k, (bag, _, sender)) in read.iter().enumerate() {
            let i = first + k;
            let (r, _, sent) = messages[k];
            self.rarity.add(bag);
            for &(b, w) in bag {
                self.postings.entry(b).or_default().push((i as u32, w));
            }
            self.place.insert(r.key.clone(), i);
            self.keys.push(r.key.clone());
            self.sent.push(*sent);
            self.bags.push(bag.clone());
            self.senders.push(sender.clone());
        }
        let rarity = &self.rarity;
        self.norms = self.bags.par_iter().map(|b| rarity.norm(b)).collect();
        for (_, tokens, _) in &read {
            self.words.push(words_of(tokens, rarity));
        }
        let n = self.len();
        let found: Vec<Vec<(usize, f32)>> = (first..n)
            .into_par_iter()
            .map_init(
                || vec![0f32; n],
                |acc, i| {
                    kin(
                        i,
                        NEIGHBOURS,
                        &self.bags,
                        &self.norms,
                        rarity,
                        &self.postings,
                        acc,
                    )
                },
            )
            .collect();
        self.near.extend(found);
        // Each new message is among the nearest of those nearest it, where
        // it is nearer than their farthest.
        for i in first..n {
            for k in 0..self.near[i].len() {
                let (j, d) = self.near[i][k];
                if j >= first {
                    continue;
                }
                let list = &mut self.near[j];
                if list.len() < NEIGHBOURS || list.last().is_some_and(|l| d < l.1) {
                    let at = list.partition_point(|x| x.1 < d || (x.1 == d && x.0 < i));
                    list.insert(at, (i, d));
                    list.truncate(NEIGHBOURS);
                }
            }
        }
        for i in first..n {
            let v = rarity.project(&self.bags[i]);
            self.projected.push(v);
        }
        for i in first..n {
            let v = smoothed(&self.projected, i, &self.near[i]);
            self.base.push(v);
        }
    }

    /// Each message's direction before labels teach it.
    pub fn base(&self) -> &[Vector] {
        &self.base
    }

    /// How well labels would be suggested from directions `base`: each
    /// message filed under labels with others in them, its own labels
    /// hidden, its labels made again without it and it pulled as mail not
    /// yet filed is, put back under one of its own by the label nearest it.
    pub fn label_check(&self, messages: &[Given], base: &[Vector]) -> LabelCheck {
        let n = self.len();
        let mut labels: Vec<BTreeSet<String>> = vec![BTreeSet::new(); n];
        for (r, l, _) in messages {
            if let Some(&i) = self.place.get(&r.key) {
                labels[i] = l.iter().map(|x| fold(x)).collect();
            }
        }
        // Each label's members, the sum of their directions, and how many
        // of them hold each token.
        let mut members: BTreeMap<&String, Vec<usize>> = BTreeMap::new();
        for (i, ls) in labels.iter().enumerate() {
            for l in ls {
                members.entry(l).or_default().push(i);
            }
        }
        let sums: BTreeMap<&String, (Vector, HashMap<u64, usize>)> = members
            .iter()
            .map(|(l, of)| {
                let mut sum = [0f32; DIM];
                let mut held: HashMap<u64, usize> = HashMap::new();
                for &i in of {
                    sum.iter_mut().zip(&base[i]).for_each(|(s, x)| *s += x);
                    for t in self.bags[i].iter() {
                        *held.entry(t.0).or_default() += 1;
                    }
                }
                (*l, (sum, held))
            })
            .collect();
        let weigh = |b: u64, w: f32| w * self.rarity.factor(b);
        let hits: Vec<bool> = (0..n)
            .into_par_iter()
            .filter(|&i| {
                labels[i]
                    .iter()
                    .any(|l| members.get(l).is_some_and(|m| m.len() >= 2))
            })
            .map(|i| {
                let bag = &self.bags[i];
                let total: f32 = bag.iter().map(|t| weigh(t.0, t.1)).sum();
                // Each label as it would be without this message.
                let without: Vec<(&String, Vector, f32)> = sums
                    .iter()
                    .filter_map(|(l, (sum, held))| {
                        let mine = labels[i].contains(*l);
                        if mine && members[l].len() < 2 {
                            return None;
                        }
                        let mut v = *sum;
                        if mine {
                            v.iter_mut().zip(&base[i]).for_each(|(s, x)| *s -= x);
                        }
                        let shared: f32 = bag
                            .iter()
                            .filter(|t| held.get(&t.0).is_some_and(|&c| c > usize::from(mine)))
                            .map(|t| weigh(t.0, t.1))
                            .sum();
                        let share = if total > 0.0 { shared / total } else { 0.0 };
                        Some((*l, normalised(v), share))
                    })
                    .collect();
                let mut v = base[i];
                for (_, l, share) in &without {
                    v.iter_mut()
                        .zip(l)
                        .for_each(|(x, y)| *x += PULL * share * y);
                }
                let v = normalised(v);
                without
                    .iter()
                    .min_by(|a, b| degrees(&v, &a.1).total_cmp(&degrees(&v, &b.1)))
                    .is_some_and(|(l, _, _)| labels[i].contains(*l))
            })
            .collect();
        LabelCheck {
            hits: hits.iter().filter(|h| **h).count(),
            said: hits.len(),
            total: hits.len(),
        }
    }

    /// How well labels would be suggested by the `k` filed messages
    /// nearest each, by directions `base`, each voting for its labels, the
    /// nearer the more: each filed message, its own labels hidden, put back
    /// under one of its own.
    pub fn label_check_nearest(
        &self,
        messages: &[Given],
        base: &[Vector],
        k: usize,
        within: f32,
    ) -> LabelCheck {
        let labels = self.labels_of(messages);
        let count = label_counts(&labels);
        let filed: Vec<usize> = (0..self.len()).filter(|&i| !labels[i].is_empty()).collect();
        let hits: Vec<(bool, bool)> = filed
            .par_iter()
            .copied()
            .filter(|&i| labels[i].iter().any(|l| count[l] >= 2))
            .map(|i| {
                let said = vote(&labels, base, &filed, i, k, within);
                (said.is_some(), said.is_some_and(|l| labels[i].contains(&l)))
            })
            .collect();
        LabelCheck {
            hits: hits.iter().filter(|h| h.1).count(),
            said: hits.iter().filter(|h| h.0).count(),
            total: hits.len(),
        }
    }

    /// Each message's labels, folded, by its place.
    fn labels_of(&self, messages: &[Given]) -> Vec<BTreeSet<String>> {
        let mut labels: Vec<BTreeSet<String>> = vec![BTreeSet::new(); self.len()];
        for (r, l, _) in messages {
            if let Some(&i) = self.place.get(&r.key) {
                labels[i] = l.iter().map(|x| fold(x)).collect();
            }
        }
        labels
    }

    /// The model of what is read, taught by the labels of `messages` and
    /// by `not_it`, each message and label, folded, it was said not to be.
    pub fn model(&mut self, messages: &[Given], not_it: &BTreeSet<(Key, String)>) -> Model {
        let n = self.len();
        let mut labels: Vec<Vec<String>> = vec![Vec::new(); n];
        for (r, l, _) in messages {
            if let Some(&i) = self.place.get(&r.key) {
                labels[i] = l.clone();
            }
        }
        let rarity = &self.rarity;
        let bags = &self.bags;
        let mut vectors = self.base.clone();
        let weigh = |b: u64, w: f32| w * rarity.factor(b);
        // What each label is: the mean of its messages, and the tokens seen
        // in them, by how rare each is.
        let mut members: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
        for (i, labels) in labels.iter().enumerate() {
            for l in labels {
                members
                    .entry(fold(l))
                    .or_insert_with(|| (l.clone(), Vec::new()))
                    .1
                    .push(i);
            }
        }
        let mut label_vectors = BTreeMap::new();
        let mut label_tokens: BTreeMap<&String, BTreeSet<u64>> = BTreeMap::new();
        for (folded, (name, of)) in &members {
            let mut sum = [0f32; DIM];
            for &i in of {
                sum.iter_mut().zip(&vectors[i]).for_each(|(s, x)| *s += x);
            }
            label_vectors.insert(folded.clone(), (name.clone(), normalised(sum)));
            label_tokens.insert(
                folded,
                of.iter()
                    .flat_map(|&i| bags[i].iter().map(|t| t.0))
                    .collect(),
            );
        }
        let keys = &self.keys;
        vectors.par_iter_mut().enumerate().for_each(|(i, vector)| {
            let mut v = *vector;
            let mine: BTreeSet<String> = labels[i].iter().map(|l| fold(l)).collect();
            if mine.is_empty() {
                let total: f32 = bags[i].iter().map(|t| weigh(t.0, t.1)).sum();
                for (folded, seen) in &label_tokens {
                    let shared: f32 = bags[i]
                        .iter()
                        .filter(|t| seen.contains(&t.0))
                        .map(|t| weigh(t.0, t.1))
                        .sum();
                    let share = if total > 0.0 { shared / total } else { 0.0 };
                    let l = &label_vectors[*folded].1;
                    v.iter_mut()
                        .zip(l)
                        .for_each(|(x, y)| *x += PULL * share * y);
                }
            } else {
                for folded in &mine {
                    let l = &label_vectors[folded].1;
                    v.iter_mut().zip(l).for_each(|(x, y)| *x += HOLD * y);
                }
            }
            for (folded, (_, l)) in &label_vectors {
                if not_it.contains(&(keys[i].clone(), folded.clone())) {
                    v.iter_mut().zip(l).for_each(|(x, y)| *x -= PUSH * y);
                }
            }
            *vector = normalised(v);
        });
        let axes = self.axes.get_or_insert_with(|| Axes::of(&vectors)).clone();
        Model {
            lens: self.lens,
            keys: self.keys.clone(),
            place: self.place.clone(),
            vectors,
            near: self.near.clone(),
            words: self.words.clone(),
            senders: self.senders.clone(),
            labels,
            label_vectors,
            axes,
        }
    }
}

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

/// A cluster: its members by their place in the model, its centre, the
/// label it is if it is one, its commonest words or senders, and the
/// number it keeps from one clustering to the next.
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub id: u32,
    pub members: Vec<usize>,
    pub centre: Vector,
    pub label: Option<String>,
    pub words: Vec<String>,
}

/// The clusters of a model, and which each message is in, and whether it
/// is far from its centre, fitting nowhere.
#[derive(Clone, Debug, Default)]
pub struct Clustering {
    pub clusters: Vec<Cluster>,
    pub of: Vec<usize>,
    pub loner: Vec<bool>,
}

/// How many clusters `n` messages make.
pub fn how_many(n: usize) -> usize {
    ((n as f64).sqrt().ceil() as usize)
        .clamp(1, 30)
        .min(n.max(1))
}

/// Spherical k-means: `k` centres of `vectors` by cosine, begun from
/// `start` where given, else by k-means++ with a fixed seed. Each round is
/// taken in parallel, and its sums in blocks added in turn, so the same
/// vectors give the same clusters on any number of threads.
fn kmeans(vectors: &[Vector], k: usize, start: Vec<Vector>, rng: &mut Rng) -> Vec<usize> {
    let n = vectors.len();
    if n == 0 || k == 0 {
        return vec![0; n];
    }
    let mut centres = start;
    centres.truncate(k);
    if centres.is_empty() {
        centres.push(vectors[rng.below(n)]);
    }
    let mut near: Vec<f32> = vec![f32::MIN; n];
    let mut counted = 0;
    while centres.len() < k {
        let new = &centres[counted..];
        near.par_iter_mut().zip(vectors).for_each(|(m, v)| {
            for c in new {
                *m = m.max(dot(v, c));
            }
        });
        counted = centres.len();
        let far: Vec<f64> = near
            .iter()
            .map(|&m| f64::from((1.0 - m).max(0.0)).powi(2))
            .collect();
        let total: f64 = far.iter().sum();
        let pick = if total <= 0.0 {
            rng.below(n)
        } else {
            let mut at = rng.unit() * total;
            far.iter()
                .position(|&f| {
                    at -= f;
                    at <= 0.0
                })
                .unwrap_or(n - 1)
        };
        centres.push(vectors[pick]);
    }
    let mut of = vec![usize::MAX; n];
    for _ in 0..60 {
        let best: Vec<usize> = vectors
            .par_iter()
            .map(|v| {
                (0..k)
                    .max_by(|&a, &b| {
                        dot(v, &centres[a])
                            .total_cmp(&dot(v, &centres[b]))
                            .then(b.cmp(&a))
                    })
                    .unwrap_or(0)
            })
            .collect();
        if best == of {
            break;
        }
        of = best;
        let blocks: Vec<(Vec<Vector>, Vec<usize>)> = vectors
            .par_chunks(1024)
            .zip(of.par_chunks(1024))
            .map(|(vs, os)| {
                let mut sums = vec![[0f32; DIM]; k];
                let mut counts = vec![0usize; k];
                for (v, &c) in vs.iter().zip(os) {
                    sums[c].iter_mut().zip(v).for_each(|(s, x)| *s += x);
                    counts[c] += 1;
                }
                (sums, counts)
            })
            .collect();
        let mut sums = vec![[0f32; DIM]; k];
        let mut counts = vec![0usize; k];
        for (bs, bc) in &blocks {
            for c in 0..k {
                sums[c].iter_mut().zip(&bs[c]).for_each(|(s, x)| *s += x);
                counts[c] += bc[c];
            }
        }
        for c in 0..k {
            if counts[c] == 0 {
                // An empty cluster begins again at the message farthest
                // from its own centre.
                let far = (0..n)
                    .min_by(|&a, &b| {
                        dot(&vectors[a], &centres[of[a]])
                            .total_cmp(&dot(&vectors[b], &centres[of[b]]))
                            .then(a.cmp(&b))
                    })
                    .unwrap_or(0);
                centres[c] = vectors[far];
            } else {
                centres[c] = normalised(sums[c]);
            }
        }
    }
    of
}

/// Cluster `model`, begun from `before`, the clusters it had, each by its
/// number, its centre and its members' keys, so that a cluster keeps its
/// number and its place in turn. A cluster holding the mail of more than
/// one label is split between them: each of its messages goes with its own
/// label, or with the nearest by angle.
pub fn cluster(model: &Model, before: &[(u32, Vector, Vec<Key>)], seed: u64) -> Clustering {
    let n = model.len();
    let mut rng = Rng::new(seed ^ model.lens as u64);
    let k = how_many(n);
    let start: Vec<Vector> = before.iter().map(|b| b.1).collect();
    let of = kmeans(&model.vectors, k, start, &mut rng);
    // Each cluster's members; split where they hold two labels' mail.
    let mut groups: Vec<(Vec<usize>, Option<String>)> = Vec::new();
    for c in 0..k {
        let members: Vec<usize> = (0..n).filter(|&i| of[i] == c).collect();
        if members.is_empty() {
            continue;
        }
        // A message's own label is its last: the deepest of a folder's,
        // Grant of Projects/Grant, or one given after them.
        let mut held: BTreeMap<String, usize> = BTreeMap::new();
        for &i in &members {
            if let Some(l) = model.labels[i].last() {
                *held.entry(fold(l)).or_default() += 1;
            }
        }
        if held.len() <= 1 {
            let label = held.keys().next().map(|f| model.label_vectors[f].0.clone());
            groups.push((members, label));
            continue;
        }
        let names: Vec<&String> = held.keys().collect();
        let mut split: BTreeMap<&String, Vec<usize>> = BTreeMap::new();
        for &i in &members {
            let own = model.labels[i]
                .iter()
                .rev()
                .map(|l| fold(l))
                .find(|f| held.contains_key(f));
            let to = match own {
                Some(f) => names.iter().position(|x| **x == f).unwrap_or(0),
                None => (0..names.len())
                    .max_by(|&a, &b| {
                        let va = &model.label_vectors[names[a]].1;
                        let vb = &model.label_vectors[names[b]].1;
                        dot(&model.vectors[i], va)
                            .total_cmp(&dot(&model.vectors[i], vb))
                            .then(b.cmp(&a))
                    })
                    .unwrap_or(0),
            };
            split.entry(names[to]).or_default().push(i);
        }
        for (f, members) in split {
            groups.push((members, Some(model.label_vectors[f].0.clone())));
        }
    }
    // Numbers kept from before: each old cluster to the new one it shares
    // most messages with.
    let old_of: HashMap<&Key, u32> = before
        .iter()
        .flat_map(|(id, _, keys)| keys.iter().map(move |k| (k, *id)))
        .collect();
    let mut pairs: Vec<(usize, usize, u32)> = Vec::new();
    for (g, (members, _)) in groups.iter().enumerate() {
        let mut shared: BTreeMap<u32, usize> = BTreeMap::new();
        for &i in members {
            if let Some(&id) = old_of.get(&model.keys[i]) {
                *shared.entry(id).or_default() += 1;
            }
        }
        pairs.extend(shared.into_iter().map(|(id, s)| (s, g, id)));
    }
    pairs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut ids: Vec<Option<u32>> = vec![None; groups.len()];
    let mut taken = BTreeSet::new();
    for (_, g, id) in pairs {
        if ids[g].is_none() && taken.insert(id) {
            ids[g] = Some(id);
        }
    }
    let mut next = before.iter().map(|b| b.0 + 1).max().unwrap_or(0);
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| groups[b].0.len().cmp(&groups[a].0.len()).then(a.cmp(&b)));
    for g in order {
        if ids[g].is_none() {
            ids[g] = Some(next);
            next += 1;
        }
    }
    let mut clusters: Vec<Cluster> = groups
        .into_par_iter()
        .zip(ids)
        .map(|((members, label), id)| {
            let mut sum = [0f32; DIM];
            for &i in &members {
                sum.iter_mut()
                    .zip(&model.vectors[i])
                    .for_each(|(s, x)| *s += x);
            }
            let words = top_words(model, &members);
            Cluster {
                id: id.unwrap_or(0),
                centre: normalised(sum),
                members,
                label,
                words,
            }
        })
        .collect();
    clusters.sort_by_key(|c| c.id);
    let mut of = vec![0; n];
    let mut loner = vec![false; n];
    for (c, cluster) in clusters.iter().enumerate() {
        for &i in &cluster.members {
            of[i] = c;
            loner[i] = cluster.members.len() > 1 && dot(&model.vectors[i], &cluster.centre) < LONER;
        }
    }
    Clustering {
        clusters,
        of,
        loner,
    }
}

/// A cluster's parts: its `members`, by their place in the model, split
/// by angle into about the root of a quarter of how many they are, at most
/// eight, none for fewer than twelve. Given the `centres` the parts had,
/// each member goes to the nearest of them, so that the parts hold from one
/// look to the next. Each part's members, in turn, and the centres.
pub fn parts(
    model: &Model,
    members: &[usize],
    centres: Option<&[Vector]>,
    seed: u64,
) -> (Vec<Vec<usize>>, Vec<Vector>) {
    let vectors: Vec<Vector> = members.iter().map(|&i| model.vectors[i]).collect();
    let k = if members.len() < 12 {
        1
    } else {
        ((members.len() as f64 / 4.0).sqrt().ceil() as usize).clamp(2, 8)
    };
    let (of, k) = match centres.filter(|c| !c.is_empty()) {
        Some(c) => {
            let near = |v: &Vector| {
                (0..c.len())
                    .max_by(|&a, &b| dot(v, &c[a]).total_cmp(&dot(v, &c[b])).then(b.cmp(&a)))
                    .unwrap_or(0)
            };
            (vectors.iter().map(near).collect(), c.len())
        }
        None => (kmeans(&vectors, k, Vec::new(), &mut Rng::new(seed)), k),
    };
    let mut parts = vec![Vec::new(); k];
    let mut sums = vec![[0f32; DIM]; k];
    for (j, &c) in of.iter().enumerate() {
        parts[c].push(members[j]);
        sums[c]
            .iter_mut()
            .zip(&vectors[j])
            .for_each(|(s, x)| *s += x);
    }
    let centres: Vec<Vector> = match centres.filter(|c| !c.is_empty()) {
        Some(c) => c.to_vec(),
        None => sums.into_iter().map(normalised).collect(),
    };
    // Parts that hardly differ, as mail a program sends, numbered, are one.
    let alike = centres
        .iter()
        .enumerate()
        .all(|(a, x)| centres[a + 1..].iter().all(|y| dot(x, y) > ALIKE));
    if k > 1 && alike {
        return (vec![members.to_vec()], Vec::new());
    }
    (parts, centres)
}

/// How near by angle the centres of a cluster's parts are, at least, to be
/// one part: within about 18°.
const ALIKE: f32 = 0.95;

/// What a part of a cluster is named by: the two words, or senders, most
/// its own, weighty in it and not as weighty in the rest of the cluster, of
/// `all` its members; not those the cluster is named by.
pub fn part_words(
    model: &Model,
    members: &[usize],
    all: &[usize],
    without: &[String],
) -> Vec<String> {
    let whole = weights(model, all);
    let mut words: Vec<(&str, f32)> = weights(model, members)
        .into_iter()
        .filter(|(w, _)| !w.is_empty() && !without.iter().any(|x| x == w))
        .map(|(w, x)| (w, x * x / whole.get(w).copied().unwrap_or(x).max(1e-6)))
        .collect();
    words.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    words
        .into_iter()
        .take(2)
        .map(|(w, _)| w.to_string())
        .collect()
}

/// What a cluster is named by: its three weightiest words, or, read by
/// the people lens, its three commonest senders.
fn top_words(model: &Model, members: &[usize]) -> Vec<String> {
    let mut words = ranked_words(model, members);
    words.truncate(3);
    words
}

/// The words of `members`, the weightiest first, or, read by the people
/// lens, their senders, the commonest first.
fn ranked_words(model: &Model, members: &[usize]) -> Vec<String> {
    let mut all: Vec<(&str, f32)> = weights(model, members)
        .into_iter()
        .filter(|(w, _)| !w.is_empty())
        .collect();
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    all.into_iter()
        .take(8)
        .map(|(w, _)| w.to_string())
        .collect()
}

/// What each word of `members` weighs in them all, or, read by the people
/// lens, how many each sender sent.
fn weights<'a>(model: &'a Model, members: &[usize]) -> HashMap<&'a str, f32> {
    let mut weight: HashMap<&str, f32> = HashMap::new();
    if model.lens == Lens::People {
        for &i in members {
            *weight.entry(model.senders[i].as_ref()).or_default() += 1.0;
        }
    } else {
        for &i in members {
            for (w, x) in model.words[i].iter() {
                *weight.entry(w.as_str()).or_default() += x;
            }
        }
    }
    weight
}

/// The width σ that makes a message's memberships in its neighbours,
/// `exp(−max(0, d − ρ)/σ)`, sum to `target`, found by halving: UMAP's
/// `smooth_knn_dist`.
fn width(dists: &[f32], rho: f32, target: f32) -> f32 {
    let mean = dists.iter().sum::<f32>() / dists.len().max(1) as f32;
    let (mut lo, mut hi, mut mid) = (0.0f32, f32::INFINITY, 1.0f32);
    for _ in 0..64 {
        let sum: f32 = dists
            .iter()
            .map(|&d| {
                if d > rho {
                    (-(d - rho) / mid).exp()
                } else {
                    1.0
                }
            })
            .sum();
        if (sum - target).abs() < 1e-5 {
            break;
        }
        if sum > target {
            hi = mid;
            mid = (lo + hi) / 2.0;
        } else {
            lo = mid;
            mid = if hi.is_finite() {
                (lo + hi) / 2.0
            } else {
                mid * 2.0
            };
        }
    }
    mid.max(1e-3 * mean)
}

/// A message's membership in each of its neighbours, from their distances:
/// `exp(−(d − ρ)/σ)`, ρ the nearest distance and σ set so that they sum to
/// `log₂(k + 1)`, as UMAP counts the message itself among its neighbours.
fn memberships(dists: &[f32]) -> Vec<f32> {
    let Some(rho) = dists.iter().copied().reduce(f32::min) else {
        return Vec::new();
    };
    let sigma = width(dists, rho, ((dists.len() + 1) as f32).log2());
    dists
        .iter()
        .map(|&d| {
            if d > rho {
                (-(d - rho) / sigma).exp()
            } else {
                1.0
            }
        })
        .collect()
}

/// The fuzzy graph UMAP lays out, from each message's neighbours: each
/// pair once, `i < j`, weighed by the chance that either lists the other,
/// `a + b − a·b`.
fn graph(near: &[Vec<(usize, f32)>]) -> Vec<(usize, usize, f32)> {
    let directed: Vec<Vec<f32>> = near
        .par_iter()
        .map(|list| memberships(&list.iter().map(|&(_, d)| d).collect::<Vec<_>>()))
        .collect();
    let mut pairs: BTreeMap<(usize, usize), (f32, f32)> = BTreeMap::new();
    for (i, list) in near.iter().enumerate() {
        for (&(j, _), &w) in list.iter().zip(&directed[i]) {
            let pair = pairs.entry((i.min(j), i.max(j))).or_insert((0.0, 0.0));
            if i < j {
                pair.0 = w;
            } else {
                pair.1 = w;
            }
        }
    }
    pairs
        .into_iter()
        .filter_map(|((i, j), (a, b))| {
            let w = a + b - a * b;
            (w > 0.0).then_some((i, j, w))
        })
        .collect()
}

/// UMAP (McInnes, Healy and Melville, 2018) of the graph `edges` over the
/// points begun at `start`, with t-UMAP's kernel `1/(1 + d²)`, which spreads
/// the map more than UMAP's own and suits a grid of cells. Each edge is
/// drawn as often as its weight asks, from each end in turn: that end is
/// drawn toward the other and pushed from `NEGATIVE` others taken at
/// random. Within a round each point moves itself alone, against where the
/// others were as the round began, so the points are taken in parallel and
/// the map is the same, bit for bit, on any number of threads.
fn umap(
    edges: &[(usize, usize, f32)],
    start: Vec<Point>,
    held: Option<&[bool]>,
    rounds: usize,
    rate: f32,
    seed: u64,
) -> Vec<Point> {
    let n = start.len();
    let top = edges.iter().map(|e| e.2).fold(1e-4f32, f32::max);
    // Each point's half of each of its edges: the other end, and how many
    // rounds apart it is drawn.
    let mut ends: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    for &(i, j, w) in edges {
        ends[i].push((j, top / w));
        ends[j].push((i, top / w));
    }
    let mut due: Vec<Vec<f32>> = ends
        .iter()
        .map(|e| e.iter().map(|&(_, every)| every).collect())
        .collect();
    let mut y = start;
    for round in 0..rounds {
        let now = round as f32;
        let alpha = rate * (1.0 - now / rounds as f32);
        let before = y.clone();
        y.par_iter_mut()
            .zip(due.par_iter_mut())
            .enumerate()
            .for_each(|(i, (p, due))| {
                if held.is_some_and(|h| h[i]) {
                    return;
                }
                let mut rng = Rng::new(
                    seed ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ ((round as u64) << 40),
                );
                for (e, &(j, every)) in ends[i].iter().enumerate() {
                    if due[e] > now {
                        continue;
                    }
                    pull(p, &before[j], alpha);
                    for _ in 0..NEGATIVE {
                        let k = rng.below(n);
                        if k != i {
                            push(p, &before[k], alpha);
                        }
                    }
                    due[e] += every;
                }
            });
    }
    if held.is_some() { y } else { centred(y) }
}

/// A step of `p` toward `q`, by t-UMAP's attraction, each part held to 4.
fn pull(p: &mut Point, q: &Point, alpha: f32) {
    let (dx, dy) = (p[0] - q[0], p[1] - q[1]);
    let d2 = dx * dx + dy * dy;
    if d2 <= 0.0 {
        return;
    }
    let f = -2.0 / (1.0 + d2);
    p[0] += (f * dx).clamp(-4.0, 4.0) * alpha;
    p[1] += (f * dy).clamp(-4.0, 4.0) * alpha;
}

/// A step of `p` away from `q`, by t-UMAP's repulsion, each part held to 4.
fn push(p: &mut Point, q: &Point, alpha: f32) {
    let (dx, dy) = (p[0] - q[0], p[1] - q[1]);
    let d2 = dx * dx + dy * dy;
    let f = if d2 > 0.0 {
        2.0 / ((0.001 + d2) * (1.0 + d2))
    } else {
        4.0
    };
    p[0] += (f * dx).clamp(-4.0, 4.0) * alpha;
    p[1] += (f * dy).clamp(-4.0, 4.0) * alpha;
}

/// `points` moved to have their mean at the middle.
fn centred(mut points: Vec<Point>) -> Vec<Point> {
    let n = points.len().max(1) as f32;
    let mean = points
        .iter()
        .fold([0f32; 2], |m, p| [m[0] + p[0], m[1] + p[1]]);
    for p in &mut points {
        p[0] -= mean[0] / n;
        p[1] -= mean[1] / n;
    }
    points
}

/// `points` centred and scaled so that the farthest part of any is 10
/// from the middle, as UMAP begins.
fn spread(points: Vec<Point>) -> Vec<Point> {
    let mut points = centred(points);
    let far = points
        .iter()
        .flat_map(|p| [p[0].abs(), p[1].abs()])
        .fold(0f32, f32::max);
    if far > 0.0 {
        for p in &mut points {
            p[0] *= 10.0 / far;
            p[1] *= 10.0 / far;
        }
    }
    points
}

/// Where the model's messages are on the map, laid out by UMAP over each
/// one's nearest by angle. Begun from `start`, each message's old place
/// where it had one and a place among its nearest where not, gently: fewer
/// rounds, smaller steps. Else begun from the first two principal
/// components, a little shaken so that the same mail twice comes apart.
pub fn layout(model: &Model, start: Option<&[Option<Point>]>, seed: u64) -> Vec<Point> {
    let n = model.len();
    if n < 2 {
        return vec![[0.0, 0.0]; n];
    }
    let seed = seed ^ model.lens as u64;
    let edges = graph(&model.ties());
    let (begun, rounds, rate) = match start {
        Some(s) => {
            let begun = (0..n)
                .map(|i| {
                    s.get(i)
                        .copied()
                        .flatten()
                        .unwrap_or_else(|| place(model, i, s))
                })
                .collect();
            (begun, 120, 0.25)
        }
        None => {
            let mut rng = Rng::new(seed);
            let begun = model
                .vectors
                .iter()
                .map(|v| model.axes.coordinates(v))
                .map(|[a, b]| {
                    [
                        (a + 1e-4 * rng.normal()) as f32,
                        (b + 1e-4 * rng.normal()) as f32,
                    ]
                })
                .collect();
            (begun, if n <= 10_000 { 500 } else { 200 }, 1.0)
        }
    };
    umap(&edges, spread(begun), None, rounds, rate, seed)
}

/// The messages `new` settled among the rest of `at`, which stay where
/// they are: each begun where `place` puts it, then drawn toward its
/// nearest and pushed from others, as UMAP lays them out, the rest held.
pub fn settle(model: &Model, at: &mut [Point], new: &[usize], seed: u64) {
    if new.is_empty() || at.len() < 2 {
        return;
    }
    let known: Vec<Option<Point>> = {
        let mut known: Vec<Option<Point>> = at.iter().copied().map(Some).collect();
        for &i in new {
            known[i] = None;
        }
        known
    };
    for &i in new {
        at[i] = place(model, i, &known);
    }
    let ties = model.ties();
    let mut edges = Vec::new();
    for &i in new {
        let dists: Vec<f32> = ties[i].iter().map(|&(_, d)| d).collect();
        for (&(j, _), w) in ties[i].iter().zip(memberships(&dists)) {
            if w > 0.0 {
                edges.push((i, j, w));
            }
        }
    }
    let mut held = vec![true; at.len()];
    for &i in new {
        held[i] = false;
    }
    let moved = umap(
        &edges,
        at.to_vec(),
        Some(&held),
        SETTLE,
        0.5,
        seed ^ model.lens as u64,
    );
    at.copy_from_slice(&moved);
}

/// Where the message `i` goes among those placed, `at`, by message: the
/// mean of the places of its nearest by tokens, each by how near; or, if
/// none of them is placed, of its five nearest placed by angle.
pub fn place(model: &Model, i: usize, at: &[Option<Point>]) -> Point {
    let mut near: Vec<(f32, Point)> = model.near[i]
        .iter()
        .filter_map(|&(j, d)| Some((1.0 - d, at.get(j).copied().flatten()?)))
        .collect();
    if near.is_empty() {
        near = (0..at.len().min(model.len()))
            .filter(|&j| j != i)
            .filter_map(|j| Some((dot(&model.vectors[i], &model.vectors[j]), at[j]?)))
            .collect();
        near.sort_by(|a, b| b.0.total_cmp(&a.0));
        near.truncate(5);
    }
    let total: f32 = near.iter().map(|(c, _)| c.max(0.0) + 1e-3).sum();
    if near.is_empty() || total <= 0.0 {
        return [0.0, 0.0];
    }
    let mut at = [0f32; 2];
    for (c, p) in &near {
        let w = (c.max(0.0) + 1e-3) / total;
        at[0] += w * p[0];
        at[1] += w * p[1];
    }
    at
}

/// The rotation, reflection, scale and shift that lays `new` best onto
/// `old`, point by point, as Procrustes found it; applied to all of
/// `points`.
pub fn procrustes(new: &[Point], old: &[Point], points: &[Point]) -> Vec<Point> {
    let n = new.len().min(old.len());
    if n < 2 {
        return points.to_vec();
    }
    let mean = |s: &[Point]| {
        let mut m = [0f64; 2];
        for p in &s[..n] {
            m[0] += f64::from(p[0]) / n as f64;
            m[1] += f64::from(p[1]) / n as f64;
        }
        m
    };
    let (ma, mb) = (mean(new), mean(old));
    let (mut sxx, mut sxy, mut syx, mut syy, mut norm) = (0f64, 0f64, 0f64, 0f64, 0f64);
    for k in 0..n {
        let a = [f64::from(new[k][0]) - ma[0], f64::from(new[k][1]) - ma[1]];
        let b = [f64::from(old[k][0]) - mb[0], f64::from(old[k][1]) - mb[1]];
        sxx += a[0] * b[0];
        sxy += a[0] * b[1];
        syx += a[1] * b[0];
        syy += a[1] * b[1];
        norm += a[0] * a[0] + a[1] * a[1];
    }
    if norm <= 0.0 {
        return points.to_vec();
    }
    // A rotation by `t`, or a reflection, whichever lays them closer.
    let turn = (sxy - syx).atan2(sxx + syy);
    let fit_turn = (sxx + syy) * turn.cos() + (sxy - syx) * turn.sin();
    let mirror = (sxy + syx).atan2(sxx - syy);
    let fit_mirror = (sxx - syy) * mirror.cos() + (sxy + syx) * mirror.sin();
    let (c, s, flip, fit) = if fit_mirror > fit_turn {
        (mirror.cos(), mirror.sin(), true, fit_mirror)
    } else {
        (turn.cos(), turn.sin(), false, fit_turn)
    };
    let scale = fit / norm;
    points
        .iter()
        .map(|p| {
            let (x, y) = (f64::from(p[0]) - ma[0], f64::from(p[1]) - ma[1]);
            let y = if flip { -y } else { y };
            let rx = c * x - s * y;
            let ry = s * x + c * y;
            [(scale * rx + mb[0]) as f32, (scale * ry + mb[1]) as f32]
        })
        .collect()
}

/// The plane of the first two principal components of some directions,
/// and their mean: where hues are read, and the map begun.
#[derive(Clone, Debug)]
pub struct Axes {
    mean: [f64; DIM],
    components: [[f64; DIM]; 2],
}

impl Axes {
    /// The axes of `vectors`, found by power iteration, each with a sign
    /// of its own. The products are summed a block at a time in parallel
    /// and the blocks in turn, so they are the same on any number of
    /// threads.
    pub fn of(vectors: &[Vector]) -> Axes {
        let n = vectors.len().max(1);
        let mut mean = [0f64; DIM];
        for v in vectors {
            for d in 0..DIM {
                mean[d] += f64::from(v[d]) / n as f64;
            }
        }
        let centred: Vec<[f64; DIM]> = vectors
            .iter()
            .map(|v| {
                let mut c = [0f64; DIM];
                for d in 0..DIM {
                    c[d] = f64::from(v[d]) - mean[d];
                }
                c
            })
            .collect();
        let times = |x: &[f64; DIM]| {
            let blocks: Vec<[f64; DIM]> = centred
                .par_chunks(1024)
                .map(|block| {
                    let mut out = [0f64; DIM];
                    for c in block {
                        let p: f64 = c.iter().zip(x).map(|(a, b)| a * b).sum();
                        for d in 0..DIM {
                            out[d] += p * c[d];
                        }
                    }
                    out
                })
                .collect();
            let mut out = [0f64; DIM];
            for block in &blocks {
                for d in 0..DIM {
                    out[d] += block[d];
                }
            }
            out
        };
        let unit = |mut x: [f64; DIM]| {
            let norm: f64 = x.iter().map(|a| a * a).sum::<f64>().sqrt();
            if norm > 0.0 {
                x.iter_mut().for_each(|a| *a /= norm);
            }
            x
        };
        let mut rng = Rng::new(7);
        let mut components = [[0f64; DIM]; 2];
        for c in 0..2 {
            let mut x = [0f64; DIM];
            x.iter_mut().for_each(|a| *a = rng.normal());
            for _ in 0..100 {
                let mut next = times(&x);
                for done in &components[..c] {
                    let p: f64 = next.iter().zip(done).map(|(a, b)| a * b).sum();
                    for d in 0..DIM {
                        next[d] -= p * done[d];
                    }
                }
                x = unit(next);
            }
            // A sign of its own, the same whatever the start: its largest
            // part is positive.
            let big = (0..DIM)
                .max_by(|&a, &b| x[a].abs().total_cmp(&x[b].abs()))
                .unwrap_or(0);
            if x[big] < 0.0 {
                x.iter_mut().for_each(|a| *a = -*a);
            }
            components[c] = x;
        }
        Axes { mean, components }
    }

    /// `v` in the plane.
    fn coordinates(&self, v: &Vector) -> [f64; 2] {
        let along = |c: &[f64; DIM]| -> f64 {
            (0..DIM)
                .map(|d| (f64::from(v[d]) - self.mean[d]) * c[d])
                .sum()
        };
        [along(&self.components[0]), along(&self.components[1])]
    }

    /// The hue of `v`, in degrees: its angle in the plane.
    pub fn hue(&self, v: &Vector) -> f32 {
        let [a, b] = self.coordinates(v);
        (b.atan2(a).to_degrees().rem_euclid(360.0)) as f32
    }
}

/// How faithful the map is to the mail: of each of up to 200 messages
/// taken evenly, the share of its ten nearest by tokens also among its ten
/// nearest on the map; their mean.
pub fn faithfulness(model: &Model, at: &[Point]) -> f32 {
    let n = model.len().min(at.len());
    if n < 3 {
        return 1.0;
    }
    let k = 10.min(n - 1);
    let stride = n.div_ceil(200).max(1);
    let shares: Vec<f32> = (0..n)
        .step_by(stride)
        .collect::<Vec<_>>()
        .into_par_iter()
        .filter_map(|i| {
            let by_tokens: BTreeSet<usize> =
                model.near[i].iter().take(k).map(|&(j, _)| j).collect();
            if by_tokens.is_empty() {
                return None;
            }
            let by = |a: &(f32, usize), b: &(f32, usize)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
            let mut by_place: Vec<(f32, usize)> = (0..n)
                .filter(|&j| j != i)
                .map(|j| {
                    let (dx, dy) = (at[i][0] - at[j][0], at[i][1] - at[j][1]);
                    (dx * dx + dy * dy, j)
                })
                .collect();
            if by_place.len() > k {
                by_place.select_nth_unstable_by(k, by);
                by_place.truncate(k);
            }
            let shared = by_place
                .iter()
                .filter(|(_, j)| by_tokens.contains(j))
                .count();
            Some(shared as f32 / by_tokens.len() as f32)
        })
        .collect();
    if shares.is_empty() {
        1.0
    } else {
        shares.iter().sum::<f32>() / shares.len() as f32
    }
}
