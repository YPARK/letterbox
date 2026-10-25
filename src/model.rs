//! The map's model, as arithmetic alone: no file, no server, no clock. A
//! message is read as tokens, its sender, the sender's domain, its list,
//! whom else it went to, and the words of its subject and of its start,
//! and becomes a direction in 64 dimensions: each token hashed by FNV-1a
//! to a bucket, each bucket a vector of signs made from its hash and a
//! fixed seed, weighted by how rare the bucket is, summed and made of
//! unit length. Labels teach it: a label is the mean of its messages, a
//! message not yet labelled is pulled toward a label by the share of its
//! tokens seen in that label's mail, and pushed away from one it was said
//! not to be. Clusters are found by angle, a cluster holding two labels'
//! mail is split between them, and the map is laid out by UMAP over each
//! message's nearest by angle. Every step is seeded, so the same mail gives
//! the same map, bit for bit, however many threads it is made on.

use crate::index::record::{Record, fnv};
use crate::index::{Key, Lens, fold};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
/// Up to how many messages their neighbours are found by comparing every
/// pair; past it, each is compared only with those of its nearest groups.
pub const EXACT: usize = 6000;
/// How many of its nearest groups a message is compared within, past
/// `EXACT`.
const PROBES: usize = 4;
/// How many others each end of an edge is pushed from, each time the edge
/// is drawn.
const NEGATIVE: usize = 5;
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

/// The messages, as the model has them, for one lens.
pub struct Model {
    pub lens: Lens,
    pub keys: Vec<Key>,
    /// Each message's direction, labels' teaching taken in.
    pub vectors: Vec<Vector>,
    /// Each message's words, by what they weigh in it: to name clusters by.
    pub words: Vec<Vec<(String, f32)>>,
    /// Each message's sender, as named.
    pub senders: Vec<String>,
    /// Each message's labels, as given.
    pub labels: Vec<Vec<String>>,
    /// Each label, folded, its name as first given, and the mean of its
    /// messages.
    pub label_vectors: BTreeMap<String, (String, Vector)>,
}

/// One message, as the model is given it: its record, its labels, and
/// whether it was sent from here.
pub type Given<'a> = (&'a Record, Vec<String>, bool);

