//! Letters an agent drafts: kept in Drafts, never sent; and any draft taken
//! up again in letterbox, to go on writing it and send it.

use super::*;
use serde_json::json;

pub(super) fn in_folder(server: &mut crate::mcp::Server, folder: &str) -> Vec<serde_json::Value> {
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
    app.settle(PATIENCE);
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

/// A file of a folder not hidden, as a temporary folder's own name would be.
pub(super) fn a_file(name: &str, says: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::Builder::new().prefix("lbx").tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, says).unwrap();
    let path = path.to_str().unwrap().to_string();
    (dir, path)
}

#[test]
fn an_agent_attaches_a_file_the_user_named_to_a_new_letter_and_to_a_reply() {
    let (_dir, path) = a_file("notes.txt", "the notes");
    let mut server = demo_server(false);
    let original = in_folder(&mut server, "INBOX")[0].clone();
    let sent = in_folder(&mut server, "Sent").len();
    for (tool, args) in [
        (
            "draft_message",
            json!({ "to": ["a@b.example"], "subject": "New with a file", "body": "see it",
                    "attachments": [path] }),
        ),
        (
            "draft_reply",
            json!({ "uid": original["uid"], "body": "see it", "attachments": [path] }),
        ),
    ] {
        let r = call(&mut server, tool, args);
        assert_eq!(r["isError"], false, "{r}");
        assert_eq!(
            r["structuredContent"]["attached"],
            json!([{ "name": "notes.txt", "size": 9 }]),
            "{r}"
        );
    }
    assert_eq!(in_folder(&mut server, "Sent").len(), sent, "nothing sent");
    let drafts = in_folder(&mut server, "Drafts");
    for wanted in ["New with a file", "Re:"] {
        let draft = drafts
            .iter()
            .find(|m| m["subject"].as_str().unwrap().contains(wanted))
            .unwrap();
        let read = call(
            &mut server,
            "read_message",
            json!({ "folder": "Drafts", "uid": draft["uid"] }),
        );
        let got = read["structuredContent"]["attachments"].to_string();
        assert!(got.contains("notes.txt"), "{read}");
    }
}

#[test]
fn an_agent_attaches_nothing_unsure_and_no_draft_is_kept_for_it() {
    let dir = tempfile::Builder::new().prefix("lbx").tempdir().unwrap();
    let hidden = dir.path().join(".ssh");
    std::fs::create_dir(&hidden).unwrap();
    let key = hidden.join("id");
    std::fs::write(&key, "a key").unwrap();
    let link = dir.path().join("looks-fine.txt");
    std::os::unix::fs::symlink(&key, &link).unwrap();
    let huge = dir.path().join("huge.bin");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(crate::mcp::ATTACHED_AT_MOST + 1)
        .unwrap();
    let path = |p: &std::path::Path| p.to_str().unwrap().to_string();
    let mut server = demo_server(false);
    let drafts = in_folder(&mut server, "Drafts").len();
    for (given, why) in [
        (json!(["notes.txt"]), "whole path"),
        (json!([path(&dir.path().join("none.txt"))]), "cannot"),
        (json!([path(dir.path())]), "not a file"),
        (json!([path(&key)]), "hidden"),
        (json!([path(&link)]), "hidden"),
        (json!([path(&huge)]), "too much"),
        (json!("notes.txt"), "list"),
    ] {
        let r = call(
            &mut server,
            "draft_message",
            json!({ "to": ["a@b.example"], "subject": "x", "body": "y", "attachments": given }),
        );
        assert_eq!(r["isError"], true, "{given}: {r}");
        assert!(r.to_string().contains(why), "{why}: {r}");
    }
    assert_eq!(in_folder(&mut server, "Drafts").len(), drafts);
}

#[test]
fn an_agent_attaches_nothing_of_what_the_machine_or_letterbox_keeps_to_itself() {
    let dir = tempfile::Builder::new().prefix("lbx").tempdir().unwrap();
    let config = dir.path().join("accounts.toml");
    std::fs::write(&config, "").unwrap();
    let mut server = crate::mcp::Server::new(&config, true).unwrap();
    let draft = |server: &mut crate::mcp::Server, path: &std::path::Path| {
        call(
            server,
            "draft_message",
            json!({ "to": ["a@b.example"], "subject": "x", "body": "y",
                    "attachments": [path.to_str().unwrap()] }),
        )
    };
    let beside = dir.path().join("notes.txt");
    std::fs::write(&beside, "kept beside it").unwrap();
    let refused = draft(&mut server, &beside);
    assert_eq!(refused["isError"], true, "{refused}");
    let refused = draft(&mut server, &config);
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(refused.to_string().contains("keeps to itself"), "{refused}");
    #[cfg(target_os = "linux")]
    {
        let r = draft(&mut server, std::path::Path::new("/proc/self/environ"));
        assert_eq!(r["isError"], true, "{r}");
        assert!(r.to_string().contains("keeps to itself"), "{r}");
    }
}

