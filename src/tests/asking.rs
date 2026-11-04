//! What the junk filter is least sure of, or got wrong, asked of you: by L
//! in letterbox, or by an agent, through letterbox. Each answer is learnt,
//! and nothing is moved.

use super::*;
use serde_json::json;

const NOW: i64 = 2_000;

/// An index that has learnt the shop's mail is thrown away, and a
/// colleague's kept, and holds: more of each; two from someone new; and a
/// message of the shop's you kept all the same.
fn weighed() -> (Index, index::Key) {
    let mut index = Index::memory();
    index.filter = taught(30);
    let kept = sent_by("kept", "deals@shop.example", "Save now");
    let mut records = vec![
        sent_by("shop", "deals@shop.example", "Save more"),
        sent_by("ada", "ada@analytical.example", "Engine notes"),
        sent_by("new1", "someone@elsewhere.example", "Hello there"),
        sent_by("new2", "someone@elsewhere.example", "Hello again"),
        kept.clone(),
    ];
    for r in &mut records {
        r.first_seen = NOW - 10;
        r.date = None;
    }
    index
        .cache(
            records
                .into_iter()
                .map(|r| Cached::Upsert(Box::new(r)))
                .collect(),
        )
        .unwrap();
    index.teach(&kept.key, false).unwrap();
    (index, kept.key)
}

