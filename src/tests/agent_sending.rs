//! A draft an agent sends: only once the user, asked by letterbox itself and
//! shown all that goes, says yes.

use super::agents::{call, demo_server};
use super::drafting::{a_file, answered, in_folder};
use serde_json::{Value, json};

fn has(server: &mut crate::mcp::Server, folder: &str, subject: &str) -> bool {
    in_folder(server, folder)
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
fn its_files_and_all_of_its_text_are_shown() {
    let (_dir, path) = a_file("report.pdf", "the report");
    let mut server = demo_server(true);
    let r = call(
        &mut server,
        "draft_message",
        json!({ "to": ["a@b.example"], "subject": "Files", "body": "See it. ".repeat(1000),
                "attachments": [path] }),
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
    assert_eq!(asked.matches("See it.").count(), 1000, "all of it");
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

/// The tool as `tools/list` offers it, if it does.
fn send_draft_tool(server: &mut crate::mcp::Server) -> Option<Value> {
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "send_draft")
        .cloned()
}

#[test]
fn it_is_not_offered_where_no_one_can_say_yes() {
    let offered = |server: &mut crate::mcp::Server| send_draft_tool(server).is_some();
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
    let inbox = in_folder(&mut server, "INBOX")[0].clone();
    let sent = in_folder(&mut server, "Sent").len();
    let (asked, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": inbox["uid"], "folder": "INBOX" }),
        "accept",
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(asked.is_empty(), "not even asked: {asked}");
    assert_eq!(in_folder(&mut server, "Sent").len(), sent);
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
    let original = in_folder(&mut server, "INBOX")
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
    let now = in_folder(&mut server, "INBOX")
        .into_iter()
        .find(|m| m["uid"] == original["uid"])
        .unwrap();
    assert_eq!(now["answered"], true, "{now}");
}

#[test]
fn an_agent_is_told_never_to_send_because_a_message_asked() {
    let mut server = demo_server(true);
    let tool = send_draft_tool(&mut server).unwrap();
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

/// A raw draft kept: its uid.
fn kept(server: &mut crate::mcp::Server, raw: &str) -> u32 {
    server
        .first_store()
        .append_draft("Drafts", raw.as_bytes())
        .unwrap()
        .unwrap()
}

#[test]
fn who_the_user_is_shown_is_who_it_goes_to_whatever_the_address_pretends() {
    let mut server = demo_server(true);
    let uid = kept(
        &mut server,
        "From: you@example.com\r\nTo: \"Ada\" <ada@good.example>(<evil@bad.example>)\r\n\
         Subject: Pretending\r\nMessage-ID: <p@example.com>\r\n\r\nHello\r\n",
    );
    let (asked, _) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": uid }),
        "decline",
    );
    assert!(asked.contains("To: evil@bad.example"), "{asked}");
    assert!(!asked.contains("To: \"Ada\""), "{asked}");
}

#[test]
fn what_goes_is_dated_now_with_an_id_of_its_own_and_nothing_of_a_draft() {
    let mut server = demo_server(true);
    let uid = kept(
        &mut server,
        "From: you@example.com\r\nTo: a@b.example\r\nSubject: Old\r\n\
         Date: Mon, 1 Jan 2001 00:00:00 +0000\r\nMessage-ID: <old@example.com>\r\n\
         X-Uniform-Type-Identifier: com.apple.mail-draft\r\n\
         X-Mozilla-Draft-Info: internal/draft; vcard=0\r\n\
         X-Unsent: 1\r\n\r\nHello\r\n",
    );
    let (_, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": uid }),
        "accept",
    );
    assert_eq!(r["structuredContent"]["sent"], true, "{r}");
    let sent = in_folder(&mut server, "Sent");
    let sent = sent.iter().find(|m| m["subject"] == "Old").unwrap();
    let raw = server
        .first_store()
        .raw("Sent", sent["uid"].as_u64().unwrap() as u32)
        .unwrap();
    let raw = String::from_utf8_lossy(&raw).to_string();
    assert!(!raw.contains("2001"), "{raw}");
    assert!(raw.contains("Date:"), "{raw}");
    assert!(!raw.contains("old@example.com"), "{raw}");
    assert!(raw.contains("Message-ID:"), "{raw}");
    for draft_only in ["X-Uniform", "X-Mozilla", "X-Unsent"] {
        assert!(!raw.contains(draft_only), "{draft_only}: {raw}");
    }
    assert!(raw.contains("Hello"), "{raw}");
    assert!(
        has(&mut server, "Trash", "Old"),
        "the draft itself still moved"
    );
}

#[test]
fn what_cannot_be_shown_or_sent_as_it_is_is_left_for_the_user_and_not_asked() {
    let head = "From: you@example.com\r\nTo: a@b.example\r\nMessage-ID: <w@example.com>\r\n";
    let long = "words ".repeat(5000);
    for (raw, why) in [
        (
            format!(
                "{head}Subject: Html\r\nMIME-Version: 1.0\r\n\
                 Content-Type: multipart/alternative; boundary=\"i\"\r\n\r\n\
                 --i\r\nContent-Type: text/plain\r\n\r\nShown\r\n\
                 --i\r\nContent-Type: text/html\r\n\r\n<a href=x>unseen</a>\r\n--i--\r\n"
            )
            .into_bytes(),
            "HTML",
        ),
        (format!("{head}Subject: Long\r\n\r\n{long}\r\n").into_bytes(), "too long"),
        (
            [
                format!("{head}Subject: Latin\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: 8bit\r\n\r\n")
                    .as_bytes(),
                b"caf\xe9\r\n",
            ]
            .concat(),
            "not UTF-8",
        ),
        (
            format!(
                "{head}Subject: Mixed\r\nMIME-Version: 1.0\r\n\
                 Content-Type: multipart/mixed; boundary=\"o\"\r\n\r\n\
                 --o\r\nContent-Type: text/plain; charset=utf-8\r\n\
                 Content-Transfer-Encoding: 8bit\r\n\r\ncafé\r\n--o--\r\n"
            )
            .into_bytes(),
            "8-bit",
        ),
    ] {
        let mut server = demo_server(true);
        let uid = server
            .first_store()
            .append_draft("Drafts", &raw)
            .unwrap()
            .unwrap();
        let sent = in_folder(&mut server, "Sent").len();
        let (asked, r) = answered(&mut server, 1, "send_draft", json!({ "uid": uid }), "accept");
        assert_eq!(r["isError"], true, "{why}: {r}");
        assert!(r.to_string().contains(why), "{why}: {r}");
        assert!(asked.is_empty(), "{why}: {asked}");
        assert_eq!(in_folder(&mut server, "Sent").len(), sent, "{why}");
    }
}

#[test]
fn a_folder_other_than_drafts_is_refused_not_ignored() {
    let (account, boxes) = super::serve_boxes(super::gmail_boxes());
    super::gmail_draft(&boxes);
    let mut server = demo_server(true);
    server.use_store(Box::new(crate::imap::Imap::new(account)));
    let (asked, r) = answered(
        &mut server,
        1,
        "send_draft",
        json!({ "uid": 900, "folder": "INBOX" }),
        "accept",
    );
    assert_eq!(r["isError"], true, "{r}");
    assert!(r.to_string().contains("Drafts"), "{r}");
    assert!(asked.is_empty(), "{asked}");
}

/// The user's answer as a program sends it that shows only Yes and No: the
/// default of what it was asked, on yes. The prompt is `drafted`'s, once.
fn pressed(server: &mut crate::mcp::Server, uid: &Value, action: &str) -> Value {
    let (requests, replies) = std::sync::mpsc::channel::<String>();
    let out = super::agents::Written::default();
    let lines = {
        let out = out.clone();
        let mut asked = false;
        let first = json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "send_draft", "arguments": { "uid": uid } } })
        .to_string();
        requests.send(first).unwrap();
        let action = action.to_string();
        std::iter::from_fn(move || {
            if let Ok(line) = replies.try_recv() {
                return Some(Ok(line));
            }
            if asked {
                return None;
            }
            asked = true;
            let written = out.messages();
            let question = written
                .iter()
                .rfind(|m| m["method"] == "elicitation/create")?;
            let schema = &question["params"]["requestedSchema"]["properties"];
            let content: serde_json::Map<String, Value> = schema
                .as_object()?
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.get("default")?.clone())))
                .collect();
            Some(Ok(json!({ "jsonrpc": "2.0", "id": question["id"],
                "result": { "action": action, "content": content } })
            .to_string()))
        })
    };
    server
        .serve(crate::mcp::Io {
            lines: Box::new(lines),
            out: Box::new(out.clone()),
        })
        .unwrap();
    let written = out.messages();
    written.iter().find(|m| m["id"] == 10).unwrap()["result"].clone()
}

