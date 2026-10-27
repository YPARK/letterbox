//! How rare each token is, by TF-IDF, and each message's nearest, found
//! through which messages have each token.
use super::*;

/// How many of the messages have each bucket, and of how many.
#[derive(Clone, Debug, Default)]
pub struct Rarity {
    pub(super) seen: HashMap<u64, u32>,
    pub(super) n: usize,
}

impl Rarity {
    pub(super) fn of(bags: &[Bag]) -> Rarity {
        let mut r = Rarity::default();
        for bag in bags {
            r.add(bag);
        }
        r
    }

    pub(super) fn add(&mut self, bag: &Bag) {
        self.n += 1;
        for &(b, _) in bag {
            *self.seen.entry(b).or_default() += 1;
        }
    }

    pub(super) fn remove(&mut self, bag: &Bag) {
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
    pub(super) fn idf(&self, b: u64) -> f32 {
        let n = self.n as f32;
        ((n + 1.0) / (self.seen.get(&b).copied().unwrap_or(0) as f32 + 1.0)).ln() + 1.0
    }

    /// What a token of the bucket `b` weighs, for each of its own weight:
    /// by how rare it is. A token of one message alone tells it from none,
    /// and one of most tells none apart: either is only noise, and weighs
    /// nothing.
    pub(super) fn factor(&self, b: u64) -> f32 {
        let n = self.n as f32;
        let seen = self.seen.get(&b).copied().unwrap_or(0) as f32;
        let telling = n < 20.0 || (seen > 1.0 && seen <= n / 2.0);
        if telling { self.idf(b) } else { 0.0 }
    }

    /// The length of `bag`, each token weighed.
    pub(super) fn norm(&self, bag: &Bag) -> f32 {
        bag.iter()
            .map(|&(b, w)| (w * self.factor(b)).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    /// The cosine of two bags, each token weighed, of the lengths given.
    pub(super) fn cosine(&self, a: &Bag, b: &Bag, na: f32, nb: f32) -> f32 {
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
    pub(super) fn project(&self, bag: &Bag) -> Vector {
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

pub(super) fn postings(bags: &[Bag]) -> Postings {
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
pub(super) fn kin(
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
pub(super) fn kin_all(
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
pub(super) fn smoothed(projected: &[Vector], i: usize, near: &[(usize, f32)]) -> Vector {
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
