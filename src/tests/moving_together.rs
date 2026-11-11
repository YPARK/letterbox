//! An agent moves mail of several folders, or several accounts, at once,
//! once the user said yes to all of it.

use super::agents::{asked, call, demo_server, paths, reply, said, served, tool, uids};
use serde_json::{Value, json};

#[test]
fn mail_of_several_folders_is_gathered_under_a_new_folder_with_one_yes() {
    let mut server = demo_server(true);
    let inbox = uids(&mut server, "INBOX");
    // One of a thread already filed elsewhere, as Archive.
    served(
        &mut server,
        vec![
            tool(
                9,
                "file_messages",
                json!({ "uids": [inbox[0]], "to": "Archive" }),
            ),
            said("letterbox-1", "accept"),
        ],
    );
    let archived = uids(&mut server, "Archive");
    let written = served(
        &mut server,
        vec![
            tool(
                10,
                "move_messages",
                json!({ "create": true, "moves": [
                { "folder": "INBOX", "uids": [inbox[1], inbox[2]], "action": "file", "to": "Book One" },
                { "folder": "Archive", "uids": [archived[0]], "action": "file", "to": "Book One" }
            ] }),
            ),
            said("letterbox-2", "accept"),
        ],
    );
    let questions = asked(&written);
    assert_eq!(questions.len(), 1, "once for all: {written:?}");
    assert!(
        questions[0].contains("INBOX") && questions[0].contains("Archive"),
        "{}",
        questions[0]
    );
    assert!(questions[0].contains("new folder"), "{}", questions[0]);
    let done = reply(&written, 10)["structuredContent"].clone();
    assert_eq!(done["moved"], 3, "{done}");
    assert_eq!(done["made"], json!(["Book One"]), "made once: {done}");
    assert!(paths(&mut server).iter().any(|p| p == "Book One"));
    let left = uids(&mut server, "INBOX");
    assert!(!left.contains(&inbox[1]) && !left.contains(&inbox[2]));
    assert!(left.contains(&inbox[3]), "nothing else moved");
}

#[test]
fn nothing_of_several_moves_is_done_unless_the_user_says_yes_to_all() {
    let mut server = demo_server(true);
    let inbox = uids(&mut server, "INBOX");
    let written = served(
        &mut server,
        vec![
            tool(
                10,
                "move_messages",
                json!({ "moves": [
                    { "uids": [inbox[0]], "action": "trash" },
                    { "uids": [inbox[1]], "action": "file", "to": "Archive" }
                ] }),
            ),
            said("letterbox-1", "decline"),
            // A move that cannot be done is said before the user is asked.
            tool(
                11,
                "move_messages",
                json!({ "moves": [
                    { "uids": [inbox[0]], "action": "trash" },
                    { "uids": [inbox[1]], "action": "file", "to": "Made up" }
                ] }),
            ),
            tool(12, "move_messages", json!({ "moves": [] })),
            // One message, two ways: never asked of.
            tool(
                13,
                "move_messages",
                json!({ "moves": [
                    { "uids": [inbox[0]], "action": "trash" },
                    { "uids": [inbox[0]], "action": "file", "to": "Archive" }
                ] }),
            ),
        ],
    );
    assert_eq!(asked(&written).len(), 1, "{written:?}");
    assert_eq!(reply(&written, 10)["structuredContent"]["moved"], 0);
    assert_eq!(reply(&written, 11)["isError"], true);
    assert_eq!(reply(&written, 12)["isError"], true);
    assert_eq!(reply(&written, 13)["isError"], true);
    assert_eq!(uids(&mut server, "INBOX"), inbox, "nothing moved");
}

#[test]
fn several_moves_are_made_by_no_agent_that_cannot_ask_or_that_letterbox_started() {
    for asks in [false, true] {
        let mut server = demo_server(asks);
        server.background = asks;
        let inbox = uids(&mut server, "INBOX");
        let r = call(
            &mut server,
            "move_messages",
            json!({ "moves": [{ "uids": [inbox[0]], "action": "trash" }] }),
        );
        assert_eq!(r["isError"], true, "{r}");
        assert_eq!(uids(&mut server, "INBOX"), inbox);
    }
}

#[test]
fn one_new_folder_named_twice_but_for_case_is_made_once_and_filed_under() {
    let mut server = demo_server(true);
    let inbox = uids(&mut server, "INBOX");
    let written = served(
        &mut server,
        vec![
            tool(
                10,
                "move_messages",
                json!({ "create": true, "moves": [
                    { "uids": [inbox[0]], "action": "file", "to": "Book One" },
                    { "uids": [inbox[1]], "action": "file", "to": "book one" }
                ] }),
            ),
            said("letterbox-1", "accept"),
        ],
    );
    let done = reply(&written, 10)["structuredContent"].clone();
    assert_eq!(done["moved"], 2, "{done}");
    assert_eq!(done["made"], json!(["Book One"]), "{done}");
    assert_eq!(done["moves"][1]["to"], "Book One", "as it was made: {done}");
    assert_eq!(done["moves"][1]["made"], Value::Null, "{done}");
    let made: Vec<String> = paths(&mut server)
        .into_iter()
        .filter(|p| p.eq_ignore_ascii_case("book one"))
        .collect();
    assert_eq!(made, ["Book One"]);
}

#[test]
fn what_the_user_is_asked_says_each_account_and_names_as_they_read() {
    let mut server = demo_server(true);
    let inbox = uids(&mut server, "INBOX");
    let written = served(
        &mut server,
        vec![
            tool(
                10,
                "move_messages",
                json!({ "create": true, "moves": [
                    { "account": "Demo", "uids": [inbox[0]], "action": "file", "to": "Café" }
                ] }),
            ),
            said("letterbox-1", "decline"),
            // A folder there is already, but for its case: said so.
            tool(
                11,
                "file_messages",
                json!({ "uids": [inbox[0]], "to": "TRASH", "create": true }),
            ),
        ],
    );
    let question = &asked(&written)[0];
    assert!(question.contains("Demo"), "which account: {question}");
    assert!(question.contains("Café"), "as it reads: {question}");
    let refused = reply(&written, 11);
    assert_eq!(refused["isError"], true);
    let text = refused["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("already"), "{text}");
}
