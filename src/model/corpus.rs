//! What the model has read, kept from one snapshot to the next: followed
//! as mail comes and goes, and taken into a model.
use super::*;

/// What the model has read of each message, kept from one snapshot to the
/// next, so that new mail is read alone and the rest is not read again.
/// Each message's direction is made once, by how rare its tokens were
/// then; how near messages are is always by how rare they are now.
pub struct Corpus {
    pub lens: Lens,
    pub(super) keys: Vec<Key>,
    pub(super) place: HashMap<Key, usize>,
    pub(super) sent: Vec<bool>,
    pub(super) bags: Vec<Bag>,
    pub(super) rarity: Rarity,
    pub(super) postings: Postings,
    pub(super) norms: Vec<f32>,
    pub(super) projected: Vec<Vector>,
    /// Each direction made less noisy by its nearest, as it was read.
    pub(super) base: Vec<Vector>,
    pub(super) near: Vec<Vec<(usize, f32)>>,
    pub(super) words: Vec<Arc<[(String, f32)]>>,
    pub(super) senders: Vec<Arc<str>>,
    /// How many messages there were when all were last read together.
    pub(super) whole: usize,
    /// The plane hues are read in, found once all are read together.
    pub(super) axes: Option<Axes>,
}

/// A message read: its bag, its words and its sender.
pub(super) fn read((r, _, sent): &Given, lens: Lens) -> (Bag, Vec<(u64, String, f32)>, Arc<str>) {
    let tokens: Vec<(u64, String, f32)> = tokens(r, *sent, lens)
        .into_iter()
        .map(|(t, w)| (bucket(&t), t, w))
        .collect();
    // Mail sent from here names no cluster by its sender.
    let sender: Arc<str> = if *sent {
        "".into()
    } else {
        r.from.as_str().into()
    };
    (bag_of(&tokens), tokens, sender)
}

impl Corpus {
    /// Every one of `messages` read, in the order given (the index's, by
    /// key), as `lens` reads them.
    pub fn read(messages: &[Given], lens: Lens) -> Corpus {
        let read: Vec<_> = messages.par_iter().map(|m| read(m, lens)).collect();
        let bags: Vec<Bag> = read.iter().map(|r| r.0.clone()).collect();
        let rarity = Rarity::of(&bags);
        let norms: Vec<f32> = bags.par_iter().map(|b| rarity.norm(b)).collect();
        let postings = postings(&bags);
        let near = kin_all(&bags, &norms, &rarity, &postings, NEIGHBOURS);
        let projected: Vec<Vector> = bags.par_iter().map(|b| rarity.project(b)).collect();
        let base: Vec<Vector> = (0..bags.len())
            .into_par_iter()
            .map(|i| smoothed(&projected, i, &near[i]))
            .collect();
        let words = read.iter().map(|(_, t, _)| words_of(t, &rarity)).collect();
        let keys: Vec<Key> = messages.iter().map(|(r, _, _)| r.key.clone()).collect();
        Corpus {
            lens,
            place: keys
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, k)| (k, i))
                .collect(),
            keys,
            sent: messages.iter().map(|m| m.2).collect(),
            bags,
            rarity,
            postings,
            norms,
            projected,
            base,
            near,
            words,
            senders: read.into_iter().map(|r| r.2).collect(),
            whole: messages.len(),
            axes: None,
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Follow `messages`, as they are now: read those new, let go those
    /// gone; or, if the mail has grown or shrunk too far since it was read
    /// whole, read it whole again. Whether anything changed.
    pub fn follow(&mut self, messages: &[Given]) -> bool {
        let now: HashMap<&Key, bool> = messages.iter().map(|(r, _, s)| (&r.key, *s)).collect();
        let gone: Vec<usize> = (0..self.len())
            .filter(|&i| now.get(&self.keys[i]) != Some(&self.sent[i]))
            .collect();
        let new: Vec<usize> = (0..messages.len())
            .filter(|&m| {
                let (r, _, sent) = &messages[m];
                self.place
                    .get(&r.key)
                    .is_none_or(|&i| self.sent[i] != *sent)
            })
            .collect();
        if gone.is_empty() && new.is_empty() {
            return false;
        }
        let whole = self.whole.max(1) as f32;
        let kept = self.len() - gone.len();
        if messages.len() as f32 > GROWN * whole || (kept as f32) < SHRUNK * whole {
            *self = Corpus::read(messages, self.lens);
            return true;
        }
        let mut short = if gone.is_empty() {
            Vec::new()
        } else {
            self.forget(&gone)
        };
        let first = self.len();
        self.take(&new.iter().map(|&m| &messages[m]).collect::<Vec<_>>());
        // The nearest of those that lost some, found again, now that the
        // new are there too.
        short.retain(|&i| i < first);
        let mut acc = vec![0f32; self.len()];
        for i in short {
            self.near[i] = kin(
                i,
                NEIGHBOURS,
                &self.bags,
                &self.norms,
                &self.rarity,
                &self.postings,
                &mut acc,
            );
        }
        true
    }

