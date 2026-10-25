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
//! mail is split between them, and the map is laid out by t-SNE. Every
//! step is seeded, so the same mail gives the same map, bit for bit.

use crate::index::record::{Record, fnv};
use crate::index::{Key, Lens, fold};
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
/// How many messages are laid out exactly; more, and these many are, the
/// rest placed by them.
pub const LANDMARKS: usize = 3000;
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

/// A message's tokens, and what each weighs, as `lens` reads it.
pub fn tokens(r: &Record, lens: Lens) -> Vec<(String, f32)> {
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
        if !r.address.is_empty() {
            add(format!("from:{}", r.address), 2.5);
            if let Some((_, domain)) = r.address.rsplit_once('@') {
                add(format!("dom:{domain}"), 1.0);
            }
        }
        for a in r.to.iter().chain(&r.cc) {
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

/// One message, as the model is given it: its record and its labels.
pub type Given<'a> = (&'a Record, Vec<String>);

impl Model {
    /// The model of `messages`, in the order given (the index's, by key),
    /// as `lens` reads them, taught by their labels and by `not_it`, each
    /// message and label, folded, it was said not to be.
    pub fn build(messages: &[Given], not_it: &BTreeSet<(Key, String)>, lens: Lens) -> Model {
        let tokens: Vec<Vec<(u64, String, f32)>> = messages
            .iter()
            .map(|(r, _)| {
                tokens(r, lens)
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
        for (i, (_, labels)) in messages.iter().enumerate() {
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
        for (i, (record, labels)) in messages.iter().enumerate() {
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
            keys: messages.iter().map(|(r, _)| r.key.clone()).collect(),
            vectors,
            words,
            senders: messages.iter().map(|(r, _)| r.from.clone()).collect(),
            labels: messages.iter().map(|(_, l)| l.clone()).collect(),
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

/// t-SNE (van der Maaten and Hinton, 2008) of `n` points, `far(i, j)`
/// apart, in two dimensions: begun from `start` where given, gently, else
/// from points near nothing drawn with the seed, with the clusters drawn
/// apart early on.
pub fn tsne(
    n: usize,
    far: impl Fn(usize, usize) -> f32,
    start: Option<Vec<Point>>,
    seed: u64,
) -> Vec<Point> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![[0.0, 0.0]];
    }
    let perplexity = (30.0f64).min((n as f64 - 1.0) / 6.0).max(2.0);
    // The neighbours each point has, by a width found for each.
    let mut p = vec![0f64; n * n];
    for i in 0..n {
        let d: Vec<f64> = (0..n).map(|j| f64::from(far(i, j))).collect();
        let (mut lo, mut hi, mut beta) = (0.0f64, f64::INFINITY, 1.0f64);
        let target = perplexity.ln();
        for _ in 0..64 {
            let mut sum = 0.0;
            let mut sum_dp = 0.0;
            for j in 0..n {
                if j != i {
                    let e = (-beta * d[j]).exp();
                    p[i * n + j] = e;
                    sum += e;
                    sum_dp += d[j] * e;
                }
            }
            if sum <= 0.0 {
                hi = beta;
                beta = (beta + lo) / 2.0;
                continue;
            }
            let entropy = sum.ln() + beta * sum_dp / sum;
            for j in 0..n {
                p[i * n + j] /= sum;
            }
            if (entropy - target).abs() < 1e-5 {
                break;
            }
            if entropy > target {
                lo = beta;
                beta = if hi.is_finite() {
                    (beta + hi) / 2.0
                } else {
                    beta * 2.0
                };
            } else {
                hi = beta;
                beta = (beta + lo) / 2.0;
            }
        }
        p[i * n + i] = 0.0;
    }
    let mut sym = vec![0f64; n * n];
    for i in 0..n {
        for j in 0..n {
            sym[i * n + j] = ((p[i * n + j] + p[j * n + i]) / (2.0 * n as f64)).max(1e-12);
        }
    }
    let gentle = start.is_some();
    let mut rng = Rng::new(seed);
    let mut y: Vec<[f64; 2]> = match start {
        Some(s) => s
            .iter()
            .map(|q| [f64::from(q[0]), f64::from(q[1])])
            .collect(),
        None => (0..n)
            .map(|_| [rng.normal() * 1e-2, rng.normal() * 1e-2])
            .collect(),
    };
    let rounds = if gentle { 300 } else { 600 };
    let rate = (n as f64 / 12.0).max(50.0);
    let mut step = vec![[0f64; 2]; n];
    let mut gains = vec![[1f64; 2]; n];
    let mut q = vec![0f64; n * n];
    for round in 0..rounds {
        let exaggerate = if !gentle && round < 120 { 12.0 } else { 1.0 };
        let momentum = if round < 250 { 0.5 } else { 0.8 };
        let mut z = 0.0;
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = y[i][0] - y[j][0];
                let dy = y[i][1] - y[j][1];
                let w = 1.0 / (1.0 + dx * dx + dy * dy);
                q[i * n + j] = w;
                q[j * n + i] = w;
                z += 2.0 * w;
            }
        }
        for i in 0..n {
            let mut g = [0f64; 2];
            for j in 0..n {
                if i == j {
                    continue;
                }
                let w = q[i * n + j];
                let f = (exaggerate * sym[i * n + j] - w / z) * w;
                g[0] += 4.0 * f * (y[i][0] - y[j][0]);
                g[1] += 4.0 * f * (y[i][1] - y[j][1]);
            }
            for d in 0..2 {
                gains[i][d] = if (g[d] > 0.0) == (step[i][d] > 0.0) {
                    (gains[i][d] * 0.8).max(0.01)
                } else {
                    gains[i][d] + 0.2
                };
                step[i][d] = momentum * step[i][d] - rate * gains[i][d] * g[d];
            }
        }
        let mut mean = [0f64; 2];
        for i in 0..n {
            y[i][0] += step[i][0];
            y[i][1] += step[i][1];
            mean[0] += y[i][0] / n as f64;
            mean[1] += y[i][1] / n as f64;
        }
        for point in &mut y {
            point[0] -= mean[0];
            point[1] -= mean[1];
        }
    }
    y.into_iter().map(|q| [q[0] as f32, q[1] as f32]).collect()
}

/// Where the model's messages are on the map, laid out by t-SNE on their
/// angles: all of them, or past `LANDMARKS`, that many, evenly through
/// them, and the rest each placed among its nearest of those. Begun from
/// `start`, each message's old place where it had one.
pub fn layout(model: &Model, start: Option<&[Option<Point>]>, seed: u64) -> Vec<Point> {
    let n = model.len();
    let marks: Vec<usize> = if n <= LANDMARKS {
        (0..n).collect()
    } else {
        (0..LANDMARKS).map(|i| i * n / LANDMARKS).collect()
    };
    let far = |a: usize, b: usize| {
        let cos = dot(&model.vectors[marks[a]], &model.vectors[marks[b]]);
        2.0 * (1.0 - cos).max(0.0)
    };
    let begun = start.map(|s| {
        let known: Vec<(usize, Point)> = (0..n)
            .filter_map(|i| Some((i, s.get(i).copied().flatten()?)))
            .collect();
        marks
            .iter()
            .map(|&i| {
                s.get(i)
                    .copied()
                    .flatten()
                    .unwrap_or_else(|| place(model, i, &known))
            })
            .collect()
    });
    let laid = tsne(marks.len(), far, begun, seed ^ model.lens as u64);
    let mut at: Vec<Option<Point>> = vec![None; n];
    for (m, &i) in marks.iter().enumerate() {
        at[i] = Some(laid[m]);
    }
    let known: Vec<(usize, Point)> = marks.iter().zip(&laid).map(|(&i, &p)| (i, p)).collect();
    (0..n)
        .map(|i| at[i].unwrap_or_else(|| place(model, i, &known)))
        .collect()
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
/// plane of the first two principal components of them all, found by
/// power iteration.
pub fn hues(vectors: &[Vector]) -> Vec<f32> {
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
        let mut out = [0f64; DIM];
        for c in &centred {
            let p: f64 = c.iter().zip(x).map(|(a, b)| a * b).sum();
            for d in 0..DIM {
                out[d] += p * c[d];
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
            (b.atan2(a).to_degrees().rem_euclid(360.0)) as f32
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
