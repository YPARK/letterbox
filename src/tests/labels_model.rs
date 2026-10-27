//! Labels kept on the servers, and the map's model on the demo's mail.

use super::*;

// ---- labels, kept on the servers ----

#[test]
fn a_label_is_a_keyword_and_back() {
    use crate::index::labels::{keyword, unkeyword};
    assert_eq!(keyword("Grant"), "Grant");
    assert_eq!(keyword("R01/aims:v2"), "R01/aims:v2");
    assert_eq!(keyword("Big Things"), "Big&ACA-Things");
    assert_eq!(keyword("Tom & Jerry"), "Tom&ACA-&-&ACA-Jerry");
    for label in [
        "Grant",
        "Big Things",
        "Tom & Jerry",
        "Entwürfe",
        "日本語のラベル",
        "a(b)c",
    ] {
        let k = keyword(label);
        assert!(
            k.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._+/:-&,".contains(c)),
            "{k}"
        );
        assert_eq!(unkeyword(&k), label);
    }
    let long = "ünïcode ".repeat(20);
    let k = keyword(&long);
    assert!(k.len() <= crate::index::labels::KEYWORD, "{}", k.len());
    assert!(long.starts_with(&unkeyword(&k)));
}

#[test]
fn folders_are_labels_at_every_level() {
    use crate::index::FolderState;
    use crate::index::labels::{of_folder, server, system};
    let state = |role| FolderState {
        role: Some(role),
        delimiter: Some("/".into()),
        ..Default::default()
    };
    assert_eq!(
        of_folder("Projects/Grant", Some(&state(Role::Other))),
        ["Projects", "Grant"]
    );
    assert_eq!(of_folder("INBOX/Lab", Some(&state(Role::Other))), ["Lab"]);
    assert_eq!(
        of_folder("Entw&APw-rfe", Some(&state(Role::Other))),
        ["Entwürfe"]
    );
    assert!(of_folder("Sent Items", Some(&state(Role::Sent))).is_empty());

    let mut r = record("a", "Photo 51");
    r.places.push(index::Place {
        folder: "Projects/Grant".into(),
        validity: 7,
        uid: 3,
    });
    r.server = vec!["Lab".into(), "$Junk".into(), "NotJunk".into()];
    let folders = [
        ("INBOX", state(Role::Inbox)),
        ("Projects/Grant", state(Role::Other)),
    ]
    .into_iter()
    .map(|(f, s)| (("a@b.c".to_string(), f.to_string()), s))
    .collect();
    // A mail program's own keywords are none of yours.
    assert_eq!(server(&r, &folders), ["Projects", "Grant", "Lab"]);
    assert!(system(&r, &folders).inbox);
    // Gmail's labels nest as its folders do; its system labels are not.
    r.key.id = "gm:1".into();
    r.places.truncate(1);
    r.server = vec![
        "\\Inbox".into(),
        "\\Important".into(),
        "Projects/Grant".into(),
    ];
    assert_eq!(server(&r, &folders), ["Projects", "Grant"]);
    assert!(system(&r, &folders).important);
}

/// The message of `index` with `subject`: its key, labels, and whether it
/// is in the Inbox.
pub(super) fn labelled(index: &Mutex<Index>, subject: &str) -> (index::Key, Vec<String>, bool) {
    let index = index.lock().unwrap();
    let r = index
        .records
        .values()
        .find(|r| r.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}"));
    (
        r.key.clone(),
        index.labels(&r.key),
        index.system(&r.key).inbox,
    )
}

/// Say that the message `key` is filed under `label`, as the app does
/// before its server is asked.
pub(super) fn pending(index: &Mutex<Index>, key: &index::Key, label: &str) {
    index
        .lock()
        .unwrap()
        .keep(vec![Kept::Pending {
            key: key.clone(),
            label: label.into(),
            on: true,
            at: 1_000_000,
        }])
        .unwrap();
}