    /// Let the messages `gone`, by place, go; the places of those whose
    /// nearest lost some.
    pub(super) fn forget(&mut self, gone: &[usize]) -> Vec<usize> {
        let mut to: Vec<Option<usize>> = vec![None; self.len()];
        let mut kept = 0;
        let mut g = gone.iter().peekable();
        for (i, slot) in to.iter_mut().enumerate() {
            if g.peek() == Some(&&i) {
                g.next();
                self.rarity.remove(&self.bags[i]);
            } else {
                *slot = Some(kept);
                kept += 1;
            }
        }
        fn keep<T>(v: &mut Vec<T>, to: &[Option<usize>]) {
            let mut i = 0;
            v.retain(|_| {
                i += 1;
                to[i - 1].is_some()
            });
        }
        keep(&mut self.keys, &to);
        keep(&mut self.sent, &to);
        keep(&mut self.bags, &to);
        keep(&mut self.projected, &to);
        keep(&mut self.base, &to);
        keep(&mut self.near, &to);
        keep(&mut self.words, &to);
        keep(&mut self.senders, &to);
        let mut short = Vec::new();
        for (i, list) in self.near.iter_mut().enumerate() {
            let before = list.len();
            list.retain_mut(|(j, _)| match to[*j] {
                Some(k) => {
                    *j = k;
                    true
                }
                None => false,
            });
            if list.len() < before {
                short.push(i);
            }
        }
        self.place = self
            .keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| (k, i))
            .collect();
        self.postings = postings(&self.bags);
        short
    }

    /// Read `messages`, new, after the rest.
    pub(super) fn take(&mut self, messages: &[&Given]) {
        let first = self.len();
        let read: Vec<_> = messages.par_iter().map(|m| read(m, self.lens)).collect();
        for (k, (bag, _, sender)) in read.iter().enumerate() {
            let i = first + k;
            let (r, _, sent) = messages[k];
            self.rarity.add(bag);
            for &(b, w) in bag {
                self.postings.entry(b).or_default().push((i as u32, w));
            }
            self.place.insert(r.key.clone(), i);
            self.keys.push(r.key.clone());
            self.sent.push(*sent);
            self.bags.push(bag.clone());
            self.senders.push(sender.clone());
        }
        let rarity = &self.rarity;
        self.norms = self.bags.par_iter().map(|b| rarity.norm(b)).collect();
        for (_, tokens, _) in &read {
            self.words.push(words_of(tokens, rarity));
        }
        let n = self.len();
        let found: Vec<Vec<(usize, f32)>> = (first..n)
            .into_par_iter()
            .map_init(
                || vec![0f32; n],
                |acc, i| {
                    kin(
                        i,
                        NEIGHBOURS,
                        &self.bags,
                        &self.norms,
                        rarity,
                        &self.postings,
                        acc,
                    )
                },
            )
            .collect();
        self.near.extend(found);
        // Each new message is among the nearest of those nearest it, where
        // it is nearer than their farthest.
        for i in first..n {
            for k in 0..self.near[i].len() {
                let (j, d) = self.near[i][k];
                if j >= first {
                    continue;
                }
                let list = &mut self.near[j];
                if list.len() < NEIGHBOURS || list.last().is_some_and(|l| d < l.1) {
                    let at = list.partition_point(|x| x.1 < d || (x.1 == d && x.0 < i));
                    list.insert(at, (i, d));
                    list.truncate(NEIGHBOURS);
                }
            }
        }
        for i in first..n {
            let v = rarity.project(&self.bags[i]);
            self.projected.push(v);
        }
        for i in first..n {
            let v = smoothed(&self.projected, i, &self.near[i]);
            self.base.push(v);
        }
    }

    /// Each message's direction before labels teach it.
    pub fn base(&self) -> &[Vector] {
        &self.base
    }

    /// How well labels would be suggested from directions `base`: each
    /// message filed under labels with others in them, its own labels
    /// hidden, its labels made again without it and it pulled as mail not
    /// yet filed is, put back under one of its own by the label nearest it.
    pub fn label_check(&self, messages: &[Given], base: &[Vector]) -> LabelCheck {
        let n = self.len();
        let mut labels: Vec<BTreeSet<String>> = vec![BTreeSet::new(); n];
        for (r, l, _) in messages {
            if let Some(&i) = self.place.get(&r.key) {
                labels[i] = l.iter().map(|x| fold(x)).collect();
            }
        }
        // Each label's members, the sum of their directions, and how many
        // of them hold each token.
        let mut members: BTreeMap<&String, Vec<usize>> = BTreeMap::new();
        for (i, ls) in labels.iter().enumerate() {
            for l in ls {
                members.entry(l).or_default().push(i);
            }
        }
        let sums: BTreeMap<&String, (Vector, HashMap<u64, usize>)> = members
            .iter()
            .map(|(l, of)| {
                let mut sum = [0f32; DIM];
                let mut held: HashMap<u64, usize> = HashMap::new();
                for &i in of {
                    sum.iter_mut().zip(&base[i]).for_each(|(s, x)| *s += x);
                    for t in self.bags[i].iter() {
                        *held.entry(t.0).or_default() += 1;
                    }
                }
                (*l, (sum, held))
            })
            .collect();
        let weigh = |b: u64, w: f32| w * self.rarity.factor(b);
        let hits: Vec<bool> = (0..n)
            .into_par_iter()
            .filter(|&i| {
                labels[i]
                    .iter()
                    .any(|l| members.get(l).is_some_and(|m| m.len() >= 2))
            })
            .map(|i| {
                let bag = &self.bags[i];
                let total: f32 = bag.iter().map(|t| weigh(t.0, t.1)).sum();
                // Each label as it would be without this message.
                let without: Vec<(&String, Vector, f32)> = sums
                    .iter()
                    .filter_map(|(l, (sum, held))| {
                        let mine = labels[i].contains(*l);
                        if mine && members[l].len() < 2 {
                            return None;
                        }
                        let mut v = *sum;
                        if mine {
                            v.iter_mut().zip(&base[i]).for_each(|(s, x)| *s -= x);
                        }
                        let shared: f32 = bag
                            .iter()
                            .filter(|t| held.get(&t.0).is_some_and(|&c| c > usize::from(mine)))
                            .map(|t| weigh(t.0, t.1))
                            .sum();
                        let share = if total > 0.0 { shared / total } else { 0.0 };
                        Some((*l, normalised(v), share))
                    })
                    .collect();
                let mut v = base[i];
                for (_, l, share) in &without {
                    v.iter_mut()
                        .zip(l)
                        .for_each(|(x, y)| *x += PULL * share * y);
                }
                let v = normalised(v);
                without
                    .iter()
                    .min_by(|a, b| degrees(&v, &a.1).total_cmp(&degrees(&v, &b.1)))
                    .is_some_and(|(l, _, _)| labels[i].contains(*l))
            })
            .collect();
        LabelCheck {
            hits: hits.iter().filter(|h| **h).count(),
            said: hits.len(),
            total: hits.len(),
        }
    }

    /// How well labels would be suggested by the `k` filed messages
    /// nearest each, by directions `base`, each voting for its labels, the
    /// nearer the more: each filed message, its own labels hidden, put back
    /// under one of its own.
    pub fn label_check_nearest(
        &self,
        messages: &[Given],
        base: &[Vector],
        k: usize,
        within: f32,
    ) -> LabelCheck {
        let labels = self.labels_of(messages);
        let count = label_counts(&labels);
        let filed: Vec<usize> = (0..self.len()).filter(|&i| !labels[i].is_empty()).collect();
        let hits: Vec<(bool, bool)> = filed
            .par_iter()
            .copied()
            .filter(|&i| labels[i].iter().any(|l| count[l] >= 2))
            .map(|i| {
                let said = vote(&labels, base, &filed, i, k, within);
                (said.is_some(), said.is_some_and(|l| labels[i].contains(&l)))
            })
            .collect();
        LabelCheck {
            hits: hits.iter().filter(|h| h.1).count(),
            said: hits.iter().filter(|h| h.0).count(),
            total: hits.len(),
        }
    }

    /// Each message's labels, folded, by its place.
    pub(super) fn labels_of(&self, messages: &[Given]) -> Vec<BTreeSet<String>> {
        let mut labels: Vec<BTreeSet<String>> = vec![BTreeSet::new(); self.len()];
        for (r, l, _) in messages {
            if let Some(&i) = self.place.get(&r.key) {
                labels[i] = l.iter().map(|x| fold(x)).collect();
            }
        }
        labels
    }

    /// The model of what is read, taught by the labels of `messages` and
    /// by `not_it`, each message and label, folded, it was said not to be.
    pub fn model(&mut self, messages: &[Given], not_it: &BTreeSet<(Key, String)>) -> Model {
        let n = self.len();
        let mut labels: Vec<Vec<String>> = vec![Vec::new(); n];
        for (r, l, _) in messages {
            if let Some(&i) = self.place.get(&r.key) {
                labels[i] = l.clone();
            }
        }
        let rarity = &self.rarity;
        let bags = &self.bags;
        let mut vectors = self.base.clone();
        let weigh = |b: u64, w: f32| w * rarity.factor(b);
        // What each label is: the mean of its messages, and the tokens seen
        // in them, by how rare each is.
        let mut members: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
        for (i, labels) in labels.iter().enumerate() {
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
                    .flat_map(|&i| bags[i].iter().map(|t| t.0))
                    .collect(),
            );
        }
        let keys = &self.keys;
        vectors.par_iter_mut().enumerate().for_each(|(i, vector)| {
            let mut v = *vector;
            let mine: BTreeSet<String> = labels[i].iter().map(|l| fold(l)).collect();
            if mine.is_empty() {
                let total: f32 = bags[i].iter().map(|t| weigh(t.0, t.1)).sum();
                for (folded, seen) in &label_tokens {
                    let shared: f32 = bags[i]
                        .iter()
                        .filter(|t| seen.contains(&t.0))
                        .map(|t| weigh(t.0, t.1))
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
                if not_it.contains(&(keys[i].clone(), folded.clone())) {
                    v.iter_mut().zip(l).for_each(|(x, y)| *x -= PUSH * y);
                }
            }
            *vector = normalised(v);
        });
        let axes = self.axes.get_or_insert_with(|| Axes::of(&vectors)).clone();
        Model {
            lens: self.lens,
            keys: self.keys.clone(),
            place: self.place.clone(),
            vectors,
            near: self.near.clone(),
            words: self.words.clone(),
            senders: self.senders.clone(),
            labels,
            label_vectors,
            axes,
        }
    }
}