#[test]
fn a_file_attached_by_a_link_goes_by_the_name_the_user_gave_it() {
    let (dir, target) = a_file("ab12cd", "the report");
    let link = dir.path().join("report.pdf");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let mut server = demo_server(false);
    let r = call(
        &mut server,
        "draft_message",
        json!({ "to": ["a@b.example"], "subject": "x", "body": "y",
                "attachments": [link.to_str().unwrap()] }),
    );
    assert_eq!(
        r["structuredContent"]["attached"],
        json!([{ "name": "report.pdf", "size": 10 }]),
        "{r}"
    );
}

#[test]
fn a_draft_picked_says_first_of_all_that_e_edits_it() {
    let mut app = demo();
    let foot = |app: &mut App| app::screen(app, 200, 40);
    let inbox = foot(&mut app);
    assert!(!inbox.contains("edit this draft"), "{inbox}");
    let drafts = app
        .rows
        .iter()
        .position(
            |r| matches!(*r, app::Row::Folder(0, i) if app.accounts[0].folders[i].name == "Drafts"),
        )
        .unwrap();
    app.focus = Focus::Side;
    app.row = drafts;
    press(&mut app, &["<enter>"]);
    app.settle(PATIENCE);
    let screen = foot(&mut app);
    let line = screen
        .lines()
        .find(|l| l.contains("edit this draft"))
        .unwrap_or_else(|| panic!("{screen}"));
    let at = line.find("E edit this draft").unwrap();
    assert!(line[..at].trim().is_empty(), "first of the keys: {line}");
}

#[test]
fn what_is_off_limits_is_known_by_any_of_its_names_and_the_cloud_is_not() {
    use crate::mcp::off_limits;
    use std::path::Path;
    let config = Path::new("/nowhere/accounts.toml");
    let home = crate::compose::home().canonicalize().unwrap();
    let no = |p: &Path| assert!(off_limits(p, config), "{}", p.display());
    no(&home.join("Library/Keychains/login.keychain-db"));
    no(&Path::new("/System/Volumes/Data")
        .join(home.strip_prefix("/").unwrap())
        .join("Library/Keychains/x"));
    no(Path::new("/System/Volumes/Data/private/etc/passwd"));
    no(Path::new("/etc/passwd"));
    for fine in [
        "Library/CloudStorage/Dropbox/report.pdf",
        "Library/Mobile Documents/com~apple~CloudDocs/report.pdf",
        "Documents/report.pdf",
    ] {
        assert!(!off_limits(&home.join(fine), config), "{fine}");
    }
}

#[test]
fn a_tool_given_null_for_what_it_may_leave_out_leaves_it_out() {
    let mut server = demo_server(false);
    let r = call(
        &mut server,
        "draft_message",
        json!({ "to": ["a@b.example"], "cc": null, "subject": "x", "body": "y",
                "attachments": null }),
    );
    assert_eq!(r["isError"], false, "{r}");
}

// ---- What an agent is told of what it drafted ----

#[test]
fn a_draft_kept_says_which_it_is_who_it_goes_to_and_how_big_its_files_are() {
    let (_dir, path) = a_file("notes.txt", "the notes");
    let mut server = demo_server(false);
    let args = json!({ "to": ["a@b.example"], "cc": ["c@d.example"], "subject": "Twice",
                       "body": "see it", "attachments": [path] });
    let first = call(&mut server, "draft_message", args.clone());
    let again = call(&mut server, "draft_message", args);
    let (first, again) = (&first["structuredContent"], &again["structuredContent"]);
    assert_ne!(first["uid"], again["uid"], "told apart: {first} {again}");
    assert_eq!(first["subject"], "Twice", "{first}");
    assert_eq!(first["to"], json!(["a@b.example"]), "{first}");
    assert_eq!(first["cc"], json!(["c@d.example"]), "{first}");
    assert_eq!(
        first["attached"],
        json!([{ "name": "notes.txt", "size": 9 }]),
        "{first}"
    );
    let whole = call(&mut server, "list_accounts", json!({}));
    assert_eq!(
        first["account"], whole["structuredContent"]["accounts"][0]["name"],
        "{first}"
    );
    let read = call(
        &mut server,
        "read_message",
        json!({ "folder": first["saved_in"], "uid": first["uid"] }),
    );
    assert_eq!(read["structuredContent"]["subject"], "Twice", "{read}");
}

