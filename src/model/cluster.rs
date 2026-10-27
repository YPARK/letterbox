//! Clusters, found by angle, and a cluster's parts, each named by the words
//! that set it apart.
use super::*;

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
pub(super) fn kmeans(
    vectors: &[Vector],
    k: usize,
    start: Vec<Vector>,
    rng: &mut Rng,
) -> Vec<usize> {
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
pub(super) const ALIKE: f32 = 0.95;

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
pub(super) fn top_words(model: &Model, members: &[usize]) -> Vec<String> {
    let mut words = ranked_words(model, members);
    words.truncate(3);
    words
}

/// The words of `members`, the weightiest first, or, read by the people
/// lens, their senders, the commonest first.
pub(super) fn ranked_words(model: &Model, members: &[usize]) -> Vec<String> {
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