#[test]
fn filing_on_a_server_of_keywords_labels_it_and_archives_it() {
    let (account, boxes) = serve_boxes(boxes_in(Mode::Keywords));
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let (key, labels, inbox) = labelled(&index, "Photo 51");
    assert!(labels.is_empty() && inbox);
    pending(&index, &key, "Big Things");
    let (_, labels, inbox) = labelled(&index, "Photo 51");
    assert_eq!((labels, inbox), (vec!["Big Things".to_string()], false));
    let filed = imap.file("INBOX", 11, "Big Things", "").unwrap();
    let (to, uid) = filed.moved.unwrap();
    assert_eq!(to, "Archive");
    let fake = boxes.lock().unwrap();
    let archived = fake.iter().find(|b| b.name == "Archive").unwrap();
    assert_eq!(archived.letters[0].uid, uid);
    assert!(archived.letters[0].has("Big&ACA-Things"));
    drop(fake);
    sync_at(&mut imap, &index, 1_000_100);
    assert!(index.lock().unwrap().keep.pending.is_empty(), "settled");
    let (still, labels, inbox) = labelled(&index, "Photo 51");
    assert_eq!(
        (still, labels, inbox),
        (key, vec!["Big Things".to_string()], false)
    );

    // Renamed, the keyword is changed wherever it is.
    imap.rename_label("Big Things", "Funding").unwrap();
    sync_at(&mut imap, &index, 1_000_200);
    assert_eq!(labelled(&index, "Photo 51").1, ["Funding"]);
}

#[test]
fn filing_on_gmail_gives_the_label_and_takes_the_inbox_off() {
    let (account, boxes) = serve_boxes(gmail_boxes());
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    let (key, _, inbox) = labelled(&index, "R01 resubmission");
    assert!(inbox);
    pending(&index, &key, "Grant");
    let filed = imap.file("[Gmail]/All Mail", 22, "Grant", "").unwrap();
    assert_eq!(filed.moved, None);
    let labels = boxes.lock().unwrap()[2].letters[1].labels.clone();
    assert_eq!(labels, ["\\Important", "Grant"]);
    sync_at(&mut imap, &index, 1_000_100);
    assert!(index.lock().unwrap().keep.pending.is_empty());
    let (_, labels, inbox) = labelled(&index, "R01 resubmission");
    assert_eq!((labels, inbox), (vec!["Grant".to_string()], false));
    // A label renamed is renamed on every message it is on.
    imap.rename_label("Grant", "Funding").unwrap();
    sync_at(&mut imap, &index, 1_000_200);
    assert_eq!(labelled(&index, "R01 resubmission").1, ["Funding"]);
}

#[test]
fn what_one_machine_does_on_gmail_another_sees_at_its_next_pass() {
    let (account, _boxes) = serve_boxes(gmail_boxes());
    let (here, there) = (Mutex::new(Index::memory()), Mutex::new(Index::memory()));
    let (mut a, mut b) = (Imap::new(account.clone()), Imap::new(account));
    sync_once(&mut a, &here);
    sync_once(&mut b, &there);
    assert!(labelled(&there, "Aims page, third pass").2);
    // On one machine: the Aims page archived, the R01 trashed, and the
    // reply filed.
    a.archive("[Gmail]/All Mail", 21, "").unwrap();
    a.relocate("[Gmail]/All Mail", 22, "[Gmail]/Trash", "")
        .unwrap();
    // The other, at its next pass and the one after: as it was left.
    for now in [1_000_100, 1_000_200] {
        sync_at(&mut b, &there, now);
        assert!(!labelled(&there, "Aims page, third pass").2, "archived");
        let index = there.lock().unwrap();
        assert!(
            index
                .records
                .values()
                .all(|r| r.subject != "R01 resubmission"),
            "trashed"
        );
    }
}

