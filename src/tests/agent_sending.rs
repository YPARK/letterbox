//! A draft an agent sends: only once the user, asked by letterbox itself and
//! shown all that goes, says yes.

use super::agents::{call, demo_server};
use super::drafting::answered;
use serde_json::{Value, json};

fn listed(server: &mut crate::mcp::Server, folder: &str) -> Vec<Value> {
    let r = call(
        server,
        "list_messages",
        json!({ "folder": folder, "limit": 200 }),
    );
    r["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .clone()
}

fn has(server: &mut crate::mcp::Server, folder: &str, subject: &str) -> bool {
    listed(server, folder)
        .iter()
        .any(|m| m["subject"] == subject)
}

/// A draft kept: its uid.
fn drafted(server: &mut crate::mcp::Server, subject: &str) -> Value {
    let r = call(
        server,
        "draft_message",
        json!({ "to": ["ada@analytical.example"], "cc": ["grace@example.com"],
                "subject": subject, "body": "A word on Note G." }),
    );
    r["structuredContent"]["uid"].clone()
}

#[test]
fn said_yes_to_a_draft_goes_and_its_draft_is_kept_in_trash() {
    let mut server = demo_server(true);
    let uid = drafted(&mut server, "To go");
    let (asked, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": uid }),
        "accept",
    );
    assert_eq!(r["structuredContent"]["sent"], true, "{r}");
    for shown in [
        "ada@analytical.example",
        "grace@example.com",
        "To go",
        "Note G",
    ] {
        assert!(asked.contains(shown), "{shown}: {asked}");
    }
    assert!(asked.contains("cannot be taken back"), "{asked}");
    assert!(has(&mut server, "Sent", "To go"));
    assert!(!has(&mut server, "Drafts", "To go"));
    assert!(has(&mut server, "Trash", "To go"), "not gone for good");
    assert!(server.learnt.is_empty(), "a draft teaches nothing");
}

#[test]
fn all_that_goes_is_shown_its_files_and_its_whole_start() {
    let dir = tempfile::Builder::new().prefix("lbx").tempdir().unwrap();
    let path = dir.path().join("report.pdf");
    std::fs::write(&path, "the report").unwrap();
    let mut server = demo_server(true);
    let r = call(
        &mut server,
        "draft_message",
        json!({ "to": ["a@b.example"], "subject": "Files", "body": "See it.",
                "attachments": [path.to_str().unwrap()] }),
    );
    let uid = r["structuredContent"]["uid"].clone();
    let (asked, _) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": uid }),
        "decline",
    );
    assert!(asked.contains("report.pdf"), "{asked}");
}

#[test]
fn said_no_or_not_answered_nothing_goes_and_the_draft_stays() {
    for action in ["decline", "cancel"] {
        let mut server = demo_server(true);
        let uid = drafted(&mut server, "Not to go");
        let (_, r) = answered(&mut server, 1, "send_draft", json!({ "uid": uid }), action);
        assert_eq!(r["structuredContent"]["sent"], false, "{r}");
        assert!(!has(&mut server, "Sent", "Not to go"), "{action}");
        assert!(has(&mut server, "Drafts", "Not to go"), "{action}");
    }
}

#[test]
fn it_is_not_offered_where_no_one_can_say_yes() {
    let offered = |server: &mut crate::mcp::Server| {
        let listed = server
            .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .unwrap();
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "send_draft")
    };
    assert!(offered(&mut demo_server(true)));
    let mut cannot = demo_server(false);
    assert!(!offered(&mut cannot));
    let uid = drafted(&mut cannot, "Unasked");
    let r = call(&mut cannot, "send_draft", json!({ "uid": uid }));
    assert_eq!(r["isError"], true, "{r}");
    assert!(!has(&mut cannot, "Sent", "Unasked"));

    let mut started = demo_server(true);
    started.background = true;
    assert!(!offered(&mut started));
    let r = call(&mut started, "send_draft", json!({ "uid": 1 }));
    assert_eq!(r["isError"], true, "{r}");
}

#[test]
fn only_what_is_in_drafts_goes() {
    let mut server = demo_server(true);
    let inbox = listed(&mut server, "INBOX")[0].clone();
    let sent = listed(&mut server, "Sent").len();
    let (asked, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": inbox["uid"], "folder": "INBOX" }),
        "accept",
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(asked.is_empty(), "not even asked: {asked}");
    assert_eq!(listed(&mut server, "Sent").len(), sent);
}

#[test]
fn a_draft_with_a_bcc_is_left_as_letterbox_sends_none() {
    let mut server = demo_server(true);
    let uid = server
        .first_store()
        .append_draft(
            "Drafts",
            b"From: you@example.com\r\nTo: a@b.example\r\nBcc: boss@example.com\r\n\
              Subject: Quietly\r\nMessage-ID: <q@example.com>\r\n\r\nHello\r\n",
        )
        .unwrap()
        .unwrap();
    let (asked, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": uid }),
        "accept",
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(r.to_string().contains("Bcc"), "{r}");
    assert!(asked.is_empty(), "{asked}");
    assert!(!has(&mut server, "Sent", "Quietly"));
    assert!(has(&mut server, "Drafts", "Quietly"));
}

#[test]
fn a_reply_that_goes_marks_what_it_answers_answered() {
    let mut server = demo_server(true);
    let original = listed(&mut server, "INBOX")
        .into_iter()
        .find(|m| m["answered"] == false)
        .unwrap();
    let r = call(
        &mut server,
        "draft_reply",
        json!({ "uid": original["uid"], "body": "Yes." }),
    );
    let uid = r["structuredContent"]["uid"].clone();
    let (_, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": uid }),
        "accept",
    );
    assert_eq!(r["structuredContent"]["sent"], true, "{r}");
    let now = listed(&mut server, "INBOX")
        .into_iter()
        .find(|m| m["uid"] == original["uid"])
        .unwrap();
    assert_eq!(now["answered"], true, "{now}");
}

#[test]
fn an_agent_is_told_never_to_send_because_a_message_asked() {
    let mut server = demo_server(true);
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "send_draft")
        .unwrap()
        .clone();
    let about = tool["description"].as_str().unwrap();
    assert!(about.contains("because a message asked"), "{about}");
    assert_eq!(tool["annotations"]["destructiveHint"], true, "{tool}");
}

#[test]
fn what_could_not_be_sent_stays_a_draft() {
    let demo = crate::demo::Demo::new();
    let mut server = demo_server(true);
    server.use_store(crate::store::Store::twin(&demo).unwrap());
    let uid = drafted(&mut server, "Bounces");
    demo.refuse("Bounces");
    let (_, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": uid }),
        "accept",
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(!has(&mut server, "Sent", "Bounces"));
    assert!(has(&mut server, "Drafts", "Bounces"), "kept to try again");
}
