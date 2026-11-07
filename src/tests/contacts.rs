//! Who the user has written to and heard from, as the index knows it.

use super::*;
use crate::index::contacts::Contact;

/// A record of mail from `from`, to `to`, on `date`; sent by the user if `sent`.
fn record(
    id: &str,
    from: &str,
    to: &str,
    cc: &str,
    date: &str,
    sent: bool,
    more: &str,
) -> index::Record {
    let cc = if cc.is_empty() {
        String::new()
    } else {
        format!("Cc: {cc}\r\n")
    };
    let raw = format!(
        "From: {from}\r\nTo: {to}\r\n{cc}Subject: s\r\nDate: {date}\r\nMessage-ID: <{id}@x>\r\n{more}\r\nHi\r\n"
    );
    let indexed = crate::index::record::Indexed {
        uid: 1,
        raw: raw.into_bytes(),
        ..Default::default()
    };
    let mut r = index::Record::of("me@host", "INBOX", 7, &indexed, 1_000);
    r.key.id = id.to_string();
    if sent {
        r.server.push("\\Sent".to_string());
    }
    r
}

const OLD: &str = "Mon, 1 Jan 2001 00:00:00 +0000";
const NEW: &str = "Mon, 1 Jan 2024 00:00:00 +0000";
const ME: &str = "Me Myself <me@ubc.example>";

fn holding(records: Vec<index::Record>) -> Index {
    let mut index = Index::memory();
    for r in records {
        index.records.insert(r.key.clone(), r);
    }
    index
}

fn one<'a>(all: &'a [Contact], address: &str) -> &'a Contact {
    all.iter()
        .find(|c| c.address == address)
        .unwrap_or_else(|| panic!("{address} in {all:?}"))
}

#[test]
fn each_person_says_what_went_to_them_and_came_from_them() {
    let index = holding(vec![
        record(
            "1",
            "Ada Lovelace <ada@x.example>",
            "me@ubc.example",
            "",
            OLD,
            false,
            "",
        ),
        record(
            "2",
            "Ada L. <ada@x.example>",
            "me@ubc.example",
            "",
            NEW,
            false,
            "",
        ),
        record("3", ME, "ada@x.example", "grace@y.example", OLD, true, ""),
    ]);
    let all = index.contacts(None);
    let ada = one(&all, "ada@x.example");
    assert_eq!((ada.sent, ada.received), (1, 2));
    assert_eq!(ada.name, "Ada L.", "as they last signed");
    assert!(ada.last.unwrap() > 1_700_000_000);
    let grace = one(&all, "grace@y.example");
    assert_eq!((grace.sent, grace.received), (1, 0), "copied counts");
    assert_eq!(grace.name, "", "she never wrote");
}

#[test]
fn the_user_lists_and_those_who_take_no_answer_are_left_out() {
    let index = holding(vec![
        record("1", ME, "ada@x.example", "", OLD, true, ""),
        record("2", ME, "me@ubc.example", "", OLD, true, ""),
        record(
            "3",
            "Shop <offers@shop.example>",
            "me@ubc.example",
            "",
            NEW,
            false,
            "List-Unsubscribe: <mailto:x@shop.example>\r\n",
        ),
        record(
            "4",
            "Group <group@list.example>",
            "me@ubc.example",
            "",
            NEW,
            false,
            "List-Id: <group.list.example>\r\n",
        ),
        record(
            "5",
            "Bot <noreply@bot.example>",
            "me@ubc.example",
            "",
            NEW,
            false,
            "",
        ),
        record("6", ME, "do-not-reply@bot.example", "", NEW, true, ""),
    ]);
    let all = index.contacts(None);
    let addresses: Vec<&str> = all.iter().map(|c| c.address.as_str()).collect();
    assert_eq!(addresses, ["ada@x.example"], "{all:?}");
}

#[test]
fn a_draft_counts_for_no_one() {
    let mut draft = record("1", ME, "ada@x.example", "", NEW, false, "");
    draft.server.push("\\Draft".to_string());
    assert!(holding(vec![draft]).contacts(None).is_empty());
}

#[test]
fn those_written_to_come_first_then_the_latest() {
    let index = holding(vec![
        record(
            "1",
            "New <new@x.example>",
            "me@ubc.example",
            "",
            NEW,
            false,
            "",
        ),
        record("2", ME, "written@x.example", "", OLD, true, ""),
        record(
            "3",
            "Old <old@x.example>",
            "me@ubc.example",
            "",
            OLD,
            false,
            "",
        ),
    ]);
    let addresses: Vec<String> = index
        .contacts(None)
        .into_iter()
        .map(|c| c.address)
        .collect();
    assert_eq!(
        addresses,
        ["written@x.example", "new@x.example", "old@x.example"]
    );
}

