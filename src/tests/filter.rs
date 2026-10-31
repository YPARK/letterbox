//! What you throw away, learnt; how it would have done; what tells it
//! apart; and what was learnt long ago fading. What is thrown away as
//! junk is `junk`'s.

use super::*;

// ---- What you throw away, learnt ----

/// A message from `address` about `subject`, by `id`.
pub(super) fn sent_by(id: &str, address: &str, subject: &str) -> index::Record {
    let mut r = record(id, subject);
    r.address = address.to_string();
    r.from = address.to_string();
    r.preview = String::new();
    r
}

/// What `n` mails from a shop and `n` from colleagues teach, each a subject
/// of its own.
pub(super) fn taught(n: usize) -> crate::filter::Filter {
    let mut f = crate::filter::Filter::default();
    for k in 0..n {
        for (address, subject, thrown) in [
            (
                "deals@shop.example",
                format!("Save {k} percent on item {k}"),
                true,
            ),
            (
                "ada@analytical.example",
                format!("Engine notes, part {k}"),
                false,
            ),
        ] {
            let r = sent_by(&format!("{k}{thrown}"), address, &subject);
            f.learn(crate::filter::Taught {
                key: r.key.clone(),
                thrown,
                tokens: crate::filter::Filter::tokens(&r, &Default::default()),
                message_id: r.message_id.clone(),
                seeded: false,
                ..Default::default()
            });
        }
    }
    f
}

#[test]
fn what_you_throw_away_is_learnt_by_who_sent_it_not_its_subject() {
    // Too little taught: nothing said.
    let few = taught(crate::filter::READY as usize - 1);
    assert!(!few.ready());
    let new_deal = sent_by("x", "deals@shop.example", "A brand new offer, never seen");
    assert_eq!(few.would_throw(&new_deal, &Default::default()), None);
    // Enough: the shop's new mail, of a subject never seen, leans to thrown
    // away, its words never seen weighing nothing; once taught more, sure
    // enough to say so. Ada's would not be.
    let f = taught(40);
    assert!(f.ready());
    assert!(
        f.odds(&crate::filter::Filter::tokens(
            &new_deal,
            &Default::default()
        )) > 0.8
    );
    let f = taught(100);
    let p = f
        .would_throw(&new_deal, &Default::default())
        .expect("thrown away");
    assert!(p >= crate::filter::SURE, "{p}");
    let from_ada = sent_by(
        "y",
        "ada@analytical.example",
        "A brand new offer, never seen",
    );
    assert_eq!(f.would_throw(&from_ada, &Default::default()), None);
    assert!(
        f.odds(&crate::filter::Filter::tokens(
            &from_ada,
            &Default::default()
        )) < 0.5
    );
    // The same taught in the same order: the same odds.
    let again = taught(100);
    assert_eq!(
        f.odds(&crate::filter::Filter::tokens(
            &new_deal,
            &Default::default()
        )),
        again.odds(&crate::filter::Filter::tokens(
            &new_deal,
            &Default::default()
        ))
    );
}

#[test]
fn what_was_learnt_is_written_down_and_learnt_again_as_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let deal = sent_by("d", "deals@shop.example", "Save now");
    let note = sent_by("n", "ada@analytical.example", "Engine notes");
    let odds;
    {
        let mut index = Index::open_with(dir.path(), &id);
        index
            .cache(vec![
                Cached::Upsert(Box::new(deal.clone())),
                Cached::Upsert(Box::new(note.clone())),
            ])
            .unwrap();
        for _ in 0..3 {
            index.teach(&deal.key, true).unwrap();
            index.teach(&note.key, false).unwrap();
        }
        // One not indexed teaches nothing.
        index.teach(&record("gone", "Gone").key, true).unwrap();
        assert_eq!((index.filter.thrown, index.filter.kept), (3, 3));
        odds = index
            .filter
            .odds(&crate::filter::Filter::tokens(&deal, &Default::default()));
        index.flush().unwrap();
    }
    let index = Index::open_with(dir.path(), &id);
    assert_eq!((index.filter.thrown, index.filter.kept), (3, 3));
    assert_eq!(
        index
            .filter
            .odds(&crate::filter::Filter::tokens(&deal, &Default::default())),
        odds
    );
    // Its key, by where it is.
    assert!(index.key_at("a@b.c", "INBOX", 1).is_some());
}