#[test]
fn filing_on_a_server_of_folders_moves_it_to_the_folder_the_label_is() {
    let (account, boxes) = server();
    let index = Mutex::new(Index::memory());
    let mut imap = Imap::new(account);
    sync_once(&mut imap, &index);
    // Grant is Projects/Grant: the folder of that name, not a new one.
    let filed = imap.file("INBOX", 11, "grant", "").unwrap();
    assert_eq!(filed.moved.as_ref().unwrap().0, "Projects/Grant");
    assert!(filed.on_server);
    sync_once(&mut imap, &index);
    assert_eq!(labelled(&index, "Photo 51").1, ["Projects", "Grant"]);
    // Out of the Inbox, a second label is letterbox's to keep.
    let (to, uid) = filed.moved.unwrap();
    let again = imap.file(&to, uid, "Lab", "").unwrap();
    assert_eq!((again.moved, again.on_server), (None, false));
    // No folder of the name: one is made, at the top.
    let new = imap.file("INBOX", 14, "Reading list", "").unwrap();
    assert_eq!(new.moved.unwrap().0, "Reading list");
    assert!(imap.label("INBOX", 12, "Lab", true).is_err());
    imap.rename_label("Grant", "Funding").unwrap();
    let names: Vec<String> = boxes
        .lock()
        .unwrap()
        .iter()
        .map(|b| b.name.clone())
        .collect();
    assert!(names.contains(&"Projects/Funding".to_string()), "{names:?}");
}

#[test]
fn the_demo_files_as_a_server_of_keywords_does() {
    let mut demo = Demo::new();
    let mut twin = demo.twin().unwrap();
    let index = Mutex::new(Index::memory());
    sync_once(twin.as_mut(), &index);
    let (key, _, inbox) = labelled(&index, "Aims page, third pass");
    assert!(inbox);
    let uid = demo
        .page("INBOX", 0, 100)
        .unwrap()
        .summaries
        .into_iter()
        .find(|s| s.subject == "Aims page, third pass")
        .unwrap()
        .uid;
    pending(&index, &key, "Grant");
    let filed = demo.file("INBOX", uid, "Grant", "").unwrap();
    assert_eq!(filed.moved.unwrap().0, "Archive");
    sync_at(twin.as_mut(), &index, 1_000_100);
    assert!(index.lock().unwrap().keep.pending.is_empty());
    let (still, labels, inbox) = labelled(&index, "Aims page, third pass");
    assert_eq!(
        (still, labels, inbox),
        (key, vec!["Grant".to_string()], false)
    );
    // The Grant folder's letters are Projects and Grant; the keyword is the
    // same label, renamed with the folder.
    assert_eq!(
        labelled(&index, "Letter of support, signed").1,
        ["Projects", "Grant"]
    );
    demo.rename_label("Grant", "Funding").unwrap();
    sync_at(twin.as_mut(), &index, 1_000_200);
    assert_eq!(labelled(&index, "Aims page, third pass").1, ["Funding"]);
    assert_eq!(
        labelled(&index, "Letter of support, signed").1,
        ["Projects", "Funding"]
    );
}

// ---- the model, on the demo's mail ----

/// The demo's mail, as the index has it.
pub(super) fn demo_index() -> Index {
    let demo = Demo::new();
    let mut twin = demo.twin().unwrap();
    let index = Mutex::new(Index::memory());
    sync_once(twin.as_mut(), &index);
    index.into_inner().unwrap()
}

/// The model of `index`, read by `lens`.
pub(super) fn model_of(index: &Index, lens: Lens) -> crate::model::Model {
    let given: Vec<crate::model::Given> = index
        .records
        .values()
        .map(|r| (r, index.labels(&r.key), index.system(&r.key).sent))
        .collect();
    crate::model::Model::build(&given, &index.keep.not_it, lens)
}

/// The grant's mail, by subject.
pub(super) const GRANT_MAIL: [&str; 8] = [
    "R01 resubmission: internal deadline 3 November",
    "Aims page, third pass",
    "Budget justification: the sequencing line",
    "Subaward paperwork for the second site",
    "Letter of support, signed",
    "Re: Aim 2 and the spatial data",
    "Annual progress report due 15 November",
    "Biosketch format changed again",
];

pub(super) fn at_subject(index: &Index, m: &crate::model::Model, subject: &str) -> usize {
    m.keys
        .iter()
        .position(|k| index.records[k].subject == subject)
        .unwrap_or_else(|| panic!("no {subject}"))
}

