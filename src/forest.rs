//! A cluster as a tree: its parts, by what they are about, and theirs, as
//! long as a part is big enough to split; the threads of the finest, by
//! their subject; and their messages, oldest first. A branch comes after
//! those begun before it, so that new mail comes at the end of its thread,
//! or begins one at the end of its part, and nothing else moves.

use std::collections::BTreeMap;

use crate::map::Snapshot;
use crate::model::{self, Vector};

/// A branch of a cluster's tree: the cluster, a part of it, a thread or,
/// with no branches, a message.
#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    pub name: String,
    /// The messages under it, by their place, oldest first.
    pub messages: Vec<usize>,
    pub branches: Vec<Branch>,
    /// Whether its branches are parts, shown as a mosaic, rather than
    /// threads.
    pub parts: bool,
}

/// How far down a cluster's parts go, the cluster's own parts the first.
const DEEPEST: usize = 4;

impl Branch {
    pub fn is_leaf(&self) -> bool {
        self.branches.is_empty()
    }

    pub fn newest(&self) -> Option<usize> {
        self.messages.last().copied()
    }

    /// The way down to the message `i`, by the place of each branch taken.
    pub fn path_to(&self, i: usize) -> Option<Vec<usize>> {
        if self.is_leaf() {
            return (self.messages == [i]).then(Vec::new);
        }
        let k = self.branches.iter().position(|b| b.messages.contains(&i))?;
        let mut path = self.branches[k].path_to(i)?;
        path.insert(0, k);
        Some(path)
    }

    /// The branch the way `path` goes down to.
    pub fn at(&self, path: &[usize]) -> Option<&Branch> {
        match path.split_first() {
            None => Some(self),
            Some((&k, rest)) => self.branches.get(k)?.at(rest),
        }
    }

    /// The way to every branch `depth` down, in turn.
    pub fn at_depth(&self, depth: usize) -> Vec<Vec<usize>> {
        if depth == 0 {
            return vec![Vec::new()];
        }
        let mut all = Vec::new();
        for (k, b) in self.branches.iter().enumerate() {
            for mut path in b.at_depth(depth - 1) {
                path.insert(0, k);
                all.push(path);
            }
        }
        all
    }

    /// How many levels it has, itself the first.
    pub fn levels(&self) -> usize {
        1 + self.branches.iter().map(Branch::levels).max().unwrap_or(0)
    }

    /// What is shown with the branch the way `picked` goes to picked: the
    /// parts it is among, as a mosaic; else, below the finest parts, the
    /// threads of the part it is under, as a tree from that part.
    pub fn level(&self, picked: &[usize]) -> Level {
        let above = &picked[..picked.len().saturating_sub(1)];
        if self.at(above).is_some_and(|b| b.parts) {
            return Level::Mosaic(above.to_vec());
        }
        let part = (0..picked.len())
            .rev()
            .find(|&j| self.at(&picked[..j]).is_some_and(|b| b.parts))
            .map_or(Vec::new(), |j| picked[..=j].to_vec());
        Level::Tree(part)
    }

    /// The names of the branches the way `path` goes through, itself the
    /// first.
    pub fn names(&self, path: &[usize]) -> Vec<String> {
        (0..=path.len())
            .filter_map(|j| self.at(&path[..j]).map(|b| b.name.clone()))
            .collect()
    }
}

/// What a cluster gone into shows: a mosaic of the parts of the branch the
/// way goes to, or a tree from the part the way goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Level {
    Mosaic(Vec<usize>),
    Tree(Vec<usize>),
}

/// When the message `i` came, to put it in turn.
fn when(s: &Snapshot, i: usize) -> (i64, usize) {
    let d = &s.dots[i];
    (d.date.unwrap_or(d.arrived), i)
}

/// Who a message is from, or, sent from here, whom it went to.
fn who(s: &Snapshot, i: usize) -> String {
    let d = &s.dots[i];
    if d.system.sent {
        format!("to {}", d.to.first().map_or("", String::as_str))
    } else if d.from.is_empty() {
        d.address.clone()
    } else {
        d.from.clone()
    }
}

/// The tree of the group `id`, of the messages `shown` (in order), and
/// the centres its parts were split by; its parts split by `centres`, as
/// they were, if given. None if none of it is shown.
pub fn grow(
    s: &Snapshot,
    id: u32,
    shown: &[usize],
    centres: Option<&[Vector]>,
) -> Option<(Branch, Vec<Vector>)> {
    let g = s.group(id)?;
    let mut members: Vec<usize> = g
        .members
        .iter()
        .copied()
        .filter(|i| shown.binary_search(i).is_ok())
        .collect();
    if members.is_empty() {
        return None;
    }
    members.sort_by_key(|&i| when(s, i));
    let (parts, centres) = model::parts(&s.model, &members, centres, u64::from(id));
    // What the cluster is named by does not name its parts.
    let mut named: Vec<String> = g.words.clone();
    if let Some(l) = &g.label {
        named.extend(model::words(l));
    }
    let (branches, parts) = split(s, parts, &members, &named, u64::from(id), 1);
    Some((
        Branch {
            name: g.name(),
            messages: members,
            branches,
            parts,
        },
        centres,
    ))
}

