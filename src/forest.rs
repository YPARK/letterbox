//! A cluster as a tree: its parts, by what they are about; the threads of
//! each, by their subject; and their messages, oldest first. A branch comes
//! after those begun before it, so that new mail comes at the end of its
//! thread, or begins one at the end of its part, and nothing else moves.

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
}

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
    let mut branches: Vec<Branch> = parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|mut part| {
            part.sort_by_key(|&i| when(s, i));
            let words = model::part_words(&s.model, &part, &named);
            Branch {
                name: if words.is_empty() {
                    "the rest".to_string()
                } else {
                    words.join(" ")
                },
                branches: threads(s, &part),
                messages: part,
            }
        })
        .collect();
    // One part alone: its threads are the cluster's own.
    if branches.len() == 1 {
        branches = branches.remove(0).branches;
    }
    branches.sort_by_key(|b| b.messages.first().map(|&i| when(s, i)));
    Some((
        Branch {
            name: g.name(),
            messages: members,
            branches,
        },
        centres,
    ))
}

/// The threads of `messages`, oldest first: those of a subject, `Re:` and
/// `Fwd:` let go, together; a message alone is a leaf of its own.
fn threads(s: &Snapshot, messages: &[usize]) -> Vec<Branch> {
    let mut by: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &i in messages {
        let subject = model::bare_subject(&s.dots[i].subject)
            .trim()
            .to_lowercase();
        let key = if subject.is_empty() {
            format!("\0{i}")
        } else {
            subject
        };
        by.entry(key).or_default().push(i);
    }
    let mut threads: Vec<Branch> = by
        .into_values()
        .map(|mut thread| {
            thread.sort_by_key(|&i| when(s, i));
            let first = thread[0];
            let subject = model::bare_subject(&s.dots[first].subject).trim();
            let subject = if subject.is_empty() {
                "(no subject)"
            } else {
                subject
            };
            if thread.len() == 1 {
                return Branch {
                    name: format!("{subject} — {}", who(s, first)),
                    messages: thread,
                    branches: Vec::new(),
                };
            }
            Branch {
                name: subject.to_string(),
                branches: thread
                    .iter()
                    .map(|&i| Branch {
                        name: who(s, i),
                        messages: vec![i],
                        branches: Vec::new(),
                    })
                    .collect(),
                messages: thread,
            }
        })
        .collect();
    threads.sort_by_key(|b| b.messages.first().map(|&i| when(s, i)));
    threads
}