#[test]
fn the_grant_mail_clusters_together() {
    let index = demo_index();
    let m = model_of(&index, Lens::Both);
    let c = crate::model::cluster(&m, &[], 1);
    let aims = c.of[at_subject(&index, &m, "Aims page, third pass")];
    let together = GRANT_MAIL
        .iter()
        .filter(|s| c.of[at_subject(&index, &m, s)] == aims)
        .count();
    assert!(together >= 6, "{together} of the grant's 8 together");
    assert_eq!(c.clusters[aims].label.as_deref(), Some("Grant"));
    // The robots' mail is a cluster of its own, the grant's apart from it.
    let build = c.of[at_subject(&index, &m, "Build failed: letterbox main (clippy)")];
    let passed = c.of[at_subject(&index, &m, "Build passed: letterbox main")];
    assert_eq!(build, passed);
    assert_ne!(build, aims);
    // A cluster keeps its number from one clustering to the next.
    let before: Vec<_> = c
        .clusters
        .iter()
        .map(|cl| {
            let keys = cl.members.iter().map(|&i| m.keys[i].clone()).collect();
            (cl.id, cl.centre, keys)
        })
        .collect();
    let again = crate::model::cluster(&m, &before, 1);
    let ids =
        |c: &crate::model::Clustering| -> Vec<u32> { c.clusters.iter().map(|x| x.id).collect() };
    assert_eq!(ids(&again), ids(&c));
    assert_eq!(
        again.clusters[again.of[at_subject(&index, &m, "Aims page, third pass")]].id,
        c.clusters[aims].id
    );
}

#[test]
fn an_answer_lies_with_the_mail_it_answers_not_with_the_rest_sent() {
    let index = demo_index();
    let mira = "mira.okafor@uni.example";
    let reply = "Re: Biosketch format changed again";
    let people = model_of(&index, Lens::People);
    let r = at_subject(&index, &people, reply);
    assert!(index.system(&people.keys[r]).sent);
    let near = people.nearest(r, 3);
    let from = |i: usize| index.records[&people.keys[i]].address.as_str();
    assert!(
        near.iter().all(|&(i, _)| from(i) == mira),
        "{:?}",
        near.iter().map(|&(i, _)| from(i)).collect::<Vec<_>>()
    );
    // Read by both, it is among Mira's mail, near what it answers.
    let both = model_of(&index, Lens::Both);
    let r = at_subject(&index, &both, reply);
    let answered = at_subject(&index, &both, "Biosketch format changed again");
    let near = both.nearest(r, 2);
    assert_eq!(index.records[&both.keys[near[0].0]].address, mira);
    assert!(near.iter().any(|&(i, _)| i == answered));
    // Mail sent to different people is not drawn together for being sent.
    for m in [&people, &both] {
        let sent: Vec<usize> = (0..m.len())
            .filter(|&i| index.system(&m.keys[i]).sent)
            .collect();
        assert!(sent.len() >= 4);
        for &i in &sent {
            let (j, _) = m.nearest(i, 1)[0];
            assert!(
                !sent.contains(&j),
                "{} by {}",
                index.records[&m.keys[i]].subject,
                index.records[&m.keys[j]].subject
            );
        }
    }
}

#[test]
fn the_same_seed_lays_out_the_same_map() {
    let index = demo_index();
    let m = model_of(&index, Lens::Both);
    let a = crate::model::layout(&m, None, 1);
    let b = crate::model::layout(&m, None, 1);
    let bits = |p: &[crate::model::Point]| -> Vec<u32> {
        p.iter()
            .flat_map(|q| [q[0].to_bits(), q[1].to_bits()])
            .collect()
    };
    assert_eq!(bits(&a), bits(&b));
    assert_ne!(bits(&a), bits(&crate::model::layout(&m, None, 2)));
    // Faithful to the angles: most of each message's nearest are near it
    // on the map too, far more than chance would have it.
    let f = crate::model::faithfulness(&m, &a);
    let chance = 10.0 / (m.len() - 1) as f32;
    assert!(
        f > 2.5 * chance,
        "faithfulness {f}, against {chance} by chance"
    );
    // Begun from where it was, it moves little.
    let start: Vec<Option<crate::model::Point>> = a.iter().copied().map(Some).collect();
    let gentle = crate::model::layout(&m, Some(&start), 1);
    let fitted = crate::model::procrustes(&gentle, &a, &gentle);
    let spread: f32 = a.iter().map(|p| p[0].abs() + p[1].abs()).sum::<f32>() / a.len() as f32;
    let moved: f32 = fitted
        .iter()
        .zip(&a)
        .map(|(p, q)| (p[0] - q[0]).abs() + (p[1] - q[1]).abs())
        .sum::<f32>()
        / a.len() as f32;
    assert!(
        moved < spread,
        "moved {moved}, against a spread of {spread}"
    );
}

