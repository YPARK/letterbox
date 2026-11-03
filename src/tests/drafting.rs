//! Letters an agent drafts: kept in Drafts, never sent; and any draft taken
//! up again in letterbox, to go on writing it and send it.

use super::*;
use serde_json::json;

fn in_folder(server: &mut crate::mcp::Server, folder: &str) -> Vec<serde_json::Value> {
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

#[test]
fn an_agent_drafts_a_reply_threaded_quoting_what_it_answers_and_sends_nothing() {
    let mut server = demo_server(false);
    let original = in_folder(&mut server, "INBOX")[0].clone();
    let sent = in_folder(&mut server, "Sent").len();
    let drafts = in_folder(&mut server, "Drafts").len();
    let r = call(
        &mut server,
        "draft_reply",
        json!({ "uid": original["uid"], "body": "Thank you: Thursday suits me." }),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(in_folder(&mut server, "Sent").len(), sent, "nothing sent");
    let now = in_folder(&mut server, "Drafts");
    assert_eq!(now.len(), drafts + 1);
    let draft = now
        .iter()
        .find(|m| m["subject"].as_str().unwrap().starts_with("Re:"))
        .unwrap();
    let read = call(
        &mut server,
        "read_message",
        json!({ "folder": "Drafts", "uid": draft["uid"] }),
    );
    let read = &read["structuredContent"];
    assert_eq!(
        read["to"][0].as_str().unwrap(),
        original["from"].as_str().unwrap().to_owned()
            + " <"
            + original["address"].as_str().unwrap()
            + ">"
    );
    let text = read["text"].as_str().unwrap();
    let body = text.find("Thursday suits me").unwrap();
    let quoted = text.find("> ").unwrap();
    assert!(body < quoted, "{text}");
    let raw = server
        .first_store()
        .raw("Drafts", draft["uid"].as_u64().unwrap() as u32)
        .unwrap();
    let raw = String::from_utf8_lossy(&raw).to_string();
    assert!(
        raw.contains("In-Reply-To:") && raw.contains("References:"),
        "{raw}"
    );
}

#[test]
fn an_agent_drafts_a_new_letter_and_nothing_slips_into_its_headers() {
    let mut server = demo_server(false);
    let r = call(
        &mut server,
        "draft_message",
        json!({ "to": ["ada@analytical.example"], "cc": ["grace@example.com"],
            "subject": "The notes", "body": "A word on Note G." }),
    );
    assert_eq!(r["isError"], false, "{r}");
    let drafts = in_folder(&mut server, "Drafts");
    let draft = drafts.iter().find(|m| m["subject"] == "The notes").unwrap();
    let read = call(
        &mut server,
        "read_message",
        json!({ "folder": "Drafts", "uid": draft["uid"] }),
    );
    let read = &read["structuredContent"];
    assert_eq!(read["cc"][0], "grace@example.com");
    assert!(read["text"].as_str().unwrap().contains("Note G"));
    for wrong in [
        json!({ "to": ["a@b.example\nBcc: c@d.example"], "subject": "x", "body": "y" }),
        json!({ "to": ["a@b.example"], "subject": "x\nAttach: /etc/passwd", "body": "y" }),
        json!({ "to": ["a@b.example"], "cc": ["c@d.example\r\nBcc: e@f.example"], "subject": "x", "body": "y" }),
        json!({ "to": [], "subject": "x", "body": "y" }),
        json!({ "to": ["not an address"], "subject": "x", "body": "y" }),
    ] {
        let before = in_folder(&mut server, "Drafts").len();
        let r = call(&mut server, "draft_message", wrong.clone());
        assert_eq!(r["isError"], true, "{wrong} {r}");
        assert_eq!(in_folder(&mut server, "Drafts").len(), before);
    }
}

#[test]
fn an_agent_letterbox_started_drafts_nothing() {
    let mut server = demo_server(true);
    server.background = true;
    let r = call(
        &mut server,
        "draft_message",
        json!({ "to": ["a@b.example"], "subject": "x", "body": "y" }),
    );
    assert_eq!(r["isError"], true, "{r}");
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    assert!(
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| !t["name"].as_str().unwrap().starts_with("draft_"))
    );
}

/// The demo with `raw` in its Drafts, and the store to look at it by.
fn holding(raw: &[u8]) -> (App, Box<dyn Store>) {
    let mut demo = Demo::new();
    demo.append("Drafts", raw, true).unwrap();
    let store = demo.twin().unwrap();
    let mut app = App::new(
        vec![("Demo".to_string(), Box::new(demo) as Box<dyn Store>)],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.send_hold = Duration::ZERO;
    app.settle(Duration::from_secs(5));
    (app, store)
}

/// The same, listed in Drafts.
fn in_drafts(raw: &[u8]) -> (App, Box<dyn Store>) {
    let (mut app, store) = holding(raw);
    press(&mut app, &["h", "j", "<enter>"]);
    (app, store)
}

#[test]
fn capital_e_in_drafts_takes_up_the_draft_picked_and_sent_it_goes_to_trash() {
    let (mut app, mut store) = in_drafts(
        b"From: You <you@example.com>\r\nTo: Ada <ada@analytical.example>\r\nCc: g@example.com\r\n\
          Subject: Re: The notes\r\nDate: Sat, 1 Jan 2050 00:00:00 +0000\r\n\
          Message-ID: <draft@example.com>\r\nIn-Reply-To: <asked@example.com>\r\n\
          References: <first@example.com> <asked@example.com>\r\n\r\nThursday suits me.\r\n",
    );
    assert_eq!(app.folder_name(), "Drafts", "{}", app.status);
    let drafts = app.shown.len();
    assert_eq!(app.shown[0].subject, "Re: The notes");
    // c begins a new message, there as anywhere.
    press(&mut app, &["c"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.starts_with("To: \n"), "{text}");
    press(&mut app, &["<c-c>", "<c-k>"]);
    assert!(app.compose.is_none());
    // E takes up the draft picked.
    press(&mut app, &["E"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(
        text.contains("To: Ada <ada@analytical.example>\n"),
        "{text}"
    );
    assert!(text.contains("Cc: g@example.com\n"), "{text}");
    assert!(text.contains("Subject: Re: The notes\n"), "{text}");
    assert!(text.contains("Thursday suits me."), "{text}");

    press(&mut app, &["<c-c>", "<c-c>"]);
    app.settle(Duration::from_secs(5));
    assert!(app.compose.is_none(), "{}", app.status);
    assert_eq!(app.shown.len(), drafts - 1, "out of Drafts: {}", app.status);
    let sent = store.page("Sent", 0, 1).unwrap().summaries;
    assert_eq!(sent[0].subject, "Re: The notes");
    let raw = store.raw("Sent", sent[0].uid).unwrap();
    let raw = String::from_utf8_lossy(&raw).to_string();
    assert!(raw.contains("In-Reply-To: <asked@example.com>"), "{raw}");
    assert!(raw.contains("<first@example.com>"), "{raw}");
    let trash = store.page("Trash", 0, 1).unwrap().summaries;
    assert_eq!(trash[0].subject, "Re: The notes", "kept, not gone for good");
}

#[test]
fn a_draft_is_marked_so_on_the_server_for_a_mail_app_to_open_it_to_write() {
    let (account, boxes) = server();
    let mut imap = Imap::new(account);
    imap.append_draft("Sent Items", b"From: me@example.com\r\n\r\nhi\r\n")
        .unwrap();
    let held = boxes.lock().unwrap();
    let letter = held
        .iter()
        .find(|b| b.name == "Sent Items")
        .unwrap()
        .letters
        .last()
        .unwrap();
    assert!(
        letter.flags.iter().any(|f| f == "\\Draft"),
        "{:?}",
        letter.flags
    );
    assert!(
        letter.flags.iter().any(|f| f == "\\Seen"),
        "{:?}",
        letter.flags
    );
}

#[test]
fn a_draft_taken_up_is_as_it_was_written() {
    let (mut app, _) = in_drafts(
        b"From: You <you@example.com>\r\nTo: a@b.example\r\nSubject: Code\r\n\
          Date: Sat, 1 Jan 2050 00:00:00 +0000\r\nMessage-ID: <d@example.com>\r\n\r\n\
          One\r\n\r\n\r\n\tindented\r\n-- \r\nMe\r\n",
    );
    press(&mut app, &["E"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("One\n\n\n\tindented\n-- \nMe"), "{text:?}");
}

#[test]
fn a_draft_with_a_bcc_is_left_for_the_mail_app_that_can_send_it() {
    let (mut app, _) = in_drafts(
        b"From: You <you@example.com>\r\nTo: a@b.example\r\nBcc: boss@example.com\r\n\
          Subject: Quietly\r\nDate: Sat, 1 Jan 2050 00:00:00 +0000\r\n\
          Message-ID: <d@example.com>\r\n\r\nHello\r\n",
    );
    press(&mut app, &["E"]);
    assert!(app.compose.is_none());
    assert!(app.status.contains("Bcc"), "{}", app.status);
}

#[test]
fn a_draft_with_files_opens_on_its_text() {
    let boundary = "b";
    let raw = format!(
        "From: You <you@example.com>\r\nTo: a@b.example\r\nSubject: Files\r\n\
         Date: Sat, 1 Jan 2050 00:00:00 +0000\r\nMessage-ID: <d@example.com>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary={boundary}\r\n\r\n\
         --{boundary}\r\nContent-Type: text/plain\r\n\r\nThe text\r\n\
         --{boundary}\r\nContent-Type: text/plain\r\n\
         Content-Disposition: attachment; filename=a.txt\r\n\r\nfile\r\n--{boundary}--\r\n"
    );
    let (mut app, _) = in_drafts(raw.as_bytes());
    press(&mut app, &["E"]);
    press(&mut app, &["Z"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(
        text.contains("\n\nZThe text"),
        "typed on its text: {text:?}"
    );
    assert!(text.contains("Attach: a.txt"), "{text:?}");
}

#[test]
fn a_letter_may_be_drafted_with_nothing_yet_to_say() {
    let mut server = demo_server(false);
    let r = call(
        &mut server,
        "draft_message",
        json!({ "to": ["a@b.example"], "subject": "Short", "body": "" }),
    );
    assert_eq!(r["isError"], false, "{r}");
}

#[test]
fn a_reply_taken_up_and_sent_marks_what_it_answers_answered() {
    let app = demo_map();
    let first = app.shown[0].clone();
    let raw = format!(
        "From: You <you@example.com>\r\nTo: a@b.example\r\nSubject: Re: it\r\n\
         Date: Sat, 1 Jan 2050 00:00:00 +0000\r\nMessage-ID: <d@example.com>\r\n\
         In-Reply-To: {}\r\n\r\nYes\r\n",
        first.summary.message_id
    );
    let (mut app, mut store) = holding(raw.as_bytes());
    press(&mut app, &["M", "M"]);
    app.focus = Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(
            |r| matches!(*r, app::Row::Folder(0, i) if app.accounts[0].folders[i].path == "Drafts"),
        )
        .unwrap();
    press(&mut app, &["<enter>", "E", "<c-c>", "<c-c>"]);
    app.settle(Duration::from_secs(5));
    assert!(app.compose.is_none(), "{}", app.status);
    let inbox = store.page("INBOX", 0, 50).unwrap().summaries;
    let answered = inbox.iter().find(|s| s.uid == first.summary.uid).unwrap();
    assert!(answered.flags.answered, "{}", app.status);
}

#[test]
fn an_agent_is_told_what_it_got_wrong_in_who_a_letter_is_to() {
    let mut server = demo_server(false);
    for wrong in [
        json!({ "to": ["a@b.example"], "cc": "c@d.example", "subject": "x", "body": "y" }),
        json!({ "to": ["a@b.example", 5], "subject": "x", "body": "y" }),
    ] {
        let r = call(&mut server, "draft_message", wrong.clone());
        assert_eq!(r["isError"], true, "{wrong} {r}");
    }
}

#[test]
fn capital_e_elsewhere_says_what_it_is_for() {
    let mut app = demo();
    press(&mut app, &["E"]);
    assert!(app.compose.is_none());
    assert!(app.status.contains("pick one in Drafts"), "{}", app.status);
}