#[test]
fn trashing_teaches_and_the_inbox_looked_through_says_what_would_be_thrown_away() {
    let mut app = demo_map();
    let s = app.map.snapshot.clone().unwrap();
    // d on the map: thrown away, and learnt.
    let ci = s
        .dots
        .iter()
        .position(|d| d.subject.starts_with("Build passed"))
        .unwrap();
    super::mapsafety::down_to(&mut app, &s.dots[ci].key.clone());
    press(&mut app, &["d"]);
    assert_eq!(app.index.lock().unwrap().filter.thrown, 1);
    // Taught enough that the CI's mail is thrown away, and the rest kept.
    let (ci_keys, kept_keys): (Vec<_>, Vec<_>) = {
        let index = app.index.lock().unwrap();
        let ci: Vec<_> = index
            .records
            .values()
            .filter(|r| r.from == "CI")
            .map(|r| r.key.clone())
            .collect();
        let kept: Vec<_> = index
            .records
            .values()
            .filter(|r| r.from != "CI")
            .map(|r| r.key.clone())
            .take(25)
            .collect();
        (ci, kept)
    };
    {
        let mut index = app.index.lock().unwrap();
        for k in 0..25 {
            index.teach(&ci_keys[k % ci_keys.len()], true).unwrap();
            index.teach(&kept_keys[k], false).unwrap();
        }
    }
    app.settle(Duration::from_secs(5));
    let _ = app::screen(&mut app, 170, 40);
    app.settle(Duration::from_secs(5));
    press(&mut app, &["i"]);
    let s = app.map.snapshot.clone().unwrap();
    let rows = app.review_rows();
    let failed = rows
        .iter()
        .copied()
        .find(|&i| s.dots[i].from == "CI")
        .expect("the CI's mail in the Inbox");
    assert!(
        s.dots[failed].thrown.is_some(),
        "{:?}",
        s.dots[failed].thrown
    );
    // Suggested, not chosen: hinted at, and nothing thrown away by $.
    assert_eq!(app.choice(failed), app::Choice::Discard);
    assert_eq!(app.chosen(failed), None);
    let screen = app::screen(&mut app, 170, 60);
    assert!(screen.contains("[?] discard "), "{screen}");
    assert!(screen.contains('%'), "{screen}");
}

#[test]
fn only_those_chosen_are_done_and_those_kept_come_last_next_time() {
    let mut app = demo_map();
    press(&mut app, &["i"]);
    let rows = app.review_rows();
    let s = app.map.snapshot.clone().unwrap();
    let first = s.dots[rows[0]].key.clone();
    // Kept, the first; the rest not looked at.
    press(&mut app, &["<space>", "<c-c>", "<c-c>", "y"]);
    assert!(app.status.contains("1 kept"), "{}", app.status);
    assert_eq!(app.index.lock().unwrap().filter.kept, 1);
    app.settle(Duration::from_secs(5));
    let _ = app::screen(&mut app, 170, 40);
    app.settle(Duration::from_secs(5));
    // Still in the Inbox, all of them; the one kept after the rest.
    let rows = app.review_rows();
    assert_eq!(rows.len(), INBOX);
    let s = app.map.snapshot.clone().unwrap();
    assert_eq!(s.dots[*rows.last().unwrap()].key, first);
    let screen = app::screen(&mut app, 170, 40);
    assert!(screen.contains("1 kept before"), "{screen}");
}

// ---- how what is learnt would have done ----

#[test]
fn the_filter_is_graded_on_each_decision_before_it_is_learnt() {
    let mut f = taught(40);
    // A message from the shop you kept, last: it would have been thrown
    // away, and is graded so, not as what it taught.
    let kept = sent_by("k", "deals@shop.example", "Your receipt");
    f.learn(crate::filter::Taught {
        key: kept.key.clone(),
        thrown: false,
        tokens: crate::filter::Filter::tokens(&kept, &Default::default()),
        message_id: String::new(),
        seeded: false,
        ..Default::default()
    });
    let said = crate::filter::Filter::check(&f.taught);
    // Nothing said before twenty of each: one of each a turn.
    let first = 2 * crate::filter::READY as usize;
    assert_eq!(said.len(), f.taught.len() - first, "{}", said.len());
    let last = said.last().unwrap();
    assert!(!last.thrown && last.odds >= crate::filter::SURE, "{last:?}");
    assert_eq!(last.from, "deals@shop.example");
    let g = crate::filter::grade(&said, crate::filter::SURE);
    assert_eq!(g.wrong, 1, "{g:?}");
    assert!(g.caught >= 4, "{g:?}");
    assert_eq!(g.right, said.iter().filter(|s| !s.thrown).count() - 1);
}