#[test]
fn what_it_is_least_sure_of_or_got_wrong_is_asked_one_a_sender() {
    let (index, kept) = weighed();
    let edges = index.edges(5, NOW, &index.known());
    let ids: Vec<&str> = edges.iter().map(|e| e.key.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    // What it now disagrees with you on comes first, saying what you did.
    assert_eq!(edges[0].key, kept);
    assert_eq!(edges[0].decided, Some(false));
    assert!(edges[0].odds > 0.5, "{}", edges[0].odds);
    // Then one of the new sender's two; never what it is sure of.
    assert!(ids[1].starts_with("new"), "{ids:?}");
    assert_eq!(edges[1].decided, None);
    assert!((edges[1].odds - 0.5).abs() < 0.3, "{}", edges[1].odds);
    assert!(index.edges(1, NOW, &index.known()).len() == 1);
    // Nothing so old is asked.
    assert!(
        index
            .edges(5, NOW + 365 * 24 * 60 * 60, &index.known())
            .iter()
            .all(|e| e.decided.is_some())
    );
}

#[test]
fn what_you_answered_is_learnt_never_asked_again_and_forgotten_with_what_an_agent_taught() {
    let (mut index, kept) = weighed();
    let before = index.filter.kept;
    assert_eq!(index.days_since_asked(NOW), None);
    let known = index.known();
    index.teach_asked(&kept, false, &known).unwrap();
    assert_eq!(index.filter.kept, before + 1);
    assert!(
        index
            .edges(5, NOW, &index.known())
            .iter()
            .all(|e| e.key != kept)
    );
    // Nor when what is learnt after turns against your answer again.
    let shop = index
        .records
        .values()
        .find(|r| r.key.id == "shop")
        .unwrap()
        .key
        .clone();
    for _ in 0..40 {
        index.teach(&shop, true).unwrap();
    }
    let tokens = index
        .filter
        .taught
        .iter()
        .rev()
        .find(|t| t.key == kept)
        .unwrap()
        .tokens
        .clone();
    assert!(index.filter.odds(&tokens) > 0.7, "against it again");
    assert!(
        index
            .edges(5, NOW, &index.known())
            .iter()
            .all(|e| e.key != kept)
    );
    assert_eq!(
        index.days_since_asked(jiff::Timestamp::now().as_second()),
        Some(0)
    );
    let (thrown, _) = (index.filter.thrown, index.filter.kept);
    assert_eq!(index.unadvise().unwrap(), 1);
    assert_eq!((index.filter.thrown, index.filter.kept), (thrown, before));
}

#[test]
fn l_asks_in_letterbox_what_it_is_least_sure_of_and_moves_nothing() {
    let mut app = demo_map();
    let inbox = app.shown.len();
    let before = {
        let i = app.index.lock().unwrap();
        (i.filter.thrown, i.filter.kept)
    };
    press(&mut app, &["L"]);
    let app::Dialog::Asking(asking) = &app.dialog else {
        panic!("not asked: {}", app.status);
    };
    let n = asking.edges.len();
    assert!(n >= 3, "{n}");
    let first = asking.edges[0].subject.clone();
    let screen = app::screen(&mut app, 140, 40);
    assert!(screen.contains(first.as_str()), "{screen}");
    // x, it is junk; n, it is kept; space passes over one.
    press(&mut app, &["x", "n", "<space>"]);
    let after = {
        let i = app.index.lock().unwrap();
        (i.filter.thrown, i.filter.kept)
    };
    assert_eq!(after, (before.0 + 1, before.1 + 1));
    assert_eq!(app.shown.len(), inbox, "nothing moved");
    // Any other key leaves the rest, as they are.
    press(&mut app, &["<esc>"]);
    assert!(!app.dialog.is_open());
    assert!(app.status.contains("2 learnt"), "{}", app.status);
    // Those answered are not asked again, nor shown first.
    press(&mut app, &["L"]);
    if let app::Dialog::Asking(asking) = &app.dialog {
        assert!(asking.edges.iter().all(|e| e.subject != first));
    }
}

#[test]
fn an_agent_is_given_what_to_ask_and_the_user_answers_to_letterbox() {
    let mut server = demo_server(true);
    let r = call(&mut server, "uncertain_messages", json!({ "limit": 3 }));
    let found = &r["structuredContent"];
    assert_eq!(found["days_since_asked"], serde_json::Value::Null);
    assert_eq!(found["due"], true);
    let messages = found["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "{found}");
    let uids: Vec<u64> = messages
        .iter()
        .map(|m| m["uid"].as_u64().unwrap())
        .collect();
    assert!(messages[0]["from"].is_string() && messages[0]["odds"].is_number());

    let tool = json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call",
        "params": { "name": "ask_about_messages", "arguments": { "uids": uids } } })
    .to_string();
    let said = |id: &str, answer: &str| {
        json!({ "jsonrpc": "2.0", "id": id,
            "result": { "action": "accept", "content": { "answer": answer } } })
        .to_string()
    };
    let script = vec![
        tool,
        said("letterbox-1", "junk"),
        said("letterbox-2", "keep"),
        json!({ "jsonrpc": "2.0", "id": "letterbox-3", "result": { "action": "decline" } })
            .to_string(),
    ];
    let before = call(&mut server, "list_messages", json!({ "limit": 200 }));
    let out = Written::default();
    server
        .serve(crate::mcp::Io {
            lines: Box::new(script.into_iter().map(Ok)),
            out: Box::new(out.clone()),
        })
        .unwrap();
    let written = out.messages();
    let asked: Vec<&serde_json::Value> = written
        .iter()
        .filter(|m| m["method"] == "elicitation/create")
        .collect();
    assert_eq!(asked.len(), 3, "{written:?}");
    let shown = asked[0]["params"]["message"].as_str().unwrap();
    assert!(
        shown.contains(messages[0]["from"].as_str().unwrap()),
        "{shown}"
    );
    let reply = written.iter().find(|m| m["id"] == 10).unwrap();
    let result = &reply["result"]["structuredContent"];
    assert_eq!(
        (result["junk"].as_u64(), result["keep"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(result["learnt"], 2);
    // Each learnt as asked and through an agent; nothing moved.
    let learnt: Vec<(bool, bool, bool)> = server
        .learnt
        .iter()
        .map(|t| (t.thrown, t.agent, t.asked))
        .collect();
    assert_eq!(learnt, [(true, true, true), (false, true, true)]);
    let after = call(&mut server, "list_messages", json!({ "limit": 200 }));
    assert_eq!(
        before["structuredContent"]["total"],
        after["structuredContent"]["total"]
    );
}

#[test]
fn an_agent_that_cannot_ask_is_not_offered_to() {
    for mut server in [demo_server(false), {
        let mut s = demo_server(true);
        s.background = true;
        s
    }] {
        let r = call(&mut server, "ask_about_messages", json!({ "uids": [1] }));
        assert_eq!(r["isError"], true, "{r}");
        assert!(server.learnt.is_empty());
    }
}

#[test]
fn old_mail_is_not_new_though_it_was_first_seen_lately_and_a_date_to_come_gives_it_no_more_time() {
    let (mut index, _) = weighed();
    let new1 = |index: &Index| {
        index
            .edges(5, NOW, &index.known())
            .iter()
            .filter(|e| e.key.id.starts_with("new"))
            .count()
    };
    assert_eq!(new1(&index), 1);
    for r in index.records.values_mut() {
        r.date = Some(NOW - 400 * 24 * 60 * 60);
    }
    assert_eq!(
        new1(&index),
        0,
        "dated long ago, first seen as the map began"
    );
    for r in index.records.values_mut() {
        r.date = Some(NOW + 400 * 24 * 60 * 60);
        r.first_seen = NOW - 400 * 24 * 60 * 60;
    }
    assert_eq!(new1(&index), 0, "dated to come, seen long ago");
}

#[test]
fn a_message_no_longer_anywhere_is_not_asked_of() {
    let (mut index, kept) = weighed();
    index.records.get_mut(&kept).unwrap().places.clear();
    assert!(
        index
            .edges(5, NOW, &index.known())
            .iter()
            .all(|e| e.key != kept)
    );
}

#[test]
fn a_message_gone_as_it_is_answered_is_not_counted_learnt() {
    let mut app = demo_map();
    press(&mut app, &["L"]);
    let app::Dialog::Asking(asking) = &app.dialog else {
        panic!("not asked");
    };
    let key = asking.edges[0].key.clone();
    let n = asking.edges.len();
    app.index.lock().unwrap().records.remove(&key);
    press(&mut app, &["x"]);
    let app::Dialog::Asking(asking) = &app.dialog else {
        panic!("still asked of the rest");
    };
    assert_eq!((asking.edges.len(), asking.learnt), (n - 1, 0));
}

#[test]
fn what_was_answered_is_kept_though_the_program_closes_and_is_not_asked_again() {
    let mut server = demo_server(true);
    let r = call(&mut server, "uncertain_messages", json!({ "limit": 3 }));
    let uids: Vec<u64> = r["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["uid"].as_u64().unwrap())
        .collect();
    let script = vec![
        json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "ask_about_messages", "arguments": { "uids": uids } } })
        .to_string(),
        json!({ "jsonrpc": "2.0", "id": "letterbox-1",
            "result": { "action": "accept", "content": { "answer": "junk" } } })
        .to_string(),
    ];
    let out = Written::default();
    server
        .serve(crate::mcp::Io {
            lines: Box::new(script.into_iter().map(Ok)),
            out: Box::new(out.clone()),
        })
        .unwrap();
    assert_eq!(server.learnt.len(), 1, "the one answered");
    let reply = out.messages();
    let reply = reply.iter().find(|m| m["id"] == 10).unwrap();
    assert_eq!(reply["result"]["structuredContent"]["junk"], 1, "{reply}");
    // Seen at once, before letterbox has taken it in.
    let again = call(&mut server, "uncertain_messages", json!({ "limit": 3 }));
    assert_eq!(again["structuredContent"]["days_since_asked"], 0, "{again}");
    assert_eq!(
        again["structuredContent"]["due"], false,
        "not again so soon"
    );
    let asked_again = again["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["uid"].as_u64() == Some(uids[0]));
    assert!(!asked_again, "{again}");
}

#[test]
fn a_letterbox_that_only_reads_sees_what_was_answered_and_leaves_it_to_be_learnt() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let note = sent_by("n", "ada@analytical.example", "Engine notes");
    let answered = crate::filter::Taught {
        key: note.key.clone(),
        asked: true,
        agent: true,
        at: 5,
        tokens: crate::filter::Filter::tokens(&note, &Default::default()),
        ..Default::default()
    };
    let hold = Index::open_with(dir.path(), &id);
    index::advise(dir.path(), &id.to_public(), &[answered]).unwrap();
    let reader = Index::read_only(dir.path(), &id);
    assert_eq!(reader.days_since_asked(5), Some(0), "seen");
    drop(reader);
    drop(hold);
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(
        index.filter.taught.iter().filter(|t| t.asked).count(),
        1,
        "learnt, once"
    );
}