/// The demo's server, holding a letter to the user and two more, one
/// copied: its uid.
fn holding_one_to_many(server: &mut crate::mcp::Server) -> serde_json::Value {
    server
        .first_store()
        .append(
            "INBOX",
            b"From: Ada <ada@analytical.example>\r\nTo: you@example.com, Bob <bob@b.example>\r\n\
              Cc: Vice Dean <dean@u.example>\r\nSubject: To many\r\n\
              Date: Sat, 1 Jan 2050 00:00:00 +0000\r\nMessage-ID: <many@example.com>\r\n\r\nHi\r\n",
            true,
        )
        .unwrap();
    let found = call(server, "search_messages", json!({ "query": "To many" }));
    found["structuredContent"]["messages"][0]["uid"].clone()
}

#[test]
fn a_reply_to_the_sender_alone_says_who_it_leaves_out() {
    let mut server = demo_server(false);
    let uid = holding_one_to_many(&mut server);
    let r = call(
        &mut server,
        "draft_reply",
        json!({ "uid": uid, "body": "Yes" }),
    );
    let r = &r["structuredContent"];
    assert_eq!(r["to"], json!(["Ada <ada@analytical.example>"]), "{r}");
    assert_eq!(r["cc"], json!([]), "{r}");
    assert_eq!(
        r["not_copied"],
        json!(["Bob <bob@b.example>", "Vice Dean <dean@u.example>"]),
        "the user is not among them: {r}"
    );
    assert!(
        r["said"].as_str().unwrap().contains("with all"),
        "what would copy them: {r}"
    );
}

#[test]
fn a_reply_to_all_leaves_no_one_out_and_a_reply_to_one_alone_says_nothing_of_it() {
    let mut server = demo_server(false);
    let uid = holding_one_to_many(&mut server);
    let r = call(
        &mut server,
        "draft_reply",
        json!({ "uid": uid, "body": "Yes", "all": true }),
    );
    let r = &r["structuredContent"];
    assert_eq!(r["not_copied"], json!([]), "{r}");
    assert!(!r["said"].as_str().unwrap().contains("with all"), "{r}");
    assert_eq!(
        r["cc"],
        json!(["Bob <bob@b.example>", "Vice Dean <dean@u.example>"]),
        "{r}"
    );
    let sole = call(
        &mut server,
        "draft_message",
        json!({ "to": ["a@b.example"], "subject": "x", "body": "y" }),
    );
    assert!(
        sole["structuredContent"].get("not_copied").is_none(),
        "{sole}"
    );
}

#[test]
fn a_draft_kept_on_a_server_is_known_by_the_uid_the_server_says() {
    let (account, boxes) = server();
    let mut imap = Imap::new(account);
    let uid = imap
        .append_draft("Sent Items", b"From: me@example.com\r\n\r\nhi\r\n")
        .unwrap();
    let held = boxes.lock().unwrap();
    let kept = held.iter().find(|b| b.name == "Sent Items").unwrap();
    assert_eq!(uid, kept.letters.last().map(|l| l.uid));
}

#[test]
fn a_server_that_does_not_say_is_asked_by_message_id_never_guessed_at() {
    let (account, boxes) = server();
    let kept = |boxes: &Boxes, says: &str| {
        let held = boxes.lock().unwrap();
        let sent = held.iter().find(|b| b.name == "Sent Items").unwrap();
        sent.letters
            .iter()
            .find(|l| String::from_utf8_lossy(&l.raw).contains(says))
            .map(|l| l.uid)
    };
    boxes
        .lock()
        .unwrap()
        .iter_mut()
        .find(|b| b.name == "Sent Items")
        .unwrap()
        .says_appenduid = false;
    let mut imap = Imap::new(account);
    let uid = imap
        .append_draft(
            "Sent Items",
            b"From: me@example.com\r\nMessage-ID: <wanted@example.com>\r\n\r\nhi\r\n",
        )
        .unwrap();
    imap.append(
        "Sent Items",
        b"From: me@example.com\r\nMessage-ID: <later@example.com>\r\n\r\nlater\r\n",
        true,
    )
    .unwrap();
    assert_eq!(uid, kept(&boxes, "wanted@"));
    // With no Message-ID to ask by, it says it cannot tell.
    let told = imap
        .append_draft("Sent Items", b"From: me@example.com\r\n\r\nno id\r\n")
        .unwrap();
    assert_eq!(told, None, "not the highest there is");
}

/// The user's answer to what they were asked, and the tool called to ask.
pub(super) fn answered(
    server: &mut crate::mcp::Server,
    asking: u32,
    tool: &str,
    args: serde_json::Value,
    action: &str,
) -> (String, serde_json::Value) {
    answered_with(
        server,
        asking,
        tool,
        args,
        action,
        json!({ "confirm": true }),
    )
}