#[test]
fn found_by_any_of_name_or_address_whatever_the_case_the_start_of_a_word_first() {
    let index = holding(vec![
        record(
            "1",
            "Bob Madame <bob@x.example>",
            "me@ubc.example",
            "",
            NEW,
            false,
            "",
        ),
        record(
            "2",
            "Ada Lovelace <lovelace@x.example>",
            "me@ubc.example",
            "",
            OLD,
            false,
            "",
        ),
        record(
            "3",
            "Grace Hopper <grace@y.example>",
            "me@ubc.example",
            "",
            OLD,
            false,
            "",
        ),
    ]);
    let found = |words: &str| -> Vec<String> {
        index
            .find_contacts(words, None, 10)
            .into_iter()
            .map(|c| c.address)
            .collect()
    };
    assert_eq!(
        found("ADA"),
        ["lovelace@x.example", "bob@x.example"],
        "Madame holds it too"
    );
    assert_eq!(
        found("ada love"),
        ["lovelace@x.example"],
        "all of the words"
    );
    assert_eq!(found("y.example"), ["grace@y.example"]);
    assert!(found("nobody").is_empty());
    assert_eq!(
        index.find_contacts("example", None, 2).len(),
        2,
        "no more than asked"
    );
}

#[test]
fn another_accounts_mail_is_left_out_when_one_is_asked_for() {
    let mut other = record(
        "1",
        "Ada <ada@x.example>",
        "b@y.example",
        "",
        NEW,
        false,
        "",
    );
    other.key.account = "other@host".to_string();
    let index = holding(vec![
        other,
        record(
            "2",
            "Bob <bob@x.example>",
            "me@u.example",
            "",
            NEW,
            false,
            "",
        ),
    ]);
    assert_eq!(index.contacts(None).len(), 2);
    let mine = index.contacts(Some("me@host"));
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(mine[0].address, "bob@x.example");
}

#[test]
fn someone_is_written_as_they_go_on_a_letter() {
    let c = |name: &str| Contact {
        address: "a@b.example".into(),
        name: name.into(),
        sent: 0,
        received: 0,
        last: None,
    };
    assert_eq!(c("").addressed(), "a@b.example");
    assert_eq!(c("Ada Lovelace").addressed(), "Ada Lovelace <a@b.example>");
    assert_eq!(
        c("du Plessis, Corné").addressed(),
        "\"du Plessis, Corné\" <a@b.example>"
    );
    assert_eq!(
        crate::compose::addr_spec(&c("du Plessis, Corné").addressed()).unwrap(),
        "a@b.example"
    );
    assert_eq!(
        crate::compose::split_addrs(&c("du Plessis, Corné").addressed()).len(),
        1
    );
}

// ---- For an agent ----

#[test]
fn an_agent_finds_someone_by_name_from_the_mail_and_changes_nothing() {
    use serde_json::json;
    let mut server = super::agents::demo_server(false);
    let r = super::agents::call(&mut server, "find_address", json!({ "query": "ada" }));
    assert_eq!(r["isError"], false, "{r}");
    let found = &r["structuredContent"]["contacts"];
    assert_eq!(found[0]["address"], "ada@analytical.example", "{r}");
    assert_eq!(found[0]["name"], "Ada Lovelace", "{r}");
    assert!(found[0]["received"].as_u64().unwrap() > 0, "{r}");
    let none = super::agents::call(
        &mut server,
        "find_address",
        json!({ "query": "nobody here" }),
    );
    assert_eq!(none["structuredContent"]["contacts"], json!([]), "{none}");
    let empty = super::agents::call(&mut server, "find_address", json!({}));
    assert_eq!(empty["isError"], true, "{empty}");
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "find_address")
        .unwrap()
        .clone();
    assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
}

#[test]
fn an_agent_is_told_to_find_who_to_write_to_not_to_guess() {
    use serde_json::json;
    let mut server = super::agents::demo_server(false);
    let init = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" } }))
        .unwrap();
    let said = init["result"]["instructions"].as_str().unwrap();
    assert!(said.contains("find_address"), "{said}");
}

// ---- As printed ----

#[test]
fn a_person_is_printed_by_name_address_and_how_much_mail() {
    let c = Contact {
        address: "ada@x.example".into(),
        name: "Ada Lovelace".into(),
        sent: 3,
        received: 12,
        last: Some(1_704_067_200),
    };
    let line = crate::commands::contact_line(&c);
    assert!(line.contains("Ada Lovelace"), "{line}");
    assert!(line.contains("ada@x.example"), "{line}");
    assert!(line.contains("3 sent"), "{line}");
    assert!(line.contains("12 received"), "{line}");
    assert!(line.contains("2024") || line.contains("2023"), "{line}");
}

// ---- As you write ----

/// A new message begun, `typed` in who it is to.
fn typing_to(typed: &str) -> App {
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["c", typed]);
    app
}

