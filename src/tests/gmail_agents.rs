//! An agent on Gmail, where the map reads All Mail and the agent is shown
//! the Inbox: the same message, found by its `Message-ID`.

use super::agents::{call, demo_server};
use super::drafting::answered;
use super::*;
use serde_json::json;

/// The server over a fake Gmail, its map read from it.
fn on_gmail(asks: bool) -> crate::mcp::Server {
    let (account, _) = serve_boxes(gmail_boxes());
    let mut server = demo_server(asks);
    server.use_store(Box::new(Imap::new(account)));
    server
}

#[test]
fn a_message_of_the_gmail_inbox_is_found_on_the_map() {
    let mut server = on_gmail(false);
    let r = call(&mut server, "would_throw_away", json!({ "uid": 2 }));
    assert_ne!(r["isError"], true, "{r}");
    assert!(r["structuredContent"]["odds"].is_number(), "{r}");
}

#[test]
fn new_mail_of_the_gmail_inbox_is_weighed() {
    let mut server = on_gmail(false);
    let r = call(&mut server, "triage", json!({ "hours": 24 * 3650 }));
    let messages = r["structuredContent"]["accounts"][0]["messages"]
        .as_array()
        .unwrap();
    assert!(!messages.is_empty(), "{r}");
    for m in messages {
        assert!(m.get("map").is_none(), "{m}");
        assert!(m["junk_odds"].is_number(), "{m}");
    }
}

#[test]
fn what_is_said_yes_to_of_the_gmail_inbox_is_learnt() {
    let mut server = on_gmail(true);
    let (_, r) = answered(
        &mut server,
        1,
        "trash_messages",
        json!({ "uids": [2] }),
        "accept",
    );
    assert_eq!(r["structuredContent"]["moved"], 1, "{r}");
    assert_eq!(r["structuredContent"]["learnt"], 1, "{r}");
}

#[test]
fn a_draft_said_yes_to_throwing_away_teaches_nothing_even_where_all_mail_holds_it() {
    let (account, boxes) = serve_boxes(gmail_boxes());
    let mut server = demo_server(true);
    server.use_store(Box::new(Imap::new(account)));
    // Gmail keeps a draft in All Mail as well, so the map can know it.
    let drafts = "[Gmail]/Drafts";
    let uid = 900;
    {
        let mut held = boxes.lock().unwrap();
        let raw = held
            .iter()
            .find(|b| b.name == "INBOX")
            .unwrap()
            .letters
            .iter()
            .find(|l| l.uid == 2)
            .unwrap()
            .raw
            .clone();
        let at = held.iter().position(|b| b.name == drafts).unwrap();
        held[at].push(Letter::new(uid, &["\\Seen", "\\Draft"], raw));
    }
    let (_, r) = answered(
        &mut server,
        1,
        "trash_messages",
        json!({ "folder": drafts, "uids": [uid] }),
        "accept",
    );
    assert_eq!(r["structuredContent"]["moved"], 1, "{r}");
    assert_eq!(r["structuredContent"]["learnt"], 0, "{r}");
    assert!(server.learnt.is_empty());
}

#[test]
fn a_draft_asked_about_teaches_nothing_even_where_all_mail_holds_it() {
    let (account, boxes) = serve_boxes(gmail_boxes());
    let mut server = demo_server(true);
    server.use_store(Box::new(Imap::new(account)));
    {
        let mut held = boxes.lock().unwrap();
        let raw = held.iter().find(|b| b.name == "INBOX").unwrap().letters[0]
            .raw
            .clone();
        let at = held
            .iter()
            .position(|b| b.name == "[Gmail]/Drafts")
            .unwrap();
        held[at].push(Letter::new(900, &["\\Seen", "\\Draft"], raw));
    }
    let script = vec![
        json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "ask_about_messages",
                        "arguments": { "folder": "[Gmail]/Drafts", "uids": [900] } } })
        .to_string(),
        json!({ "jsonrpc": "2.0", "id": "letterbox-1",
            "result": { "action": "accept", "content": { "answer": "junk" } } })
        .to_string(),
    ];
    let out = super::agents::Written::default();
    server
        .serve(crate::mcp::Io {
            lines: Box::new(script.into_iter().map(Ok)),
            out: Box::new(out.clone()),
        })
        .unwrap();
    let written = out.messages();
    let reply = written.iter().find(|m| m["id"] == 10).unwrap();
    assert_eq!(reply["result"]["structuredContent"]["junk"], 1, "{reply}");
    assert_eq!(reply["result"]["structuredContent"]["learnt"], 0, "{reply}");
    assert!(server.learnt.is_empty());
}
