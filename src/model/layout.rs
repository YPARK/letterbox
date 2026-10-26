//! The map laid out: the nearest as a graph, laid out as UMAP lays it,
//! placed where it was, its axes and hues, and how faithful it is.
use super::*;

/// What each word of `members` weighs in them all, or, read by the people
/// lens, how many each sender sent.
pub(super) fn weights<'a>(model: &'a Model, members: &[usize]) -> HashMap<&'a str, f32> {
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
pub(super) fn width(dists: &[f32], rho: f32, target: f32) -> f32 {
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
pub(super) fn memberships(dists: &[f32]) -> Vec<f32> {
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
pub(super) fn graph(near: &[Vec<(usize, f32)>]) -> Vec<(usize, usize, f32)> {
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
pub(super) fn umap(
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
pub(super) fn pull(p: &mut Point, q: &Point, alpha: f32) {
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
pub(super) fn push(p: &mut Point, q: &Point, alpha: f32) {
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
pub(super) fn centred(mut points: Vec<Point>) -> Vec<Point> {
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
pub(super) fn spread(points: Vec<Point>) -> Vec<Point> {
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
    pub(super) mean: [f64; DIM],
    pub(super) components: [[f64; DIM]; 2],
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
    pub(super) fn coordinates(&self, v: &Vector) -> [f64; 2] {
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