#[test]
fn a_message_put_back_is_learnt_as_kept_though_the_index_let_it_go() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_with(dir.path(), &identity());
    let mut deal = sent_by("d", "deals@shop.example", "Save now");
    deal.message_id = "<d@shop>".into();
    index
        .cache(vec![Cached::Upsert(Box::new(deal.clone()))])
        .unwrap();
    index.teach(&deal.key, true).unwrap();
    // Known by its Message-ID; one never thrown away is not.
    assert!(!index.put_back(&deal.key.account, "<other@shop>").unwrap());
    assert!(index.put_back(&deal.key.account, "<d@shop>").unwrap());
    assert_eq!((index.filter.thrown, index.filter.kept), (1, 1));
    let last = index.filter.taught.last().unwrap();
    assert!(!last.thrown && last.tokens == index.filter.taught[0].tokens);
    // Once kept, put back again teaches nothing more.
    assert!(!index.put_back(&deal.key.account, "<d@shop>").unwrap());
}

#[test]
fn archiving_answering_and_putting_back_teach_that_it_was_kept() {
    let mut app = demo_map();
    let kept = |app: &App| app.index.lock().unwrap().filter.kept;
    let thrown = |app: &App| app.index.lock().unwrap().filter.thrown;
    // e on the map, down at a message: archived, kept.
    let key = {
        let s = app.map.snapshot.clone().unwrap();
        s.dots[app.map.picked().unwrap()].key.clone()
    };
    super::mapsafety::down_to(&mut app, &key);
    press(&mut app, &["e"]);
    assert_eq!(kept(&app), 1);
    // r, in the list: answered, kept.
    press(&mut app, &["M", "r"]);
    assert_eq!(kept(&app), 2);
    app.compose = None;
    // d, then d on it in the Trash: put back, kept.
    let subject = app.current().unwrap().summary.subject.clone();
    press(&mut app, &["d"]);
    assert_eq!(thrown(&app), 1);
    app.settle(Duration::from_secs(5));
    app.focus = app::Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(|r| match *r {
            app::Row::Folder(a, i) => app.accounts[a].folders[i].name == "Trash",
            _ => false,
        })
        .expect("a Trash in the sidebar");
    press(&mut app, &["<enter>"]);
    app.settle(Duration::from_secs(5));
    let at = app
        .shown
        .iter()
        .position(|l| l.summary.subject == subject)
        .expect("in the Trash");
    app.list.select(Some(at));
    app.focus = app::Focus::List;
    press(&mut app, &["d"]);
    assert!(app.status.contains("put back"), "{}", app.status);
    assert_eq!(kept(&app), 3);
}

