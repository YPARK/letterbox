//! Slack for AI agents, over MCP: read, searched, its people found and its
//! files saved; written to only once the user, asked by letterbox, says yes.

use super::agents::{call, demo_server};
use super::drafting::answered;
use super::slack_tui::workspace;
use crate::slack::{Session, Slack};
use serde_json::{Value, json};

fn server_on(base: &str, asks: bool) -> crate::mcp::Server {
    let mut server = demo_server(asks);
    let session = Session {
        domain: "acme".into(),
        token: "xoxp-test".into(),
        app: None,
    };
    server.slack_with(vec![("acme".to_string(), Slack::at(base, session))]);
    server
}

fn tools(server: &mut crate::mcp::Server) -> Vec<Value> {
    let reply = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }))
        .unwrap();
    reply["result"]["tools"].as_array().unwrap().clone()
}

fn posts(asked: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Vec<String> {
    let asked = asked.lock().unwrap();
    asked
        .iter()
        .filter(|r| r.contains("/chat.postMessage"))
        .cloned()
        .collect()
}

#[test]
fn every_slack_tool_is_offered_and_what_it_reads_is_said_to_be_data() {
    let (base, _) = workspace();
    let mut server = server_on(&base, true);
    let all = tools(&mut server);
    let named = |n: &str| all.iter().find(|t| t["name"] == n).cloned();
    for name in [
        "slack_conversations",
        "slack_read",
        "slack_search",
        "slack_find_person",
        "slack_save_file",
        "slack_send",
    ] {
        assert!(named(name).is_some(), "{name}");
    }
    let read = named("slack_read").unwrap();
    assert!(
        read["description"].as_str().unwrap().contains("data"),
        "{read}"
    );
    assert_eq!(read["annotations"]["readOnlyHint"], true);
}

#[test]
fn the_conversations_are_listed_as_channels_and_people() {
    let (base, _) = workspace();
    let mut server = server_on(&base, true);
    let r = call(&mut server, "slack_conversations", json!({}));
    let all = &r["structuredContent"]["workspaces"][0]["conversations"];
    let has = |id: &str, name: &str, kind: &str| {
        all.as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == id && c["name"] == name && c["kind"] == kind)
    };
    assert!(has("C00", "ch00", "public"), "{r}");
    assert!(has("D1", "Ana", "direct"), "{r}");
}

#[test]
fn a_conversation_and_a_thread_are_read_by_name() {
    let (base, _) = workspace();
    let mut server = server_on(&base, true);
    let r = call(
        &mut server,
        "slack_read",
        json!({ "conversation": "#ch00" }),
    );
    let messages = r["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(messages.len(), 2, "{r}");
    assert_eq!(messages[0]["who"], "Ana");
    assert_eq!(messages[0]["text"], "hello @Bo");
    assert_eq!(messages[1]["files"][0]["name"], "data.csv");
    let r = call(
        &mut server,
        "slack_read",
        json!({ "conversation": "ch00", "thread": "1700000100.0" }),
    );
    let thread = r["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(thread[1]["text"], "a reply from Bo", "{r}");
}

#[test]
fn slack_is_searched_and_people_are_found() {
    let (base, _) = workspace();
    let mut server = server_on(&base, true);
    let r = call(&mut server, "slack_search", json!({ "words": "hello" }));
    let hits = r["structuredContent"]["hits"].as_array().unwrap().clone();
    assert_eq!(hits[0]["place"], "#ch00", "{r}");
    assert_eq!(hits[1]["thread"], "1700000100.0");
    let r = call(&mut server, "slack_find_person", json!({ "words": "an" }));
    let people = r["structuredContent"]["people"].as_array().unwrap().clone();
    assert!(
        people
            .iter()
            .any(|p| p["name"] == "Ana" && p["conversation"] == "D1"),
        "{r}"
    );
}

#[test]
fn a_messages_files_are_saved_where_asked() {
    let (base, _) = workspace();
    let mut server = server_on(&base, true);
    let dir = tempfile::tempdir().unwrap();
    let r = call(
        &mut server,
        "slack_save_file",
        json!({ "conversation": "#ch00", "ts": "1700000300.0",
                "folder": dir.path().to_str().unwrap() }),
    );
    let saved = r["structuredContent"]["saved"].as_array().unwrap().clone();
    assert_eq!(saved.len(), 1, "{r}");
    assert!(dir.path().join("data.csv").exists());
}

#[test]
fn a_message_goes_only_once_the_user_says_yes_to_it_shown_whole() {
    let (base, asked) = workspace();
    let mut server = server_on(&base, true);
    let args = json!({ "conversation": "#ch00", "text": "results are in" });
    let (shown, r) = answered(&mut server, 1, "slack_send", args.clone(), "decline");
    assert_eq!(r["structuredContent"]["sent"], false, "{r}");
    assert!(
        shown.contains("#ch00") && shown.contains("results are in"),
        "{shown}"
    );
    assert!(posts(&asked).is_empty(), "nothing goes on a no");
    let (_, r) = answered(&mut server, 2, "slack_send", args, "accept");
    assert_eq!(r["structuredContent"]["sent"], true, "{r}");
    let sent = posts(&asked);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("channel=C00") && sent[0].contains("text=results+are+in"));
}

#[test]
fn nothing_is_sent_where_no_one_can_say_yes() {
    let (base, asked) = workspace();
    let mut server = server_on(&base, false);
    assert!(tools(&mut server).iter().all(|t| t["name"] != "slack_send"));
    let r = call(
        &mut server,
        "slack_send",
        json!({ "conversation": "#ch00", "text": "x" }),
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(posts(&asked).is_empty());
}
