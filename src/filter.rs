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
use std::collections::{HashMap, HashSet};

/// A decision, as written down: the message, whether it was thrown away,
/// and what was read of it then, as it may be gone by the next start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// Whether letterbox threw it away on its own, sure enough: written
    /// down, to be told of and put back, never learnt from.
    #[serde(default)]
    pub auto: bool,
    /// When it was decided, in seconds since 1970.
    #[serde(default)]
    pub at: i64,
    /// Thrown away on its own: who sent it, what about, and how sure it was.
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub odds: f32,
}

/// What is known beside a message, that tells mail you want: whom you
/// have written to, by the mail you sent.
#[derive(Clone, Debug, Default)]
pub struct Known {
    /// Each address you sent mail to, or copied, in lower case.
    pub written: HashSet<String>,
}

/// Whether `address` is one that takes no answer: `noreply`, `do-not-reply`,
/// `notifications` and the like.
fn takes_no_answer(address: &str) -> bool {
    let local = address
        .rsplit_once('@')
        .map_or(address, |(l, _)| l)
        .to_lowercase()
        .replace(['-', '_', '.'], "");
    ["noreply", "donotreply", "notification", "mailerdaemon"]
        .iter()
        .any(|w| local.contains(w))
}

/// How many of each it is to have been taught before it says anything.
pub const READY: u32 = 20;
/// How sure it is to be, at least, to say a message would be thrown away:
/// at 80%, graded on real mail, it would have thrown away one kept message
/// in 27; at 90%, one in 160, catching two in three of those thrown away.
pub const SURE: f32 = 0.9;
/// How sure it is to be, at least, to throw new mail away on its own: at
/// 95%, graded on real mail, it would have thrown away none of 161 kept.
pub const AUTO: f32 = 0.95;
/// How much of each weight is kept a decision: what was learnt long ago,
/// and not since, fades, by half in some 7,000 decisions, and no weight
/// grows without end. Graded on real mail, stronger fading only lost what
/// it caught.
pub const DECAY: f32 = 0.9999;
/// How far each step goes, and how many a decision takes.
const RATE: f32 = 0.5;
const STEPS: usize = 3;

#[derive(Clone, Debug)]
pub struct Filter {
    /// Each token's weight, by its FNV-1a hash, and the decision it was
    /// last changed at: it has faded by `decay` each decision since.
    weights: HashMap<u64, (f32, u32)>,
    bias: f32,
    /// How many decisions were learnt, and how much of a weight each keeps.
    step: u32,
    decay: f32,
    /// How many it was taught were thrown away, and kept.
    pub thrown: u32,
    pub kept: u32,
    /// Every decision, in turn, to write again as one base.
    pub taught: Vec<Taught>,
}

/// What happened since it was last told of: what was thrown away on its
/// own, and how many of your own decisions were learnt; as it is shown,
/// the one picked, and how it does now.
#[derive(Clone, Debug, Default)]
pub struct Debrief {
    pub thrown: Vec<Thrown>,
    pub decided: usize,
    pub kept: usize,
    pub thrown_by_you: usize,
    pub title: String,
    pub at: usize,
    pub grade: Option<Grade>,
}

/// Whether letterbox throws new mail away on its own, for mail come since
/// when, and when it last told of what it did; kept as a few lines beside
/// the accounts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Auto {
    pub on: bool,
    pub since: i64,
    pub debriefed: i64,
}

impl Auto {
    /// As `text` says, at `now`: on, for mail come from now, where it says
    /// nothing.
    pub fn read(text: &str, now: i64) -> Auto {
        let mut a = Auto {
            on: true,
            since: now,
            debriefed: now,
        };
        for line in text.lines() {
            match line.split_whitespace().collect::<Vec<_>>()[..] {
                ["auto", on] => a.on = on != "off",
                ["since", at] => a.since = at.parse().unwrap_or(now),
                ["debriefed", at] => a.debriefed = at.parse().unwrap_or(now),
                _ => {}
            }
        }
        a
    }
}

impl std::fmt::Display for Auto {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let on = if self.on { "on" } else { "off" };
        writeln!(f, "auto {on}")?;
        writeln!(f, "since {}", self.since)?;
        writeln!(f, "debriefed {}", self.debriefed)
    }
}

/// A message thrown away on its own, as it is told of.
#[derive(Clone, Debug)]
pub struct Thrown {
    pub key: Key,
    pub from: String,
    pub subject: String,
    pub odds: f32,
    pub why: Vec<String>,
    pub message_id: String,
    pub at: i64,
    /// Whether it was put back since.
    pub back: bool,
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

impl Default for Filter {
    fn default() -> Self {
        Filter::with_decay(DECAY)
    }
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
    /// people; and whether you wrote to its sender, whether its sender takes
    /// no answer, and whether it was sent to many.
    pub fn tokens(r: &Record, known: &Known) -> Vec<(String, f32)> {
        let mut t = crate::model::tokens(r, false, Lens::Both);
        if known.written.contains(&r.address.to_lowercase()) {
            t.push(("wrote".to_string(), 2.5));
        }
        if takes_no_answer(&r.address) {
            t.push(("noreply".to_string(), 1.5));
        }
        if r.bulk {
            t.push(("bulk".to_string(), 1.5));
        }
        t
    }

