//! An agent searches every folder of an account at once.

use super::agents::{call, demo_server, said, served, tool};
use serde_json::{Value, json};

#[test]
fn an_agent_searches_every_folder_of_an_account_at_once() {
    let mut server = demo_server(true);
    let inbox = call(&mut server, "list_messages", json!({}));
    let first = inbox["structuredContent"]["messages"][0].clone();
    let uid = first["uid"].as_u64().unwrap();
    let subject = first["subject"].as_str().unwrap().to_string();
    // Out of the Inbox, so only a search of every folder finds it.
    served(
        &mut server,
        vec![
            tool(10, "trash_messages", json!({ "uids": [uid] })),
            said("letterbox-1", "accept"),
        ],
    );
    let in_inbox = call(&mut server, "search_messages", json!({ "query": subject }));
    let subjects = |found: &Value| -> Vec<(String, String)> {
        found["structuredContent"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["folder"].as_str().unwrap_or_default().to_string(),
                    m["subject"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    assert!(
        !subjects(&in_inbox).iter().any(|(_, s)| *s == subject),
        "{in_inbox}"
    );
    let everywhere = call(
        &mut server,
        "search_messages",
        json!({ "query": subject, "folder": "*" }),
    );
    let found = subjects(&everywhere);
    assert!(
        found.iter().any(|(f, s)| f == "Trash" && *s == subject),
        "each found with its folder: {everywhere}"
    );
    let searched = everywhere["structuredContent"]["searched"]
        .as_u64()
        .unwrap();
    assert!(searched > 1, "{everywhere}");
    // At most the limit, of all the folders together.
    let one = call(
        &mut server,
        "search_messages",
        json!({ "query": "e", "folder": "*", "limit": 1 }),
    );
    assert_eq!(one["structuredContent"]["found"], 1, "{one}");
}