/// As `answered`, what they said given.
pub(super) fn answered_with(
    server: &mut crate::mcp::Server,
    asking: u32,
    tool: &str,
    args: serde_json::Value,
    action: &str,
    content: serde_json::Value,
) -> (String, serde_json::Value) {
    let script = vec![
        json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": tool, "arguments": args } })
        .to_string(),
        json!({ "jsonrpc": "2.0", "id": format!("letterbox-{asking}"),
            "result": { "action": action, "content": content } })
        .to_string(),
    ];
    let out = Written::default();
    server
        .serve(crate::mcp::Io {
            lines: Box::new(script.into_iter().map(Ok)),
            out: Box::new(out.clone()),
        })
        .unwrap();
    let written = out.messages();
    let asked = written
        .iter()
        .rfind(|m| m["method"] == "elicitation/create")
        .map(|m| m["params"]["message"].as_str().unwrap().to_string())
        .unwrap_or_default();
    let reply = written.iter().find(|m| m["id"] == 10).unwrap().clone();
    (asked, reply["result"].clone())
}

#[test]
fn a_draft_thrown_away_is_asked_about_told_apart_from_its_twin_and_kept_in_trash() {
    let (_dir, path) = a_file("big.bin", &"x".repeat(5000));
    let mut server = demo_server(true);
    let twin = |attachments: serde_json::Value| json!({ "to": ["a@b.example"], "subject": "Twin", "body": "y", "attachments": attachments });
    let old = call(&mut server, "draft_message", twin(json!([])));
    let new = call(&mut server, "draft_message", twin(json!([path])));
    let old = old["structuredContent"]["uid"].clone();
    let new = new["structuredContent"]["uid"].clone();
    let listed = in_folder(&mut server, "Drafts");
    let was = listed.iter().find(|m| m["uid"] == old).unwrap();
    let year = was["date"].as_str().unwrap()[..4].to_string();

    let (asked, r) = answered(
        &mut server,
        1,
        "trash_messages",
        json!({ "folder": "Drafts", "uids": [old] }),
        "decline",
    );
    assert_eq!(r["structuredContent"]["moved"], 0, "{r}");
    assert!(asked.contains("Twin"), "{asked}");
    assert!(asked.contains(&year), "when it was written: {asked}");
    let raw = server
        .first_store()
        .raw("Drafts", old.as_u64().unwrap() as u32)
        .unwrap();
    assert!(
        asked.contains(&crate::ui::size(raw.len())),
        "how big, as twins differ: {asked}"
    );
    let kept = |server: &mut crate::mcp::Server, uid: &serde_json::Value| {
        in_folder(server, "Drafts").iter().any(|m| m["uid"] == *uid)
    };
    assert!(kept(&mut server, &old), "the user said no");
    assert!(kept(&mut server, &new));

    let trash = in_folder(&mut server, "Trash").len();
    let (_, r) = answered(
        &mut server,
        2,
        "trash_messages",
        json!({ "folder": "Drafts", "uids": [old] }),
        "accept",
    );
    assert_eq!(r["structuredContent"]["moved"], 1, "{r}");
    assert_eq!(r["structuredContent"]["learnt"], 0, "{r}");
    assert!(server.learnt.is_empty());
    assert!(!kept(&mut server, &old), "gone from Drafts");
    assert!(kept(&mut server, &new), "its twin stays");
    assert_eq!(
        in_folder(&mut server, "Trash").len(),
        trash + 1,
        "not gone for good"
    );
}

#[test]
fn what_is_thrown_away_of_any_other_folder_is_listed_as_ever() {
    let mut server = demo_server(true);
    let uid = in_folder(&mut server, "INBOX")[0]["uid"].clone();
    let (asked, _) = answered(
        &mut server,
        1,
        "trash_messages",
        json!({ "uids": [uid] }),
        "decline",
    );
    assert!(!asked.contains(" ["), "{asked}");
}

#[test]
fn an_agent_is_told_how_files_go_on_a_draft_and_what_becomes_of_one_thrown_away() {
    let mut server = demo_server(true);
    let init = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": { "elicitation": {} } } }))
        .unwrap();
    let said = init["result"]["instructions"].as_str().unwrap();
    assert!(said.contains("attachments"), "{said}");
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    let tool = |name: &str| {
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()
            .clone()
    };
    for name in ["draft_reply", "draft_message"] {
        let t = tool(name);
        let about = t["description"].as_str().unwrap();
        assert!(about.contains("described"), "{name}: {about}");
        assert!(about.contains("trash_messages"), "how to redo one: {about}");
        let files = t["inputSchema"]["properties"]["attachments"]["description"]
            .as_str()
            .unwrap();
        assert!(files.contains("described"), "{name}: {files}");
    }
    let trash = tool("trash_messages");
    assert!(
        trash["description"].as_str().unwrap().contains("draft"),
        "{trash}"
    );
    let all = tool("draft_reply")["inputSchema"]["properties"]["all"]["description"].clone();
    assert!(all.as_str().unwrap().contains("not_copied"), "{all}");
}
