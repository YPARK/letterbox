//! A message as tokens: its words, its subject bare, each token hashed to a
//! bucket; and the arithmetic of directions, with a seeded random source.
use super::*;

pub(super) const STOP: &[&str] = &[
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

pub(super) fn bucket(token: &str) -> u64 {
    fnv(token.as_bytes()) % BUCKETS
}

/// The next of a sequence of numbers made from a seed, by SplitMix64.
pub(super) fn splitmix(state: &mut u64) -> u64 {
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

    pub(super) fn unit(&mut self) -> f64 {
        (splitmix(&mut self.0) >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(super) fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n.saturating_sub(1))
    }

    pub(super) fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit().max(1e-12), self.unit());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

pub fn dot(a: &Vector, b: &Vector) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub(super) fn normalised(mut v: Vector) -> Vector {
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
pub(super) fn bag_of(tokens: &[(u64, String, f32)]) -> Bag {
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