#[test]
fn the_map_is_the_same_on_any_number_of_threads() {
    let index = demo_index();
    let on = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let m = model_of(&index, Lens::Both);
                let c = crate::model::cluster(&m, &[], 1);
                assert!(m.vectors.iter().all(|v| v.iter().all(|x| x.is_finite())));
                let mut at = crate::model::layout(&m, None, 1);
                at.push([m.vectors[0][0], c.of.iter().sum::<usize>() as f32]);
                at
            })
    };
    let bits = |p: &[crate::model::Point]| -> Vec<u32> {
        p.iter()
            .flat_map(|q| [q[0].to_bits(), q[1].to_bits()])
            .collect()
    };
    assert_eq!(bits(&on(1)), bits(&on(4)));
}

#[test]
fn the_nearest_found_by_tokens_are_mostly_the_nearest_of_all() {
    // Mail enough that common tokens are looked through a window only.
    let (records, _) = made_mail(4_000, 3);
    let given: Vec<crate::model::Given> = records.iter().map(|r| (r, Vec::new(), false)).collect();
    let m = crate::model::Model::build(&given, &Default::default(), Lens::Both);
    let every = crate::model::nearest_by_every_pair(&given, Lens::Both, 15, (0..4_000).step_by(20));
    let mut shared = 0;
    let mut all = 0;
    for (i, best) in every {
        shared += best
            .iter()
            .filter(|x| m.near[i].iter().any(|y| y.0 == x.0))
            .count();
        all += best.len();
    }
    let recall = shared as f32 / all as f32;
    assert!(recall > 0.9, "{recall} of the nearest found");
}

#[test]
fn a_new_message_lands_among_its_kind() {
    let mut index = demo_index();
    let m = model_of(&index, Lens::Both);
    let laid = crate::model::layout(&m, None, 1);
    // A build that failed, new: placed among the others, by its nearest.
    let indexed = index::record::Indexed {
        uid: 999,
        size: 300,
        raw: raw(
            "CI <ci@runner.example>",
            "Build failed: letterbox main (test)",
            "Content-Type: text/plain; charset=utf-8\r\nList-Id: <ci.runner.example>\r\n",
            "test map::cells failed.\r\n",
        ),
        ..Default::default()
    };
    let r = index::Record::of("demo", "INBOX", 1, &indexed, 0);
    let key = r.key.clone();
    index.cache(vec![Cached::Upsert(Box::new(r))]).unwrap();
    let m2 = model_of(&index, Lens::Both);
    let known: Vec<Option<crate::model::Point>> = m2
        .keys
        .iter()
        .map(|k| m.keys.iter().position(|x| x == k).map(|i| laid[i]))
        .collect();
    let new = m2.keys.iter().position(|k| *k == key).unwrap();
    let at = crate::model::place(&m2, new, &known);
    let mut by_place: Vec<(f32, usize)> = known
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.map(|p| ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2), i)))
        .collect();
    by_place.sort_by(|a, b| a.0.total_cmp(&b.0));
    let nearest = &index.records[&m2.keys[by_place[0].1]];
    assert_eq!(nearest.address, "ci@runner.example", "{}", nearest.subject);
}