#[test]
fn what_is_in_the_trash_and_under_labels_is_learnt_once_and_graded() {
    let mut index = Index::memory();
    let mut store = Demo::new();
    // Two thrown away, one read first: dealt with, not unwanted, so left
    // out.
    let page = store.page("INBOX", 0, 40).unwrap().summaries;
    let unread = page.iter().find(|m| !m.flags.seen).unwrap().clone();
    let read = page.iter().find(|m| m.flags.seen).unwrap().clone();
    for m in [&unread, &read] {
        store.relocate("INBOX", m.uid, "Trash", "").unwrap();
    }
    let now = jiff::Timestamp::now().as_second();
    // What letterbox itself moved there is left out: taught as it was
    // trashed, alone, or not at all, trashed with others.
    let ours: std::collections::HashSet<String> = [unread.message_id.clone()].into();
    let none = crate::trash_seeds(&mut store, &Default::default(), 10, now, &ours).unwrap();
    assert!(none.is_empty(), "{none:?}");
    let thrown = crate::trash_seeds(
        &mut store,
        &Default::default(),
        10,
        now,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(thrown.len(), 1, "{thrown:?}");
    assert!(thrown[0].1.thrown && thrown[0].1.seeded);
    assert_eq!(thrown[0].1.message_id, unread.message_id);
    // Filed under labels, kept.
    let shared = std::sync::Mutex::new(index);
    let stop = std::sync::atomic::AtomicBool::new(false);
    index::sync::pass(&mut store, &shared, now, &stop, &mut |_| {}).unwrap();
    index = shared.into_inner().unwrap();
    let kept = index.kept_seeds(100);
    assert!(!kept.is_empty());
    assert!(kept.iter().all(|(_, t)| !t.thrown && t.seeded));
    let seeds: Vec<_> = thrown.into_iter().chain(kept).map(|(_, t)| t).collect();
    let n = seeds.len();
    assert_eq!(index.seed(seeds.clone()).unwrap(), n);
    // Again: nothing more.
    assert_eq!(index.seed(seeds).unwrap(), 0);
    let said = crate::check(&index, crate::filter::DECAY);
    assert!(said.contains(&format!("{n} decisions")), "{said}");
    assert!(said.contains("read from the Trash"), "{said}");
    // Forgotten: only your own decisions are left.
    assert_eq!(index.unseed().unwrap(), n);
    assert!(index.filter.taught.is_empty());
    assert_eq!((index.filter.thrown, index.filter.kept), (0, 0));
}

// ---- what else tells mail thrown away from mail kept ----

/// A message from `from` about `subject`, with `headers` beside its own.
pub(super) fn with_headers(id: &str, from: &str, subject: &str, headers: &str) -> index::Record {
    let raw = format!(
        "From: {from}\r\nTo: me@example.com\r\nSubject: {subject}\r\n\
         Message-ID: <{id}@example.com>\r\n{headers}\r\nHello.\r\n"
    );
    let indexed = index::record::Indexed {
        uid: 1,
        size: 100,
        raw: raw.into_bytes(),
        ..Default::default()
    };
    index::Record::of("a@b.c", "INBOX", 7, &indexed, 1_000)
}

#[test]
fn mail_sent_to_many_is_known_by_its_headers() {
    let plain = with_headers("p", "ada@analytical.example", "Notes", "");
    assert!(!plain.bulk);
    let unsubscribe = with_headers(
        "u",
        "deals@shop.example",
        "Sale",
        "List-Unsubscribe: <mailto:out@shop.example>\r\n",
    );
    assert!(unsubscribe.bulk);
    let precedence = with_headers("b", "news@shop.example", "News", "Precedence: bulk\r\n");
    assert!(precedence.bulk);
    // Asked of every server.
    let items = crate::imap::indexed_items(false);
    assert!(items.contains("LIST-UNSUBSCRIBE") && items.contains("PRECEDENCE"));
}

#[test]
fn the_filter_reads_whether_you_wrote_to_them_and_whether_it_was_sent_to_many() {
    let mut known = crate::filter::Known::default();
    known.written.insert("ada@analytical.example".to_string());
    let tokens = |r: &index::Record| -> Vec<String> {
        crate::filter::Filter::tokens(r, &known)
            .into_iter()
            .map(|(t, _)| t)
            .collect()
    };
    let ada = with_headers("a", "Ada <ada@analytical.example>", "Notes", "");
    assert!(
        tokens(&ada).contains(&"wrote".to_string()),
        "{:?}",
        tokens(&ada)
    );
    let deal = with_headers(
        "d",
        "Shop <no-reply@shop.example>",
        "Sale",
        "List-Unsubscribe: <mailto:out@shop.example>\r\n",
    );
    let t = tokens(&deal);
    assert!(!t.contains(&"wrote".to_string()), "{t:?}");
    assert!(t.contains(&"noreply".to_string()), "{t:?}");
    assert!(t.contains(&"bulk".to_string()), "{t:?}");
    for address in [
        "noreply@x.org",
        "do-not-reply@x.org",
        "donotreply@x.org",
        "notifications@x.org",
    ] {
        let r = with_headers("n", address, "Hi", "");
        assert!(tokens(&r).contains(&"noreply".to_string()), "{address}");
    }
}

#[test]
fn whom_you_wrote_to_is_read_from_the_mail_you_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_with(dir.path(), &identity());
    let mut sent = with_headers("s", "me@example.com", "Re: Notes", "");
    sent.to = vec!["Ada@Analytical.example".to_string()];
    sent.cc = vec!["grace@navy.example".to_string()];
    sent.places[0].folder = "Sent".into();
    index
        .cache(vec![
            Cached::Folder {
                account: "a@b.c".into(),
                folder: "Sent".into(),
                state: index::record::FolderState {
                    role: Some(crate::mail::Role::Sent),
                    ..Default::default()
                },
            },
            Cached::Upsert(Box::new(sent)),
        ])
        .unwrap();
    let known = index.known();
    assert!(known.written.contains("ada@analytical.example"));
    assert!(known.written.contains("grace@navy.example"));
    // What is taught is read so.
    let ada = with_headers("a", "ada@analytical.example", "Notes", "");
    index
        .cache(vec![Cached::Upsert(Box::new(ada.clone()))])
        .unwrap();
    index.teach(&ada.key, false).unwrap();
    let taught = index.filter.taught.last().unwrap();
    assert!(taught.tokens.iter().any(|(t, _)| t == "wrote"));
}