fn to_line(app: &App) -> String {
    let text = app.compose.as_ref().unwrap().buffer.text();
    text.lines().next().unwrap().to_string()
}

#[test]
fn typing_who_it_is_to_suggests_who_fits_and_tab_takes_the_first() {
    let mut app = typing_to("ada");
    let screen = app::screen(&mut app, 150, 40);
    assert!(
        screen.contains("Ada Lovelace <ada@analytical.example>"),
        "{screen}"
    );
    assert!(screen.contains("tab takes it"), "{screen}");
    press(&mut app, &["<tab>"]);
    assert_eq!(to_line(&app), "To: Ada Lovelace <ada@analytical.example>, ");
    let screen = app::screen(&mut app, 150, 40);
    assert!(!screen.contains("tab takes it"), "{screen}");
    // With none showing, tab goes on to the next field, as ever.
    press(&mut app, &["<tab>", "grace@example.com"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.contains("Cc: grace@example.com"), "{text}");
}

#[test]
fn it_goes_on_to_a_second_person_after_a_comma_and_in_cc() {
    let mut app = typing_to("ada");
    press(&mut app, &["<tab>", "gra"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("Grace Hopper"), "{screen}");
    press(&mut app, &["<tab>"]);
    let line = to_line(&app);
    assert!(
        line.contains("Ada Lovelace") && line.contains("Grace Hopper"),
        "{line}"
    );
    press(&mut app, &["<tab>", "ada"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("tab takes it"), "in Cc too: {screen}");
}

#[test]
fn nothing_is_suggested_of_too_little_nor_who_is_there_nor_outside_who_it_is_to() {
    let mut app = typing_to("a");
    let screen = app::screen(&mut app, 150, 40);
    assert!(!screen.contains("tab takes it"), "one letter: {screen}");
    let mut app = typing_to("ada@analytical.example");
    let screen = app::screen(&mut app, 150, 40);
    assert!(!screen.contains("tab takes it"), "there already: {screen}");
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["c", "<tab>", "<tab>", "ada"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(!screen.contains("tab takes it"), "the subject: {screen}");
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["c", "<tab>", "<tab>", "<tab>", "ada"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(
        !screen.contains("tab takes it"),
        "what is written: {screen}"
    );
}

#[test]
fn esc_lets_the_suggestion_go_and_not_the_message() {
    for key in ["<esc>", "<c-g>"] {
        let mut app = typing_to("ada");
        press(&mut app, &[key]);
        assert!(app.compose.is_some(), "{key}: {}", app.status);
        assert_eq!(to_line(&app), "To: ada", "{key}");
        let screen = app::screen(&mut app, 150, 40);
        assert!(!screen.contains("tab takes it"), "{key}: {screen}");
        // Tab goes on now, what was typed left as it is.
        press(&mut app, &["<tab>", "x"]);
        assert_eq!(to_line(&app), "To: ada", "{key}");
    }
}

#[test]
fn with_nothing_known_writing_goes_on_as_ever() {
    let mut app = demo();
    app.index = Arc::new(Mutex::new(Index::memory()));
    press(&mut app, &["c", "ada", "<tab>", "x"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(text.starts_with("To: ada\nCc: x"), "{text}");
}

// ---- Looked through ----

#[test]
fn capital_w_lists_everyone_typing_narrows_it_and_enter_begins_a_letter_to_them() {
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["W"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("People"), "{screen}");
    assert!(screen.contains("ada@analytical.example"), "{screen}");
    assert!(screen.contains("received"), "{screen}");
    press(&mut app, &["hopper"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("Grace Hopper"), "{screen}");
    assert!(!screen.contains("ada@analytical.example"), "{screen}");
    press(&mut app, &["<enter>"]);
    assert!(app.compose.is_some(), "{}", app.status);
    let line = to_line(&app);
    assert!(line.starts_with("To: Grace Hopper <"), "{line}");
    let at = app.compose.as_ref().unwrap().buffer.row();
    assert_eq!(at, 2, "on to its subject");
}

#[test]
fn arrows_pick_who_and_esc_leaves_it_writing_to_no_one() {
    let mut app = super::mapscreen::demo_map();
    press(&mut app, &["W", "<down>", "<up>", "<down>"]);
    press(&mut app, &["<esc>"]);
    assert!(app.compose.is_none());
    assert!(matches!(app.dialog, app::Dialog::Closed));
    press(&mut app, &["W", "<down>", "<enter>"]);
    assert!(to_line(&app).contains('@'), "{}", to_line(&app));
}

#[test]
fn no_one_known_says_why() {
    let mut app = demo();
    app.index = Arc::new(Mutex::new(Index::memory()));
    press(&mut app, &["W"]);
    let screen = app::screen(&mut app, 150, 40);
    assert!(screen.contains("No one yet"), "{screen}");
    press(&mut app, &["<enter>"]);
    assert!(app.compose.is_none());
}
