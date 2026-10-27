//! A snapshot of the index made into what the map draws: its model, its
//! clusters, where each message is, and where each would go.
use super::*;

/// What a snapshot is made from, taken from the index under its lock.
pub struct Input {
    pub(super) version: u64,
    pub(super) lens: Lens,
    pub(super) records: Vec<(Record, Vec<String>, bool, System)>,
    pub(super) not_it: std::collections::BTreeSet<(Key, String)>,
    pub(super) layout: Option<BTreeMap<Key, (f32, f32)>>,
    pub(super) before: Vec<(u32, Vector, Vec<Key>)>,
    pub(super) layout_wanted: bool,
    pub(super) last: Option<Last>,
    pub(super) corpus: Option<Corpus>,
    /// How likely each message in an Inbox is to be thrown away, where it
    /// is sure enough to say.
    pub(super) thrown: BTreeMap<Key, f32>,
    /// The messages taught as kept: looked at before.
    pub(super) looked: std::collections::BTreeSet<Key>,
}

/// A message as it is drawn from: its record, its labels, whether it has
/// one of letterbox's, its state, and whether another account has it.
pub(super) type Drawn = (Record, Vec<String>, bool, System, bool);

/// What the last snapshot made, to be kept where nothing it was made from
/// changed.
pub(super) struct Last {
    pub(super) model: Arc<Model>,
    pub(super) clustering: Clustering,
    pub(super) hues: Vec<f32>,
    pub(super) at: Option<Vec<Point>>,
    pub(super) faithfulness: Option<f32>,
    pub(super) not_it: std::collections::BTreeSet<(Key, String)>,
}

/// What a snapshot of `index` by `lens` is made from: begun from `last`,
/// its clusters and where its messages were, and from what `corpus` has
/// read; laid out if `layout` and it never was.
pub fn input(
    index: &Index,
    lens: Lens,
    last: Option<&Snapshot>,
    corpus: Option<Corpus>,
    layout: bool,
) -> Input {
    let records = index
        .records
        .values()
        .map(|r| {
            let mine = index.keep.labels.contains_key(&r.key)
                || index
                    .keep
                    .pending
                    .range((r.key.clone(), String::new())..)
                    .take_while(|((k, _), _)| *k == r.key)
                    .any(|(_, w)| w.on);
            (r.clone(), index.labels(&r.key), mine, index.system(&r.key))
        })
        .collect();
    let known = index.known();
    let thrown = index
        .records
        .values()
        .filter(|r| index.system(&r.key).inbox)
        .filter_map(|r| Some((r.key.clone(), index.filter.would_throw(r, &known)?)))
        .collect();
    let looked = index
        .filter
        .taught
        .iter()
        .filter(|t| !t.thrown)
        .map(|t| t.key.clone())
        .collect();
    let last = last.filter(|s| s.lens == lens);
    let before = last
        .map(|s| {
            s.clustering
                .clusters
                .iter()
                .map(|c| {
                    let keys = c.members.iter().map(|&i| s.model.keys[i].clone()).collect();
                    (c.id, c.centre, keys)
                })
                .collect()
        })
        .unwrap_or_default();
    Input {
        version: index.version,
        lens,
        records,
        not_it: index.keep.not_it.clone(),
        layout: index.keep.layouts.get(&lens).cloned(),
        before,
        layout_wanted: layout,
        last: last.map(|s| Last {
            model: s.model.clone(),
            clustering: s.clustering.clone(),
            hues: s.hues.clone(),
            at: s.at.clone(),
            faithfulness: s.faithfulness,
            not_it: s.not_it.clone(),
        }),
        corpus,
        thrown,
        looked,
    }
}