/// The branches of `whole`, oldest first, as `parts` splits it, and
/// whether they are parts: each part named by the words that set it apart
/// from the rest of `whole`, but those `named` above it, and split again
/// as `model::parts` splits it, `depth` deep, till too small to split;
/// one part alone, the threads of `whole`.
fn split(
    s: &Snapshot,
    parts: Vec<Vec<usize>>,
    whole: &[usize],
    named: &[String],
    seed: u64,
    depth: usize,
) -> (Vec<Branch>, bool) {
    let parts: Vec<Vec<usize>> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return (threads(s, whole), false);
    }
    let mut branches: Vec<Branch> = parts
        .into_iter()
        .enumerate()
        .map(|(k, mut part)| {
            part.sort_by_key(|&i| when(s, i));
            let words = model::part_words(&s.model, &part, whole, named);
            let (branches, parts) = if depth < DEEPEST {
                let seed = seed.wrapping_mul(31).wrapping_add(k as u64 + 1);
                let (finer, _) = model::parts(&s.model, &part, None, seed);
                let mut named = named.to_vec();
                named.extend(words.iter().cloned());
                split(s, finer, &part, &named, seed, depth + 1)
            } else {
                (threads(s, &part), false)
            };
            Branch {
                name: if words.is_empty() {
                    "the rest".to_string()
                } else {
                    words.join(" ")
                },
                branches,
                parts,
                messages: part,
            }
        })
        .collect();
    branches.sort_by_key(|b| b.messages.first().map(|&i| when(s, i)));
    (branches, true)
}

/// A subject as threads are told apart: `Re:` and `Fwd:` let go, and each
/// number one alike, as mail a program sends differs by its number alone.
pub fn thread_of(subject: &str) -> String {
    model::bare_subject(subject)
        .split_whitespace()
        .map(|w| {
            let bare = w.trim_matches(|c: char| !c.is_alphanumeric());
            if !bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit()) {
                w.replace(bare, "…")
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A message as it is threaded: its ID, what it answers, Gmail's thread
/// of it, and its subject.
#[derive(Clone, Debug)]
pub struct Threaded {
    pub message_id: String,
    pub parents: Vec<String>,
    pub thread: Option<u64>,
    pub subject: String,
}

/// The threads of `items`, by their places in it: who answered whom, by
/// their headers, and Gmail's threads, whatever their subjects; answers to
/// one not here together; and those that answer nothing and are answered
/// by nothing, by subject, as `thread_of` tells them apart.
pub fn thread_groups(items: &[Threaded]) -> Vec<Vec<usize>> {
    fn root(up: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while up[r] != r {
            r = up[r];
        }
        let mut i = i;
        while up[i] != r {
            let next = up[i];
            up[i] = r;
            i = next;
        }
        r
    }
    let mut up: Vec<usize> = (0..items.len()).collect();
    fn join(a: usize, b: usize, up: &mut [usize]) {
        let (ra, rb) = (root(up, a), root(up, b));
        if ra != rb {
            up[ra.max(rb)] = ra.min(rb);
        }
    }
    let id = |s: &str| s.trim().trim_matches(['<', '>']).to_lowercase();
    let by_id: BTreeMap<String, usize> = items
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.message_id.trim().is_empty())
        .map(|(i, t)| (id(&t.message_id), i))
        .collect();
    let mut linked = vec![false; items.len()];
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    let mut threads: BTreeMap<u64, usize> = BTreeMap::new();
    for (i, t) in items.iter().enumerate() {
        for p in t.parents.iter().map(|p| id(p)) {
            linked[i] = true;
            match by_id.get(&p) {
                Some(&j) => {
                    linked[j] = true;
                    join(i, j, &mut up);
                }
                None => {
                    let first = *missing.entry(p).or_insert(i);
                    join(i, first, &mut up);
                }
            }
        }
        if let Some(g) = t.thread {
            linked[i] = true;
            let first = *threads.entry(g).or_insert(i);
            join(i, first, &mut up);
        }
    }
    let mut subjects: BTreeMap<String, usize> = BTreeMap::new();
    for (i, t) in items.iter().enumerate().filter(|(i, _)| !linked[*i]) {
        let subject = thread_of(&t.subject).to_lowercase();
        if !subject.is_empty() {
            let first = *subjects.entry(subject).or_insert(i);
            join(i, first, &mut up);
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..items.len() {
        let r = root(&mut up, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().collect()
}

/// The threads of `messages`, oldest first, as `thread_groups` tells them
/// apart; a message alone is a leaf of its own.
fn threads(s: &Snapshot, messages: &[usize]) -> Vec<Branch> {
    let items: Vec<Threaded> = messages
        .iter()
        .map(|&i| {
            let d = &s.dots[i];
            Threaded {
                message_id: d.message_id.clone(),
                parents: d.parents.clone(),
                thread: d.thread,
                subject: d.subject.clone(),
            }
        })
        .collect();
    let by: Vec<Vec<usize>> = thread_groups(&items)
        .into_iter()
        .map(|g| g.into_iter().map(|k| messages[k]).collect())
        .collect();
    let bare = |i: usize| model::bare_subject(&s.dots[i].subject).trim().to_string();
    let mut threads: Vec<Branch> = by
        .into_iter()
        .map(|mut thread| {
            thread.sort_by_key(|&i| when(s, i));
            let first = thread[0];
            if thread.len() == 1 {
                let subject = match bare(first) {
                    s if s.is_empty() => "(no subject)".to_string(),
                    s => s,
                };
                return Branch {
                    name: format!("{subject} — {}", who(s, first)),
                    messages: thread,
                    branches: Vec::new(),
                    parts: false,
                };
            }
            // Told apart by a number, each says its own subject.
            let differ = thread.iter().any(|&i| bare(i) != bare(first));
            Branch {
                name: thread_of(&s.dots[first].subject),
                branches: thread
                    .iter()
                    .map(|&i| Branch {
                        name: if differ {
                            format!("{} — {}", bare(i), who(s, i))
                        } else {
                            who(s, i)
                        },
                        messages: vec![i],
                        branches: Vec::new(),
                        parts: false,
                    })
                    .collect(),
                messages: thread,
                parts: false,
            }
        })
        .collect();
    threads.sort_by_key(|b| b.messages.first().map(|&i| when(s, i)));
    threads
}