impl Model {
    /// The model of `messages`, in the order given (the index's, by key),
    /// as `lens` reads them, taught by their labels and by `not_it`, each
    /// message and label, folded, it was said not to be.
    pub fn build(messages: &[Given], not_it: &BTreeSet<(Key, String)>, lens: Lens) -> Model {
        let tokens: Vec<Vec<(u64, String, f32)>> = messages
            .iter()
            .map(|(r, _, sent)| {
                tokens(r, *sent, lens)
                    .into_iter()
                    .map(|(t, w)| (bucket(&t), t, w))
                    .collect()
            })
            .collect();
        let mut df: HashMap<u64, u32> = HashMap::new();
        for t in &tokens {
            let mut seen = BTreeSet::new();
            for (b, _, _) in t {
                if seen.insert(*b) {
                    *df.entry(*b).or_default() += 1;
                }
            }
        }
        let n = messages.len() as f32;
        let idf = |b: u64| ((n + 1.0) / (df.get(&b).copied().unwrap_or(0) as f32 + 1.0)).ln() + 1.0;
        // What a token weighs: by how rare it is. A token of one message
        // alone tells it from none, and one of most tells none apart:
        // either is only noise, and weighs nothing.
        let weigh = |b: u64, w: f32| {
            let seen = df.get(&b).copied().unwrap_or(0) as f32;
            let telling = n < 20.0 || (seen > 1.0 && seen <= n / 2.0);
            if telling { w * idf(b) } else { 0.0 }
        };
        let mut vectors: Vec<Vector> = tokens
            .iter()
            .map(|t| {
                let mut v = [0f32; DIM];
                for &(b, _, w) in t {
                    let mut state = b ^ SEED;
                    let signs = splitmix(&mut state);
                    let weight = weigh(b, w);
                    for (d, x) in v.iter_mut().enumerate() {
                        *x += if signs >> d & 1 == 1 { weight } else { -weight };
                    }
                }
                normalised(v)
            })
            .collect();
        let words: Vec<Vec<(String, f32)>> = tokens
            .iter()
            .map(|t| {
                t.iter()
                    .filter_map(|(b, s, w)| Some((s.strip_prefix("w:")?.to_string(), w * idf(*b))))
                    .collect()
            })
            .collect();
        // What each label is: the mean of its messages, and the tokens seen
        // in them, by how rare each is.
        let mut members: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
        for (i, (_, labels, _)) in messages.iter().enumerate() {
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
                    .flat_map(|&i| tokens[i].iter().map(|t| t.0))
                    .collect(),
            );
        }
        for (i, (record, labels, _)) in messages.iter().enumerate() {
            let mut v = vectors[i];
            let mine: BTreeSet<String> = labels.iter().map(|l| fold(l)).collect();
            if mine.is_empty() {
                let total: f32 = tokens[i].iter().map(|t| weigh(t.0, t.2)).sum();
                for (folded, seen) in &label_tokens {
                    let shared: f32 = tokens[i]
                        .iter()
                        .filter(|t| seen.contains(&t.0))
                        .map(|t| weigh(t.0, t.2))
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
                if not_it.contains(&(record.key.clone(), folded.clone())) {
                    v.iter_mut().zip(l).for_each(|(x, y)| *x -= PUSH * y);
                }
            }
            vectors[i] = normalised(v);
        }
        Model {
            lens,
            keys: messages.iter().map(|(r, _, _)| r.key.clone()).collect(),
            vectors,
            words,
            // Mail sent from here names no cluster by its sender.
            senders: messages
                .iter()
                .map(|(r, _, sent)| if *sent { String::new() } else { r.from.clone() })
                .collect(),
            labels: messages.iter().map(|(_, l, _)| l.clone()).collect(),
            label_vectors,
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
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
/// `start` where given, else by k-means++ with a fixed seed.
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
    while centres.len() < k {
        let far: Vec<f64> = vectors
            .iter()
            .map(|v| {
                let near = centres.iter().map(|c| dot(v, c)).fold(f32::MIN, f32::max);
                f64::from((1.0 - near).max(0.0)).powi(2)
            })
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
        let mut moved = false;
        for (i, v) in vectors.iter().enumerate() {
            let best = (0..k)
                .max_by(|&a, &b| {
                    dot(v, &centres[a])
                        .total_cmp(&dot(v, &centres[b]))
                        .then(b.cmp(&a))
                })
                .unwrap_or(0);
            if of[i] != best {
                of[i] = best;
                moved = true;
            }
        }
        if !moved {
            break;
        }
        let mut sums = vec![[0f32; DIM]; k];
        let mut counts = vec![0usize; k];
        for (i, v) in vectors.iter().enumerate() {
            sums[of[i]].iter_mut().zip(v).for_each(|(s, x)| *s += x);
            counts[of[i]] += 1;
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
        .into_iter()
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

/// What a cluster is named by: its three weightiest words, or, read by
/// the people lens, its three commonest senders.
fn top_words(model: &Model, members: &[usize]) -> Vec<String> {
    let mut weight: BTreeMap<&str, f32> = BTreeMap::new();
    if model.lens == Lens::People {
        for &i in members {
            *weight.entry(model.senders[i].as_str()).or_default() += 1.0;
        }
    } else {
        for &i in members {
            for (w, x) in &model.words[i] {
                *weight.entry(w.as_str()).or_default() += x;
            }
        }
    }
    let mut all: Vec<(&str, f32)> = weight.into_iter().filter(|(w, _)| !w.is_empty()).collect();
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    all.into_iter()
        .take(3)
        .map(|(w, _)| w.to_string())
        .collect()
}

/// Each of `vectors`' `k` nearest others by angle, as (which, distance),
/// the distance `1 − cos`, nearest first. Up to `exact` vectors, every
/// pair is compared; past that, each vector only with the members of its
/// `PROBES` nearest of √n groups, found by a few rounds of spherical
/// k-means begun evenly through them. Ties go to the lower index, so the
/// same vectors give the same neighbours on any number of threads.
pub fn neighbours(vectors: &[Vector], k: usize, exact: usize) -> Vec<Vec<(usize, f32)>> {
    let n = vectors.len();
    let k = k.min(n.saturating_sub(1));
    if k == 0 {
        return vec![Vec::new(); n];
    }
    if n <= exact {
        return (0..n)
            .into_par_iter()
            .map(|i| nearest_among(vectors, i, 0..n, k))
            .collect();
    }
    let m = ((n as f64).sqrt().ceil() as usize).max(PROBES);
    let mut centres: Vec<Vector> = (0..m).map(|c| vectors[c * n / m]).collect();
    let nearest_centre = |v: &Vector, centres: &[Vector]| {
        (0..centres.len())
            .max_by(|&a, &b| {
                dot(v, &centres[a])
                    .total_cmp(&dot(v, &centres[b]))
                    .then(b.cmp(&a))
            })
            .unwrap_or(0)
    };
    let mut of = vec![0usize; n];
    for _ in 0..4 {
        of = vectors
            .par_iter()
            .map(|v| nearest_centre(v, &centres))
            .collect();
        let mut sums = vec![[0f32; DIM]; m];
        let mut counts = vec![0usize; m];
        for (v, &c) in vectors.iter().zip(&of) {
            sums[c].iter_mut().zip(v).for_each(|(s, x)| *s += x);
            counts[c] += 1;
        }
        for c in 0..m {
            if counts[c] > 0 {
                centres[c] = normalised(sums[c]);
            }
        }
    }
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); m];
    for (i, &c) in of.iter().enumerate() {
        members[c].push(i);
    }
    (0..n)
        .into_par_iter()
        .map(|i| {
            let mut by: Vec<(usize, f32)> = centres
                .iter()
                .enumerate()
                .map(|(c, centre)| (c, dot(&vectors[i], centre)))
                .collect();
            by.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let among = by
                .iter()
                .take(PROBES)
                .flat_map(|&(c, _)| members[c].iter().copied());
            nearest_among(vectors, i, among, k)
        })
        .collect()
}

/// The `k` nearest to `vectors[i]` of those `among`, by angle.
fn nearest_among(
    vectors: &[Vector],
    i: usize,
    among: impl Iterator<Item = usize>,
    k: usize,
) -> Vec<(usize, f32)> {
    let by = |a: &(usize, f32), b: &(usize, f32)| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0));
    let mut all: Vec<(usize, f32)> = among
        .filter(|&j| j != i)
        .map(|j| (j, (1.0 - dot(&vectors[i], &vectors[j])).max(0.0)))
        .collect();
    if all.len() > k {
        all.select_nth_unstable_by(k, by);
        all.truncate(k);
    }
    all.sort_by(by);
    all
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
    centred(y)
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
    let edges = graph(&neighbours(&model.vectors, NEIGHBOURS, EXACT));
    let (begun, rounds, rate) = match start {
        Some(s) => {
            let known: Vec<(usize, Point)> = (0..n)
                .filter_map(|i| Some((i, s.get(i).copied().flatten()?)))
                .collect();
            let begun = (0..n)
                .map(|i| {
                    s.get(i)
                        .copied()
                        .flatten()
                        .unwrap_or_else(|| place(model, i, &known))
                })
                .collect();
            (begun, 120, 0.25)
        }
        None => {
            let mut rng = Rng::new(seed);
            let begun = principal(&model.vectors)
                .into_iter()
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
    umap(&edges, spread(begun), rounds, rate, seed)
}

/// Where the message `i` goes among those `known`: the mean of the places
/// of its five nearest by angle, each by how near it is.
pub fn place(model: &Model, i: usize, known: &[(usize, Point)]) -> Point {
    let mut near: Vec<(f32, Point)> = known
        .iter()
        .filter(|(j, _)| *j != i)
        .map(|&(j, p)| (dot(&model.vectors[i], &model.vectors[j]), p))
        .collect();
    near.sort_by(|a, b| b.0.total_cmp(&a.0));
    near.truncate(5);
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

/// Each message's hue, in degrees: the angle of its direction in the
/// plane of the first two principal components of them all.
pub fn hues(vectors: &[Vector]) -> Vec<f32> {
    principal(vectors)
        .into_iter()
        .map(|[a, b]| (b.atan2(a).to_degrees().rem_euclid(360.0)) as f32)
        .collect()
}

/// Each of `vectors` in the plane of their first two principal components,
/// found by power iteration, each with a sign of its own. The products are
/// summed a block at a time in parallel and the blocks in turn, so the
/// result is the same on any number of threads.
fn principal(vectors: &[Vector]) -> Vec<[f64; 2]> {
    let n = vectors.len();
    if n == 0 {
        return Vec::new();
    }
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
    let mut components: Vec<[f64; DIM]> = Vec::new();
    for _ in 0..2 {
        let mut x = [0f64; DIM];
        x.iter_mut().for_each(|a| *a = rng.normal());
        for _ in 0..100 {
            let mut next = times(&x);
            for c in &components {
                let p: f64 = next.iter().zip(c).map(|(a, b)| a * b).sum();
                for d in 0..DIM {
                    next[d] -= p * c[d];
                }
            }
            x = unit(next);
        }
        // A sign of its own, the same whatever the start: its largest part
        // is positive.
        let big = (0..DIM)
            .max_by(|&a, &b| x[a].abs().total_cmp(&x[b].abs()))
            .unwrap_or(0);
        if x[big] < 0.0 {
            x.iter_mut().for_each(|a| *a = -*a);
        }
        components.push(x);
    }
    centred
        .iter()
        .map(|c| {
            let a: f64 = c.iter().zip(&components[0]).map(|(x, y)| x * y).sum();
            let b: f64 = c.iter().zip(&components[1]).map(|(x, y)| x * y).sum();
            [a, b]
        })
        .collect()
}

/// How faithful the map is to the angles: of each of up to 200 messages
/// taken evenly, the share of its ten nearest by angle also among its ten
/// nearest on the map; their mean.
pub fn faithfulness(model: &Model, at: &[Point]) -> f32 {
    let n = model.len().min(at.len());
    if n < 3 {
        return 1.0;
    }
    let k = 10.min(n - 1);
    let stride = n.div_ceil(200).max(1);
    let mut total = 0.0;
    let mut count = 0;
    for i in (0..n).step_by(stride) {
        let by_angle: BTreeSet<usize> = model.nearest(i, k).into_iter().map(|(j, _)| j).collect();
        let mut by_place: Vec<(f32, usize)> = (0..n)
            .filter(|&j| j != i)
            .map(|j| {
                let (dx, dy) = (at[i][0] - at[j][0], at[i][1] - at[j][1]);
                (dx * dx + dy * dy, j)
            })
            .collect();
        by_place.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let shared = by_place
            .iter()
            .take(k)
            .filter(|(_, j)| by_angle.contains(j))
            .count();
        total += shared as f32 / k as f32;
        count += 1;
    }
    total / count as f32
}
