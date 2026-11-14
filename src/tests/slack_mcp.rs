//! Slack for AI agents, over MCP: read, searched, its people found and its
//! files saved; written to only once the user, asked by letterbox, says yes.

use super::agents::{call, demo_server};
use super::drafting::answered;
use super::slack_tui::{slack, workspace};
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

/// What was asked of Slack's `method`.
fn asked_for(asked: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, method: &str) -> Vec<String> {
    let asked = asked.lock().unwrap();
    asked
        .iter()
        .filter(|r| r.contains(method))
        .cloned()
        .collect()
}

fn posts(asked: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Vec<String> {
    asked_for(asked, "/chat.postMessage")
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
        "slack_recent",
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

fn uploads(asked: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Vec<String> {
    let asked = asked.lock().unwrap();
    asked
        .iter()
        .filter(|r| r.contains("/files.completeUploadExternal"))
        .cloned()
        .collect()
}

#[test]
fn files_go_only_once_the_user_says_yes_to_each_shown() {
    let (base, asked) = workspace();
    let mut server = server_on(&base, true);
    let dir = tempfile::Builder::new().prefix("lbx").tempdir().unwrap();
    let file = dir.path().join("figure.pdf");
    std::fs::write(&file, "%PDF").unwrap();
    let args = json!({ "conversation": "#ch00", "files": [file.display().to_string()] });
    let (shown, r) = answered(&mut server, 1, "slack_send", args.clone(), "decline");
    assert_eq!(r["structuredContent"]["sent"], false, "{r}");
    assert!(
        shown.contains("figure.pdf") && shown.contains(&file.display().to_string()),
        "the whole path is shown: {shown}"
    );
    assert!(uploads(&asked).is_empty(), "nothing goes on a no");
    let (_, r) = answered(&mut server, 2, "slack_send", args, "accept");
    assert_eq!(r["structuredContent"]["sent"], true, "{r}");
    let sent = uploads(&asked);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("channel_id=C00"), "{}", sent[0]);
    assert!(posts(&asked).is_empty(), "files go as files");
}

#[test]
fn a_hidden_file_or_one_not_named_whole_is_not_sent() {
    let (base, asked) = workspace();
    let mut server = server_on(&base, true);
    let dir = tempfile::Builder::new().prefix("lbx").tempdir().unwrap();
    let hidden = dir.path().join(".env");
    std::fs::write(&hidden, "SECRET=1").unwrap();
    for path in [hidden.display().to_string(), "notes.txt".to_string()] {
        let r = call(
            &mut server,
            "slack_send",
            json!({ "conversation": "#ch00", "text": "x", "files": [path] }),
        );
        assert_eq!(r["isError"], true, "{path}: {r}");
    }
    assert!(uploads(&asked).is_empty() && posts(&asked).is_empty());
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

/// A workspace where the user is Bo, in #lab, a direct message with Ana and
/// a group of three; where Slack's search gives each page of `pages` as its
/// number asks, of `total` pages.
fn searched(
    pages: Vec<Value>,
    total: u32,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    slack(move |_| {
        let mut answers = vec![
            (
                "auth.test",
                json!({"ok": true, "user": "bo", "user_id": "U2", "team": "Acme",
                       "url": "https://acme.slack.com/"}),
            ),
            (
                "users.list",
                json!({"ok": true, "members": [
                    {"id": "U1", "name": "ana", "profile": {"display_name": "Ana"}},
                    {"id": "U2", "name": "bo", "profile": {"display_name": "Bo"}},
                ]}),
            ),
            (
                "users.conversations",
                json!({"ok": true, "channels": [
                    {"id": "C00", "name": "lab", "is_channel": true},
                    {"id": "D1", "is_im": true, "user": "U1"},
                    {"id": "G1", "name": "mpdm-ana--bo--cy-1", "is_mpim": true},
                ]}),
            ),
        ];
        let keys = [
            "search.messages page=1",
            "search.messages page=2",
            "search.messages page=3",
        ];
        for (key, matches) in keys.into_iter().zip(pages) {
            answers.push((
                key,
                json!({"ok": true, "messages": {"matches": matches,
                       "paging": {"pages": total}}}),
            ));
        }
        answers
    })
}

/// A message Slack's search finds, `ago` seconds ago.
fn hit(channel: Value, ago: i64, text: &str, link: &str) -> Value {
    let ts = format!("{}.000100", jiff::Timestamp::now().as_second() - ago);
    json!({"channel": channel, "ts": ts, "user": "U1", "text": text,
           "permalink": format!("https://acme.slack.com/archives/{link}")})
}

fn searches(asked: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Vec<String> {
    asked_for(asked, "/search.messages")
}

#[test]
fn what_came_to_slack_lately_is_gathered_beside_the_mail() {
    let lab = json!({"id": "C00", "name": "lab"});
    let (base, asked) = searched(
        vec![json!([
            hit(
                lab.clone(),
                1_800,
                "<@U2> approve this?",
                "C00/p3?thread_ts=1600000000.0"
            ),
            hit(
                json!({"id": "G1", "name": "mpdm-ana--bo--cy-1"}),
                2_000,
                "lunch?",
                "G1/p9"
            ),
            hit(
                json!({"id": "C99", "name": "elsewhere"}),
                2_500,
                "not the user's",
                "C99/p1"
            ),
            hit(
                json!({"id": "D1", "name": "U1", "is_im": true}),
                3_600,
                "can you look at the draft?",
                "D1/p2"
            ),
            hit(lab.clone(), 7_200, "<@U2> the run finished", "C00/p1"),
            hit(lab, 36_000, "from long before", "C00/p0"),
        ])],
        1,
    );
    let mut server = server_on(&base, true);
    let r = call(&mut server, "slack_recent", json!({ "hours": 6 }));
    let space = &r["structuredContent"]["workspaces"][0];
    assert_eq!(space["name"], "acme", "{r}");
    assert_eq!(space["you"], "Bo", "{r}");
    // The user's conversations alone, by the one busy last first, each with
    // its kind; each one's oldest first; what came before the hours left out.
    let found = space["conversations"].as_array().unwrap().clone();
    let names: Vec<&str> = found.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["lab", "mpdm-ana--bo--cy-1", "Ana"], "{r}");
    let kinds: Vec<&str> = found.iter().map(|c| c["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["public", "group", "direct"], "{r}");
    let lab = found[0]["messages"].as_array().unwrap().clone();
    assert_eq!(lab.len(), 2, "the one from before is left out: {r}");
    assert_eq!(lab[0]["text"], "@Bo the run finished");
    assert!(lab[0].get("thread").is_none(), "{r}");
    // A reply in a thread begun long before is there, with its thread.
    assert_eq!(lab[1]["text"], "@Bo approve this?");
    assert_eq!(lab[1]["thread"], "1600000000.0");
    assert_eq!(
        found[2]["messages"][0]["text"],
        "can you look at the draft?"
    );
    assert_eq!(space["more"], false, "{r}");
    assert!(
        r["structuredContent"]["note"]
            .as_str()
            .unwrap()
            .contains("data")
    );
    // Searched from two days before the hours began: `after:` leaves out
    // the day it names, by the user's clock, wherever they are.
    let since = jiff::Timestamp::now().as_second() - 6 * 3600;
    let day = jiff::Timestamp::from_second(since - 2 * 86_400)
        .unwrap()
        .strftime("%Y-%m-%d")
        .to_string();
    let searched = searches(&asked);
    assert_eq!(searched.len(), 1, "{searched:?}");
    assert!(
        searched[0].contains(&format!("query=after%3A{day}")),
        "{searched:?}"
    );
}

#[test]
fn pages_are_read_until_one_reaches_before_the_hours() {
    let lab = || json!({"id": "C00", "name": "lab"});
    let page = |ago: i64, old: bool| {
        let mut v = vec![hit(lab(), ago, "new", "C00/p1")];
        if old {
            v.push(hit(lab(), 90_000, "old", "C00/p0"));
        }
        Value::Array(v)
    };
    // The second page reaches before: no third is asked for, and nothing
    // more is said to be there.
    let (base, asked) = searched(vec![page(100, false), page(200, true), page(300, false)], 5);
    let mut server = server_on(&base, true);
    let r = call(&mut server, "slack_recent", json!({ "hours": 24 }));
    let space = &r["structuredContent"]["workspaces"][0];
    assert_eq!(
        space["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "{r}"
    );
    assert_eq!(space["more"], false, "{r}");
    assert_eq!(searches(&asked).len(), 2);
    // Every page new: as many as are read, and more said to be there.
    let (base, asked) = searched(
        vec![page(100, false), page(200, false), page(300, false)],
        5,
    );
    let mut server = server_on(&base, true);
    let r = call(&mut server, "slack_recent", json!({ "hours": 24 }));
    let space = &r["structuredContent"]["workspaces"][0];
    assert_eq!(
        space["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "{r}"
    );
    assert_eq!(space["more"], true, "{r}");
    assert_eq!(searches(&asked).len(), 3);
}

#[test]
fn a_page_that_cannot_be_read_keeps_what_was_read_before_it() {
    let lab = json!({"id": "C00", "name": "lab"});
    // Page two is not answered: Slack refuses it.
    let (base, _) = searched(vec![json!([hit(lab, 100, "new", "C00/p1")])], 3);
    let mut server = server_on(&base, true);
    let r = call(&mut server, "slack_recent", json!({ "hours": 24 }));
    let space = &r["structuredContent"]["workspaces"][0];
    assert!(space.get("error").is_none(), "{r}");
    assert_eq!(
        space["conversations"][0]["messages"][0]["text"], "new",
        "{r}"
    );
    assert_eq!(space["more"], true, "{r}");
}

#[test]
fn with_no_slack_set_up_nothing_is_there_and_nothing_fails() {
    let mut server = demo_server(true);
    let r = call(&mut server, "slack_recent", json!({}));
    assert_ne!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["workspaces"], json!([]), "{r}");
}