/// The snapshot `input` makes: the model, its clusters and hues, and where
/// each message is. What the model has read follows the index: new mail
/// is read alone, and where no message came, went or changed its labels,
/// the last snapshot's model, clusters and map are kept as they were.
/// Messages stay where the layout kept, or the last snapshot, has them,
/// and new mail is settled among its nearest; a layout is made now if one
/// is wanted and none was kept.
pub fn compute(input: Input) -> Snapshot {
    let given: Vec<model::Given> = input
        .records
        .iter()
        .map(|(r, labels, _, system)| (r, labels.clone(), system.sent))
        .collect();
    let (mut corpus, changed) = match input.corpus {
        Some(mut c) if c.lens == input.lens => {
            let changed = c.follow(&given);
            (c, changed)
        }
        _ => (Corpus::read(&given, input.lens), true),
    };
    let unchanged = input.last.as_ref().filter(|l| {
        !changed
            && l.not_it == input.not_it
            && l.model.len() == given.len()
            && given.iter().all(|(r, labels, _)| {
                l.model
                    .place
                    .get(&r.key)
                    .is_some_and(|&i| l.model.labels[i] == *labels)
            })
    });
    let n = corpus.len();
    let (model, clustering, hues) = match unchanged {
        Some(l) => (l.model.clone(), l.clustering.clone(), l.hues.clone()),
        None => {
            let model = Arc::new(corpus.model(&given, &input.not_it));
            let clustering = model::cluster(&model, &input.before, SEED);
            let hues = model.hues();
            (model, clustering, hues)
        }
    };
    drop(given);
    let (at, fresh, faithfulness) = if let Some(kept) = &input.layout {
        // Where the layout kept has each message, else where the last
        // snapshot had it; the rest settled among their nearest.
        let mut known: Vec<Option<Point>> = model
            .keys
            .iter()
            .map(|k| kept.get(k).map(|&(x, y)| [x, y]))
            .collect();
        if let Some(l) = &input.last
            && let Some(old) = &l.at
        {
            for (i, k) in model.keys.iter().enumerate() {
                if known[i].is_none()
                    && let Some(&j) = l.model.place.get(k)
                {
                    known[i] = old.get(j).copied();
                }
            }
        }
        let new: Vec<usize> = (0..n).filter(|&i| known[i].is_none()).collect();
        let mut at: Vec<Point> = known.iter().map(|p| p.unwrap_or([0.0, 0.0])).collect();
        model::settle(&model, &mut at, &new, SEED);
        // As faithful as it was, where nothing moved or changed.
        let faith = match unchanged {
            Some(l) if l.at.as_ref() == Some(&at) => l.faithfulness,
            _ => None,
        }
        .unwrap_or_else(|| model::faithfulness(&model, &at));
        (Some(at), None, Some(faith))
    } else if input.layout_wanted && n > 0 {
        let at = model::layout(&model, None, SEED);
        let fresh = model
            .keys
            .iter()
            .zip(&at)
            .map(|(k, p)| (k.clone(), p[0], p[1]))
            .collect();
        let faith = model::faithfulness(&model, &at);
        (Some(at), Some(fresh), Some(faith))
    } else {
        (None, None, None)
    };
    let mut ids: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (r, _, _, _) in &input.records {
        if !r.message_id.is_empty() {
            ids.entry(r.message_id.as_str())
                .or_default()
                .push(r.key.account.as_str());
        }
    }
    let twins: Vec<bool> = input
        .records
        .iter()
        .map(|(r, _, _, _)| {
            ids.get(r.message_id.as_str())
                .is_some_and(|a| a.iter().any(|x| *x != r.key.account))
        })
        .collect();
    drop(ids);
    // The dots, in the model's order, made of the records themselves.
    let mut records: Vec<Option<Drawn>> = input
        .records
        .into_iter()
        .zip(twins)
        .map(|((r, labels, mine, system), twin)| Some((r, labels, mine, system, twin)))
        .collect();
    let mut order = vec![0usize; n];
    for (i, slot) in records.iter().enumerate() {
        if let Some((r, ..)) = slot
            && let Some(&at) = model.place.get(&r.key)
        {
            order[at] = i;
        }
    }
    let dots: Vec<Dot> = order
        .into_iter()
        .map(|i| {
            let (r, labels, mine, system, twin) =
                records[i].take().expect("each record a dot once");
            let thrown = input.thrown.get(&r.key).copied();
            let looked = input.looked.contains(&r.key);
            Dot {
                key: r.key,
                from: r.from,
                address: r.address,
                to: r.to,
                subject: r.subject,
                preview: r.preview,
                message_id: r.message_id,
                date: r.date,
                arrived: r.date.unwrap_or(r.first_seen).min(r.first_seen),
                size: r.size,
                flags: r.flags,
                labels,
                mine,
                system,
                places: r.places,
                twin,
                thrown,
                looked,
                parents: r.parents,
                thread: r.thread,
            }
        })
        .collect();
    // One group a label, however many clusters its mail is split between:
    // the first cluster's number, the parts' messages together, and its
    // largest part where its anchor is.
    let mut groups: Vec<Group> = Vec::new();
    let mut of_label: BTreeMap<String, usize> = BTreeMap::new();
    for c in &clustering.clusters {
        let merged = c
            .label
            .as_ref()
            .and_then(|l| of_label.get(&crate::index::fold(l)).copied());
        match merged {
            Some(g) => {
                let g = &mut groups[g];
                if c.members.len() > g.core.len() {
                    g.core = c.members.clone();
                }
                g.members.extend(&c.members);
                g.members.sort_unstable();
            }
            None => {
                if let Some(l) = &c.label {
                    of_label.insert(crate::index::fold(l), groups.len());
                }
                groups.push(Group {
                    id: c.id,
                    members: c.members.clone(),
                    core: c.members.clone(),
                    label: c.label.clone(),
                    words: c.words.clone(),
                    hue: 0.0,
                    unread: 0,
                });
            }
        }
    }
    let mut group_at = vec![0usize; dots.len()];
    for (g, group) in groups.iter_mut().enumerate() {
        group.unread = group
            .members
            .iter()
            .filter(|&&i| !dots[i].flags.seen)
            .count();
        for &i in &group.members {
            group_at[i] = g;
        }
    }
    // A cluster's hue is its members' mean, by angle on the hue circle.
    let groups = groups
        .into_iter()
        .map(|mut g| {
            let (mut x, mut y) = (0f32, 0f32);
            for &i in &g.members {
                x += hues[i].to_radians().cos();
                y += hues[i].to_radians().sin();
            }
            g.hue = y.atan2(x).to_degrees().rem_euclid(360.0);
            g
        })
        .collect();

    let voted = model.voted(|i, l| {
        input
            .not_it
            .contains(&(model.keys[i].clone(), l.to_string()))
    });
    Snapshot {
        version: input.version,
        lens: input.lens,
        voted,
        dots,
        model,
        clustering,
        groups,
        group_at,
        hues,
        at,
        faithfulness,
        fresh,
        not_it: input.not_it,
        corpus: Some(corpus),
    }
}