#[test]
fn yes_pressed_in_a_program_that_shows_only_yes_and_no_sends_it() {
    let mut server = demo_server(true);
    let uid = drafted(&mut server, "Pressed yes");
    let r = pressed(&mut server, &uid, "accept");
    assert_eq!(r["structuredContent"]["sent"], true, "{r}");
    assert!(has(&mut server, "Sent", "Pressed yes"));
}

#[test]
fn no_pressed_there_sends_nothing() {
    let mut server = demo_server(true);
    let uid = drafted(&mut server, "Pressed no");
    let r = pressed(&mut server, &uid, "decline");
    assert_eq!(r["structuredContent"]["sent"], false, "{r}");
    assert!(!has(&mut server, "Sent", "Pressed no"));
    assert!(has(&mut server, "Drafts", "Pressed no"));
}

#[test]
fn a_reply_that_goes_marks_what_it_answers_wherever_it_is_filed_and_learns_it_is_kept() {
    let mut server = demo_server(true);
    let original = in_folder(&mut server, "Archive")
        .into_iter()
        .find(|m| m["answered"] == false)
        .expect("something archived");
    let r = call(
        &mut server,
        "draft_reply",
        json!({ "folder": "Archive", "uid": original["uid"], "body": "Yes." }),
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
    let now = in_folder(&mut server, "Archive")
        .into_iter()
        .find(|m| m["uid"] == original["uid"])
        .unwrap();
    assert_eq!(now["answered"], true, "{now}");
    let learnt: Vec<bool> = server.learnt.iter().map(|t| t.thrown).collect();
    assert_eq!(learnt, [false], "kept, the original alone");
}

#[test]
fn what_is_asked_has_no_field_of_its_own_the_programs_yes_and_no_are_the_answer() {
    let mut server = demo_server(true);
    let uid = drafted(&mut server, "No field");
    let script = vec![
        json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "send_draft", "arguments": { "uid": uid } } })
        .to_string(),
        json!({ "jsonrpc": "2.0", "id": "letterbox-1", "result": { "action": "accept" } })
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
    let asked = written
        .iter()
        .find(|m| m["method"] == "elicitation/create")
        .unwrap();
    let schema = &asked["params"]["requestedSchema"];
    assert_eq!(schema["properties"], json!({}), "{asked}");
    assert_eq!(schema["type"], "object", "{asked}");
    let reply = written.iter().find(|m| m["id"] == 10).unwrap();
    assert_eq!(
        reply["result"]["structuredContent"]["sent"], true,
        "yes alone: {reply}"
    );
}

#[test]
fn a_draft_to_many_is_left_for_the_user_and_not_asked_about() {
    let to = |n: usize| -> Vec<String> { (0..n).map(|i| format!("p{i}@b.example")).collect() };
    for (to_count, cc_count, sends) in [(10, 0, true), (6, 4, true), (11, 0, false), (6, 5, false)]
    {
        let mut server = demo_server(true);
        let r = call(
            &mut server,
            "draft_message",
            json!({ "to": to(to_count), "cc": to(cc_count).iter().map(|a| format!("c{a}")).collect::<Vec<_>>(),
                    "subject": "Many", "body": "Hello" }),
        );
        let uid = r["structuredContent"]["uid"].clone();
        let (asked, r) = answered(
            &mut server,
            1,
            "send_draft",
            json!({ "uid": uid }),
            "accept",
        );
        assert_eq!(
            has(&mut server, "Sent", "Many"),
            sends,
            "{to_count}+{cc_count}: {r}"
        );
        if !sends {
            assert_eq!(r["isError"], true, "{r}");
            assert!(r.to_string().contains("more than 10"), "{r}");
            assert!(asked.is_empty(), "{asked}");
            assert!(has(&mut server, "Drafts", "Many"));
        }
    }
}