    /// A filter whose weights keep `decay` of themselves a decision.
    pub fn with_decay(decay: f32) -> Filter {
        Filter {
            weights: HashMap::new(),
            bias: 0.0,
            step: 0,
            decay,
            thrown: 0,
            kept: 0,
            taught: Vec::new(),
        }
    }

    /// The weight of the token hashed `b`, as it has faded till now.
    fn weight_of(&self, b: u64) -> f32 {
        self.weights.get(&b).map_or(0.0, |&(w, at)| {
            w * self.decay.powi(self.step.saturating_sub(at) as i32)
        })
    }

    /// The weight of `token`, as it has faded till now.
    pub fn weight(&self, token: &str) -> f32 {
        self.weight_of(fnv(token.as_bytes()))
    }

    fn margin(&self, x: &[(u64, f32)]) -> f32 {
        self.bias + x.iter().map(|(b, v)| self.weight_of(*b) * v).sum::<f32>()
    }

    /// How likely mail read as `tokens` is to be thrown away.
    pub fn odds(&self, tokens: &[(String, f32)]) -> f32 {
        sigmoid(self.margin(&features(tokens)))
    }

    /// Learn from a decision. The rarer kind weighs more, so that many of
    /// one do not drown the other.
    pub fn learn(&mut self, t: Taught) {
        // What it threw away on its own is its own guess: kept to be told
        // of, but learnt from, it would teach itself.
        if t.auto {
            self.taught.push(t);
            return;
        }
        let x = features(&t.tokens);
        let y = if t.thrown { 1.0 } else { 0.0 };
        let (thrown, kept) = (self.thrown as f32 + 1.0, self.kept as f32 + 1.0);
        let weight = if t.thrown {
            kept / thrown
        } else {
            thrown / kept
        }
        .clamp(0.2, 5.0);
        self.step += 1;
        for _ in 0..STEPS {
            let g = (sigmoid(self.margin(&x)) - y) * weight * RATE;
            self.bias -= g * 0.1;
            for (b, v) in &x {
                let w = self.weight_of(*b) - g * v;
                self.weights.insert(*b, (w, self.step));
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
        Self::check_with(taught, DECAY)
    }

    /// As `check`, its weights keeping `decay` of themselves a decision.
    pub fn check_with(taught: &[Taught], decay: f32) -> Vec<Said> {
        let mut f = Filter::with_decay(decay);
        let mut said = Vec::new();
        for t in taught.iter().filter(|t| !t.auto) {
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

    /// Why mail read as `tokens` would be thrown away, in words: what
    /// weighs most toward it, at most three things, its words together.
    pub fn why(&self, tokens: &[(String, f32)]) -> Vec<String> {
        let mut weighed: Vec<(&str, f32)> = tokens
            .iter()
            .zip(features(tokens))
            .map(|((t, _), (b, v))| (t.as_str(), self.weight_of(b) * v))
            .filter(|(_, c)| *c > 0.0)
            .collect();
        weighed.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        let mut said: Vec<String> = Vec::new();
        let mut words: Vec<&str> = Vec::new();
        let mut words_at = None;
        for (t, _) in weighed {
            if let Some(w) = t.strip_prefix("w:") {
                if words.len() < 3 {
                    words_at.get_or_insert(said.len());
                    words.push(w);
                }
                continue;
            }
            if said.len() + usize::from(words_at.is_some()) >= 3 {
                continue;
            }
            let reason = if let Some(a) = t.strip_prefix("from:") {
                format!("from {a}")
            } else if let Some(d) = t.strip_prefix("dom:") {
                format!("from {d}")
            } else if let Some(l) = t.strip_prefix("list:") {
                format!("list {l}")
            } else if let Some(a) = t.strip_prefix("to:") {
                format!("sent with {a}")
            } else {
                match t {
                    "bulk" => "sent to many".to_string(),
                    "noreply" => "takes no answer".to_string(),
                    _ => continue,
                }
            };
            said.push(reason);
        }
        if let Some(at) = words_at {
            said.insert(at.min(said.len()), format!("words: {}", words.join(", ")));
        }
        said.truncate(3);
        said
    }

    /// How likely `r` is to be thrown away, if it has been taught enough
    /// to say, and is sure enough.
    pub fn would_throw(&self, r: &Record, known: &Known) -> Option<f32> {
        if !self.ready() {
            return None;
        }
        let p = self.odds(&Self::tokens(r, known));
        (p >= SURE).then_some(p)
    }
}