/// The map of `s` laid out again, begun from where each message is, and
/// laid back onto the old by Procrustes: by the labelled messages, the
/// labels' anchors, where there are three or more, else by all.
pub fn relayout(s: &Snapshot) -> Vec<Point> {
    let Some(old) = &s.at else {
        return model::layout(&s.model, None, SEED);
    };
    let start: Vec<Option<Point>> = old.iter().copied().map(Some).collect();
    let new = model::layout(&s.model, Some(&start), SEED);
    let labelled: Vec<usize> = (0..s.dots.len())
        .filter(|&i| !s.dots[i].labels.is_empty())
        .collect();
    let held: Vec<usize> = if labelled.len() >= 3 {
        labelled
    } else {
        (0..s.dots.len()).collect()
    };
    let a: Vec<Point> = held.iter().map(|&i| new[i]).collect();
    let b: Vec<Point> = held.iter().map(|&i| old[i]).collect();
    model::procrustes(&a, &b, &new)
}

/// `at`, where the messages `keys` would be, as where each message of `s`
/// would be: as `at` has it, or, come since, where `s` has it.
pub fn onto(s: &Snapshot, keys: &[Key], at: &[Point]) -> Option<Vec<Point>> {
    let now = s.at.as_ref()?;
    let by: std::collections::HashMap<&Key, Point> = keys.iter().zip(at.iter().copied()).collect();
    Some(
        s.model
            .keys
            .iter()
            .zip(now)
            .map(|(k, &p)| by.get(k).copied().unwrap_or(p))
            .collect(),
    )
}

/// How many messages of `s` would move noticeably, laid out at `at`: by
/// more than a twentieth of the map's breadth.
pub fn moved(s: &Snapshot, at: &[Point]) -> usize {
    let Some(old) = &s.at else {
        return at.len();
    };
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in old {
        for d in 0..2 {
            lo[d] = lo[d].min(p[d]);
            hi[d] = hi[d].max(p[d]);
        }
    }
    let breadth = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2)).sqrt();
    old.iter()
        .zip(at)
        .filter(|(p, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() > breadth / 20.0)
        .count()
}

/// Lay the map of `index` out again by every lens, from nothing, as the
/// seed has it, and keep the layouts; what to say of it.
pub fn rebuild(index: &mut Index) -> anyhow::Result<String> {
    let mut said = Vec::new();
    for lens in Lens::ALL {
        let mut input = input(index, lens, None, None, true);
        input.layout = None;
        let s = compute(input);
        if let Some(fresh) = s.fresh {
            index.keep(vec![crate::index::Kept::Layout { lens, at: fresh }])?;
        }
        said.push(format!(
            "{}, {:.0}% faithful",
            lens.name(),
            s.faithfulness.unwrap_or(1.0) * 100.0
        ));
    }
    index.flush()?;
    Ok(format!(
        "laid out {} messages by the model {}: {}",
        index.records.len(),
        model::VERSION,
        said.join("; ")
    ))
}
