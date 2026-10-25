//! What you throw away, learnt as you do it. Each message trashed teaches
//! that mail like it is thrown away; each filed, flagged or kept, that it is
//! not. Like is by what the map reads of a message, never its subject alone:
//! who sent it, from where, by which list, and its words. It is logistic
//! regression learnt online, a few steps a decision, so that the same
//! decisions in the same order give the same weights; each decision is
//! written down, with what was read of the message then, as the journal
//! `taught`, and replayed as letterbox starts.

use crate::index::record::fnv;
use crate::index::{Key, Lens, Record};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A decision, as written down: the message, whether it was thrown away,
/// and what was read of it then, as it may be gone by the next start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Taught {
    pub key: Key,
    pub thrown: bool,
    pub tokens: Vec<(String, f32)>,
    /// Its `Message-ID`, by which a message put back from the Trash is
    /// known again once the index has let it go.
    #[serde(default)]
    pub message_id: String,
    /// Whether it was read from where mail already was, the Trash or a
    /// label, not decided as you went.
    #[serde(default)]
    pub seeded: bool,
}

/// How many of each it is to have been taught before it says anything.
pub const READY: u32 = 20;
/// How sure it is to be, at least, to say a message would be thrown away:
/// at 80%, graded on real mail, it would have thrown away one kept message
/// in 27; at 90%, one in 160, catching two in three of those thrown away.
pub const SURE: f32 = 0.9;
/// How far each step goes, and how many a decision takes.
const RATE: f32 = 0.5;
const STEPS: usize = 3;

#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Each token's weight, by its FNV-1a hash.
    weights: HashMap<u64, f32>,
    bias: f32,
    /// How many it was taught were thrown away, and kept.
    pub thrown: u32,
    pub kept: u32,
    /// Every decision, in turn, to write again as one base.
    pub taught: Vec<Taught>,
}

/// What it would have said of a decision, before learning it.
#[derive(Clone, Debug)]
pub struct Said {
    pub odds: f32,
    pub thrown: bool,
    pub seeded: bool,
    pub key: Key,
    pub from: String,
}

/// How it would have done, sure to `level`: of those thrown away, how many
/// it would have said; of those kept, how many it would have thrown away.
pub fn grade(said: &[Said], level: f32) -> Grade {
    let mut g = Grade::default();
    for s in said {
        let says = s.odds >= level;
        match (s.thrown, says) {
            (true, true) => g.caught += 1,
            (true, false) => g.missed += 1,
            (false, true) => g.wrong += 1,
            (false, false) => g.right += 1,
        }
    }
    g
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Grade {
    /// Thrown away, and it would have said so.
    pub caught: usize,
    pub missed: usize,
    /// Kept, and it would have thrown them away: what matters most.
    pub wrong: usize,
    pub right: usize,
}

/// Who sent mail read as `tokens`, as the map reads them.
fn from_of(tokens: &[(String, f32)]) -> String {
    tokens
        .iter()
        .find_map(|(t, _)| t.strip_prefix("from:"))
        .unwrap_or_default()
        .to_string()
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// What is read of `tokens`: each by its hash, its weight shared out so
/// that a long message weighs no more than a short one.
fn features(tokens: &[(String, f32)]) -> Vec<(u64, f32)> {
    let norm = tokens
        .iter()
        .map(|(_, w)| w * w)
        .sum::<f32>()
        .sqrt()
        .max(1e-6);
    tokens
        .iter()
        .map(|(t, w)| (fnv(t.as_bytes()), w / norm))
        .collect()
}

impl Filter {
    /// What it reads of `r`: its tokens, as the map reads them by words and
    /// people.
    pub fn tokens(r: &Record) -> Vec<(String, f32)> {
        crate::model::tokens(r, false, Lens::Both)
    }

    fn margin(&self, x: &[(u64, f32)]) -> f32 {
        self.bias
            + x.iter()
                .map(|(b, v)| self.weights.get(b).copied().unwrap_or(0.0) * v)
                .sum::<f32>()
    }

    /// How likely mail read as `tokens` is to be thrown away.
    pub fn odds(&self, tokens: &[(String, f32)]) -> f32 {
        sigmoid(self.margin(&features(tokens)))
    }

    /// Learn from a decision. The rarer kind weighs more, so that many of
    /// one do not drown the other.
    pub fn learn(&mut self, t: Taught) {
        let x = features(&t.tokens);
        let y = if t.thrown { 1.0 } else { 0.0 };
        let (thrown, kept) = (self.thrown as f32 + 1.0, self.kept as f32 + 1.0);
        let weight = if t.thrown {
            kept / thrown
        } else {
            thrown / kept
        }
        .clamp(0.2, 5.0);
        for _ in 0..STEPS {
            let g = (sigmoid(self.margin(&x)) - y) * weight * RATE;
            self.bias -= g * 0.1;
            for (b, v) in &x {
                *self.weights.entry(*b).or_default() -= g * v;
            }
        }
        if t.thrown {
            self.thrown += 1;
        } else {
            self.kept += 1;
        }
        self.taught.push(t);
    }

    /// Whether it has been taught enough of both to say anything.
    pub fn ready(&self) -> bool {
        self.thrown >= READY && self.kept >= READY
    }

    /// How it would have done: each decision in `taught`, in turn, said
    /// before it was learnt, by what was learnt before it alone, once that
    /// was enough to say anything.
    pub fn check(taught: &[Taught]) -> Vec<Said> {
        let mut f = Filter::default();
        let mut said = Vec::new();
        for t in taught {
            if f.ready() {
                said.push(Said {
                    odds: f.odds(&t.tokens),
                    thrown: t.thrown,
                    seeded: t.seeded,
                    key: t.key.clone(),
                    from: from_of(&t.tokens),
                });
            }
            f.learn(t.clone());
        }
        said
    }

    /// How likely `r` is to be thrown away, if it has been taught enough
    /// to say, and is sure enough.
    pub fn would_throw(&self, r: &Record) -> Option<f32> {
        if !self.ready() {
            return None;
        }
        let p = self.odds(&Self::tokens(r));
        (p >= SURE).then_some(p)
    }
}
