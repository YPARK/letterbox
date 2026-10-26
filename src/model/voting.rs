//! Where a message not yet filed would go: voted by the filed mail nearest
//! it, each for its labels; and how well that would have done.
use super::*;

/// How much the mail may grow, or shrink, since it was last read whole,
/// before it is read whole again, rarity and all: by a quarter, or a fifth.
pub(super) const GROWN: f32 = 1.25;

pub(super) const SHRUNK: f32 = 0.8;

/// How well labels were put back: of so many filed, how many under one
/// of their own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LabelCheck {
    pub hits: usize,
    pub total: usize,
    /// How many were said to go somewhere at all.
    pub said: usize,
}

/// How many filed messages nearest one vote for where it would go, and
/// how near, in degrees, each is to be to vote at all: graded on real
/// mail, within 45° it said where for nine in ten filed messages, and was
/// right for four in five of them; nearer, it said less, and farther, it
/// was less often right.
pub const VOTERS: usize = 10;

pub const VOTED_WITHIN: f32 = 45.0;

/// How many messages hold each label.
pub(super) fn label_counts(labels: &[BTreeSet<String>]) -> HashMap<String, usize> {
    let mut count = HashMap::new();
    for ls in labels {
        for l in ls {
            *count.entry(l.clone()).or_insert(0) += 1;
        }
    }
    count
}

/// The label the `k` messages of `filed` nearest `i` by `base` vote for,
/// each its labels, the nearer the more; `i` itself not among them.
pub(super) fn vote(
    labels: &[BTreeSet<String>],
    base: &[Vector],
    filed: &[usize],
    i: usize,
    k: usize,
    within: f32,
) -> Option<String> {
    ranked_votes(labels, base, filed, i, k, within)
        .into_iter()
        .next()
}

/// The labels the `k` messages of `filed` nearest `i` vote for, most voted
/// first: of those within `within` degrees of it, if any are.
pub(super) fn ranked_votes(
    labels: &[BTreeSet<String>],
    base: &[Vector],
    filed: &[usize],
    i: usize,
    k: usize,
    within: f32,
) -> Vec<String> {
    let least = within.to_radians().cos();
    let mut near: Vec<(usize, f32)> = filed
        .iter()
        .copied()
        .filter(|&j| j != i)
        .map(|j| (j, dot(&base[i], &base[j])))
        .filter(|&(_, cos)| cos >= least)
        .collect();
    let k = k.min(near.len());
    if k == 0 {
        return Vec::new();
    }
    near.select_nth_unstable_by(k - 1, |a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut votes: BTreeMap<&String, f32> = BTreeMap::new();
    for &(j, cos) in &near[..k] {
        for l in &labels[j] {
            *votes.entry(l).or_default() += cos.max(0.0) + 1e-3;
        }
    }
    let mut ranked: Vec<(&String, f32)> = votes.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    ranked.into_iter().map(|(l, _)| l.clone()).collect()
}

/// How well labels would be suggested by always the commonest: the
/// baseline any other is to beat.
pub fn commonest_label_check(messages: &[Given]) -> LabelCheck {
    let labels: Vec<BTreeSet<String>> = messages
        .iter()
        .map(|(_, l, _)| l.iter().map(|x| fold(x)).collect())
        .collect();
    let count = label_counts(&labels);
    let Some(commonest) = count
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(l, _)| l.clone())
    else {
        return LabelCheck::default();
    };
    let eligible: Vec<&BTreeSet<String>> = labels
        .iter()
        .filter(|ls| ls.iter().any(|l| count[l] >= 2))
        .collect();
    LabelCheck {
        hits: eligible.iter().filter(|ls| ls.contains(&commonest)).count(),
        said: eligible.len(),
        total: eligible.len(),
    }
}