#[test]
fn a_sender_never_seen_is_judged_by_whether_you_wrote_to_them_and_how_it_was_sent() {
    let mut known = crate::filter::Known::default();
    let mut f = crate::filter::Filter::default();
    for k in 0..30 {
        let colleague = format!("colleague{k}@uni.example");
        known.written.insert(colleague.clone());
        for (r, thrown) in [
            (
                with_headers(
                    &format!("s{k}"),
                    &format!("no-reply@shop{k}.example"),
                    &format!("Offer {k}"),
                    "List-Unsubscribe: <mailto:out@x>\r\n",
                ),
                true,
            ),
            (
                with_headers(&format!("c{k}"), &colleague, &format!("Draft {k}"), ""),
                false,
            ),
        ] {
            f.learn(crate::filter::Taught {
                key: r.key.clone(),
                thrown,
                tokens: crate::filter::Filter::tokens(&r, &known),
                message_id: r.message_id.clone(),
                seeded: false,
                ..Default::default()
            });
        }
    }
    // Each sender new, each subject new: only how it came tells them apart.
    let shop = with_headers(
        "x",
        "no-reply@never.example",
        "Something else entirely",
        "List-Unsubscribe: <mailto:out@x>\r\n",
    );
    known.written.insert("new@uni.example".into());
    let friend = with_headers("y", "new@uni.example", "Something else entirely", "");
    let odds = |r: &index::Record| f.odds(&crate::filter::Filter::tokens(r, &known));
    assert!(odds(&shop) > 0.8, "{}", odds(&shop));
    assert!(odds(&friend) < 0.2, "{}", odds(&friend));
}

#[test]
fn the_demos_robots_send_to_many_and_what_was_read_before_is_read_again() {
    let mut store = Demo::new();
    let shared = std::sync::Mutex::new(Index::memory());
    let stop = std::sync::atomic::AtomicBool::new(false);
    let now = jiff::Timestamp::now().as_second();
    index::sync::pass(&mut store, &shared, now, &stop, &mut |_| {}).unwrap();
    let ci: Vec<index::Record> = shared
        .lock()
        .unwrap()
        .records
        .values()
        .filter(|r| r.list == "ci.runner.example")
        .cloned()
        .collect();
    assert!(
        !ci.is_empty() && ci.iter().all(|r| r.bulk),
        "the CI's mail is bulk"
    );
    // Read before the index asked whether it was: read again, its places,
    // labels and when it was first seen kept.
    let mut stale = ci[0].clone();
    stale.bulk = false;
    shared
        .lock()
        .unwrap()
        .cache(vec![Cached::Upsert(Box::new(stale.clone()))])
        .unwrap();
    let n = index::sync::refresh(&mut store, &shared, &[stale.key.clone()], now).unwrap();
    assert_eq!(n, 1);
    let index = shared.lock().unwrap();
    let again = &index.records[&stale.key];
    assert!(again.bulk);
    assert_eq!(again.first_seen, ci[0].first_seen);
    assert_eq!(again.places, stale.places);
}

