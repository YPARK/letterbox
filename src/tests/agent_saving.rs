//! A message's attachments saved by an agent: all of them, or one by its
//! name, each under a name not yet taken, nothing written over.

use super::agents::{call, demo_server};
use serde_json::json;

/// The uid of the demo's letter from `from`.
fn uid_from(server: &mut crate::mcp::Server, from: &str) -> u64 {
    let listed = call(server, "list_messages", json!({ "limit": 50 }));
    listed["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["from"] == from)
        .unwrap_or_else(|| panic!("no letter from {from}: {listed}"))["uid"]
        .as_u64()
        .unwrap()
}

#[test]
fn an_agent_saves_a_messages_attachments_all_or_one() {
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().display().to_string();
    let mut server = demo_server(false);
    let uid = uid_from(&mut server, "Alan Turing");

    let all = call(
        &mut server,
        "save_attachment",
        json!({ "uid": uid, "to": to }),
    );
    let saved = all["structuredContent"]["saved"].as_array().unwrap();
    assert_eq!(saved.len(), 2, "{all}");
    let states = std::fs::read_to_string(dir.path().join("states.txt")).unwrap();
    assert!(states.contains("P0, R"), "as sent: {states}");
    assert!(dir.path().join("computable-numbers.pdf").is_file());

    // One, by its name, again: kept beside the first, not over it.
    let one = call(
        &mut server,
        "save_attachment",
        json!({ "uid": uid, "to": to, "name": "states.txt" }),
    );
    assert_eq!(
        one["structuredContent"]["saved"],
        json!([dir.path().join("states (2).txt").display().to_string()]),
        "{one}"
    );
}

#[test]
fn a_name_it_has_not_says_what_it_has() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = demo_server(false);
    let uid = uid_from(&mut server, "Alan Turing");
    let r = call(
        &mut server,
        "save_attachment",
        json!({ "uid": uid, "to": dir.path(), "name": "nothing.doc" }),
    );
    assert_eq!(r["isError"], true, "{r}");
    let said = r.to_string();
    assert!(said.contains("no attachment named nothing.doc"), "{said}");
    assert!(
        said.contains("states.txt") && said.contains("read_message"),
        "{said}"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_message_without_attachments_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = demo_server(false);
    let uid = uid_from(&mut server, "Ada Lovelace");
    let r = call(
        &mut server,
        "save_attachment",
        json!({ "uid": uid, "to": dir.path() }),
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(r.to_string().contains("no attachments"), "{r}");
}

#[test]
fn two_attachments_of_one_name_are_both_kept() {
    let raw = "From: a@example.com\r\nTo: b@example.com\r\nSubject: two\r\n\
        Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\n\r\nTwo of one name.\r\n\
        --b\r\nContent-Type: text/plain; name=\"notes.txt\"\r\n\
        Content-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nfirst\r\n\
        --b\r\nContent-Type: text/plain; name=\"notes.txt\"\r\n\
        Content-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nsecond\r\n\
        --b--\r\n";
    let dir = tempfile::tempdir().unwrap();
    let saved =
        crate::app::save_attachments(raw.as_bytes(), Some("notes.txt"), dir.path()).unwrap();
    assert_eq!(
        saved,
        vec![
            dir.path().join("notes.txt"),
            dir.path().join("notes (2).txt")
        ]
    );
    let read = |p: &std::path::Path| std::fs::read_to_string(p).unwrap();
    assert_eq!(read(&saved[0]).trim(), "first");
    assert_eq!(read(&saved[1]).trim(), "second");
}

#[test]
fn an_agent_letterbox_started_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = demo_server(false);
    server.background = true;
    let uid = uid_from(&mut server, "Alan Turing");
    let r = call(
        &mut server,
        "save_attachment",
        json!({ "uid": uid, "to": dir.path() }),
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(
        r.to_string().contains("letterbox started this agent"),
        "{r}"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
