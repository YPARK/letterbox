//! An agent makes a folder, or files mail under one it makes, only once the
//! user said yes to it.

use super::agents::{asked, call, demo_server, paths, reply, said, served, tool};
use serde_json::{Value, json};

#[test]
fn an_agent_makes_a_folder_only_once_the_user_said_yes_to_it() {
    let mut server = demo_server(true);
    let before = paths(&mut server);
    let existing = before
        .iter()
        .find(|p| p.as_str() != "INBOX")
        .unwrap()
        .clone();
    let written = served(
        &mut server,
        vec![
            tool(10, "create_folder", json!({ "path": "Book One" })),
            said("letterbox-1", "accept"),
            tool(11, "create_folder", json!({ "path": "Book Two" })),
            said("letterbox-2", "decline"),
            // There already, as it is or but for its case: never asked.
            tool(12, "create_folder", json!({ "path": "Book One" })),
            tool(
                13,
                "create_folder",
                json!({ "path": existing.to_uppercase() }),
            ),
            tool(14, "create_folder", json!({ "path": "  " })),
            // Named beyond ASCII: kept as the server keeps such names, and
            // known again whatever its case.
            tool(15, "create_folder", json!({ "path": "Reisen Übersicht" })),
            said("letterbox-3", "accept"),
            tool(16, "create_folder", json!({ "path": "reisen übersicht" })),
        ],
    );
    let asked = asked(&written);
    assert_eq!(asked.len(), 3, "{written:?}");
    let shown = &asked[0];
    assert!(shown.contains("Book One"), "{shown}");
    assert_eq!(reply(&written, 10)["structuredContent"]["made"], "Book One");
    assert_eq!(
        reply(&written, 11)["structuredContent"]["made"],
        Value::Null
    );
    for id in [12, 13, 14, 16] {
        assert_eq!(reply(&written, id)["isError"], true, "{id}: {written:?}");
    }
    assert_eq!(
        reply(&written, 15)["structuredContent"]["made"],
        "Reisen &ANw-bersicht"
    );
    let after = paths(&mut server);
    assert!(after.iter().any(|p| p == "Book One"), "{after:?}");
    assert!(!after.iter().any(|p| p == "Book Two"), "the user said no");
    assert_eq!(after.len(), before.len() + 2);
}

#[test]
fn an_agent_files_mail_under_a_folder_it_makes_with_one_yes() {
    let mut server = demo_server(true);
    let inbox = call(&mut server, "list_messages", json!({}));
    let uid = inbox["structuredContent"]["messages"][0]["uid"]
        .as_u64()
        .unwrap();
    let written = served(
        &mut server,
        vec![
            // Without create, a folder there is not is never made up.
            tool(
                10,
                "file_messages",
                json!({ "uids": [uid], "to": "Book One" }),
            ),
            tool(
                11,
                "file_messages",
                json!({ "uids": [uid], "to": "Book One", "create": true }),
            ),
            said("letterbox-1", "accept"),
        ],
    );
    assert_eq!(reply(&written, 10)["isError"], true, "{written:?}");
    let asked = asked(&written);
    assert_eq!(asked.len(), 1, "one question, for both: {written:?}");
    let shown = &asked[0];
    assert!(shown.contains("new folder"), "{shown}");
    let filed = reply(&written, 11)["structuredContent"].clone();
    assert_eq!(filed["moved"], 1, "{filed}");
    assert_eq!(filed["made"], "Book One", "{filed}");
    // Made, and filed there as the store files: moved, or labelled.
    assert!(paths(&mut server).iter().any(|p| p == "Book One"));
    let left = call(&mut server, "list_messages", json!({}));
    assert!(
        !left["structuredContent"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["uid"] == uid),
        "{left}"
    );
}

#[test]
fn a_folder_is_made_by_no_agent_that_cannot_ask_or_that_letterbox_started() {
    let mut server = demo_server(false);
    let r = call(&mut server, "create_folder", json!({ "path": "Book One" }));
    assert_eq!(r["isError"], true, "{r}");
    let mut started = demo_server(true);
    started.background = true;
    let r = call(&mut started, "create_folder", json!({ "path": "Book One" }));
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("letterbox started"),
        "{r}"
    );
    for s in [&mut server, &mut started] {
        assert!(!paths(s).iter().any(|p| p == "Book One"));
    }
}