#[test]
fn filing_one_grant_message_makes_the_others_suggestions() {
    let mut index = demo_index();
    let before = model_of(&index, Lens::Both);
    let r01 = before.keys[at_subject(&index, &before, GRANT_MAIL[0])].clone();
    let budget = at_subject(&index, &before, GRANT_MAIL[2]);
    index
        .keep(vec![Kept::Pending {
            key: r01.clone(),
            label: "Grant".into(),
            on: true,
            at: 0,
        }])
        .unwrap();
    let m = model_of(&index, Lens::Both);
    let not_it = index.keep.not_it.clone();
    let said: Vec<&str> = m
        .suggestions("Grant", &not_it)
        .iter()
        .map(|&(i, _)| index.records[&m.keys[i]].subject.as_str())
        .collect();
    assert!(!said.contains(&GRANT_MAIL[0]), "filed, not suggested");
    // The budget, and your answer to Mira about the biosketch; not the
    // aims page, which lies on the edge of what is near.
    for s in [GRANT_MAIL[2], "Re: Biosketch format changed again"] {
        assert!(said.contains(&s), "{s} in {said:?}");
    }
    // Your answer to Mira about the biosketch is the grant's too.
    let grant = said
        .iter()
        .take(2)
        .filter(|s| GRANT_MAIL.contains(s) || s.contains("Biosketch"))
        .count();
    assert_eq!(grant, 2, "the nearest are the grant's: {said:?}");
    let now = crate::model::degrees(&m.vectors[budget], &m.label_vectors["grant"].1);
    // Said not to be: no longer suggested, and pushed away.
    let budget_key = m.keys[budget].clone();
    index
        .keep(vec![Kept::NotIt {
            key: budget_key,
            label: "Grant".into(),
        }])
        .unwrap();
    let m = model_of(&index, Lens::Both);
    let said: Vec<&str> = m
        .suggestions("Grant", &index.keep.not_it)
        .iter()
        .map(|&(i, _)| index.records[&m.keys[i]].subject.as_str())
        .collect();
    assert!(!said.contains(&GRANT_MAIL[2]), "{said:?}");
    let pushed = crate::model::degrees(&m.vectors[budget], &m.label_vectors["grant"].1);
    assert!(pushed > now, "pushed away: {now} to {pushed}");
}

#[test]
fn two_rebuilds_are_byte_identical() {
    let build = || {
        let index = demo_index();
        let m = model_of(&index, Lens::Both);
        let c = crate::model::cluster(&m, &[], 1);
        let laid = crate::model::layout(&m, None, 1);
        let hues = m.hues();
        let mut bytes = Vec::new();
        for (i, p) in laid.iter().enumerate() {
            bytes.extend(p[0].to_le_bytes());
            bytes.extend(p[1].to_le_bytes());
            bytes.extend(hues[i].to_le_bytes());
            bytes.extend((c.of[i] as u32).to_le_bytes());
        }
        bytes
    };
    assert_eq!(build(), build());
}

#[test]
fn hues_follow_the_angle_and_procrustes_undoes_a_turn() {
    let index = demo_index();
    let m = model_of(&index, Lens::Both);
    let hues = m.hues();
    assert!(hues.iter().all(|h| (0.0..360.0).contains(h)));
    let a = at_subject(&index, &m, "Build failed: letterbox main (clippy)");
    let b = at_subject(&index, &m, "Build failed: faba (test timeout)");
    let apart = (hues[a] - hues[b]).abs();
    assert!(
        apart.min(360.0 - apart) < 60.0,
        "{} and {}",
        hues[a],
        hues[b]
    );
    // Turned, mirrored, grown and moved: laid back where it was.
    let old: Vec<crate::model::Point> = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 2.0], [3.0, 1.0]];
    let new: Vec<crate::model::Point> = old
        .iter()
        .map(|p| [2.0 * p[1] + 5.0, 2.0 * p[0] - 1.0])
        .collect();
    let back = crate::model::procrustes(&new, &old, &new);
    for (p, q) in back.iter().zip(&old) {
        assert!(
            (p[0] - q[0]).abs() < 1e-4 && (p[1] - q[1]).abs() < 1e-4,
            "{p:?} {q:?}"
        );
    }
}