#[test]
fn filter_learn_reads_what_you_kept_again_so_it_is_read_as_new_mail_is() {
    let mut store = Demo::new();
    // The CI's mail, filed under Projects.
    let ci = store
        .page("INBOX", 0, 40)
        .unwrap()
        .summaries
        .into_iter()
        .find(|m| m.from == "CI")
        .unwrap();
    store.relocate("INBOX", ci.uid, "Projects", "").unwrap();
    let shared = std::sync::Mutex::new(Index::memory());
    let stop = std::sync::atomic::AtomicBool::new(false);
    let now = jiff::Timestamp::now().as_second();
    index::sync::pass(&mut store, &shared, now, &stop, &mut |_| {}).unwrap();
    let mut index = shared.into_inner().unwrap();
    // Each bulk message filed under a label, read as the index once read it.
    let stale: Vec<index::Record> = index
        .kept_seeds(1000)
        .into_iter()
        .filter_map(|(_, t)| index.records.get(&t.key).cloned())
        .filter(|r| r.bulk)
        .map(|mut r| {
            r.bulk = false;
            r
        })
        .collect();
    assert!(!stale.is_empty(), "the CI's mail, filed");
    index
        .cache(
            stale
                .iter()
                .cloned()
                .map(|r| Cached::Upsert(Box::new(r)))
                .collect(),
        )
        .unwrap();
    let mut stores: Vec<(String, Box<dyn Store>)> = vec![("Demo".into(), Box::new(store))];
    let (_, kept, learnt) = crate::learn(&mut index, &mut stores, 1000, now).unwrap();
    assert!(kept > 0 && learnt > 0);
    for r in &stale {
        let t = index
            .filter
            .taught
            .iter()
            .find(|t| t.key == r.key)
            .expect("learnt");
        assert!(t.tokens.iter().any(|(w, _)| w == "bulk"), "{}", r.subject);
    }
}

// ---- what was learnt long ago fades ----

pub(super) fn decided(r: &index::Record, thrown: bool) -> crate::filter::Taught {
    crate::filter::Taught {
        key: r.key.clone(),
        thrown,
        tokens: crate::filter::Filter::tokens(r, &Default::default()),
        message_id: r.message_id.clone(),
        ..Default::default()
    }
}

#[test]
fn no_weight_grows_without_end() {
    let deal = sent_by("x", "deals@shop.example", "Sale");
    let note = sent_by("y", "ada@analytical.example", "Engine");
    let weight = |decay: f32, n: usize| {
        let mut f = crate::filter::Filter::with_decay(decay);
        for _ in 0..n {
            f.learn(decided(&deal, true));
            f.learn(decided(&note, false));
        }
        f.weight("from:deals@shop.example")
    };
    // Unbounded, it goes on growing; bounded, it settles, in as many
    // decisions as a weight takes to fade.
    let d = crate::filter::DECAY;
    let n = (1.0 / (1.0 - d)) as usize;
    let (bounded, unbounded) = (
        weight(d, 4 * n) - weight(d, 2 * n),
        weight(1.0, 4 * n) - weight(1.0, 2 * n),
    );
    assert!(bounded.abs() < unbounded / 4.0, "{bounded} {unbounded}");
    assert!(weight(d, 4 * n) > 0.0);
}

#[test]
fn a_sender_not_heard_of_in_a_long_time_weighs_less() {
    let deal = sent_by("x", "deals@shop.example", "Sale");
    let note = sent_by("y", "ada@analytical.example", "Engine");
    let after = |decay: f32| {
        let mut f = crate::filter::Filter::with_decay(decay);
        for _ in 0..30 {
            f.learn(decided(&deal, true));
            f.learn(decided(&note, false));
        }
        let early = f.weight("from:deals@shop.example");
        // Many decisions since, none of them the shop's.
        let n = (1.0 / (1.0 - crate::filter::DECAY)) as usize;
        for k in 0..n {
            f.learn(decided(
                &sent_by(
                    &format!("t{k}"),
                    &format!("a{k}@x{k}.example"),
                    &format!("w{k}"),
                ),
                true,
            ));
            f.learn(decided(
                &sent_by(
                    &format!("k{k}"),
                    &format!("b{k}@y{k}.example"),
                    &format!("v{k}"),
                ),
                false,
            ));
        }
        f.weight("from:deals@shop.example") / early
    };
    assert!((after(1.0) - 1.0).abs() < 1e-6, "kept as it was, unfaded");
    assert!(
        after(crate::filter::DECAY) < 0.5,
        "{}",
        after(crate::filter::DECAY)
    );
    assert!(after(crate::filter::DECAY) > 0.0);
}
