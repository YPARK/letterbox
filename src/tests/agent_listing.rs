//! What an agent is told of the accounts it reads: which, and of what
//! version of letterbox, as much as it asks for.

use super::agents::{call, demo_server};
use serde_json::json;

#[test]
fn list_accounts_says_which_letterbox_answers() {
    let mut server = demo_server(false);
    let r = call(&mut server, "list_accounts", json!({}));
    assert_eq!(
        r["structuredContent"]["letterbox"],
        env!("CARGO_PKG_VERSION"),
        "{r}"
    );
}

#[test]
fn list_accounts_may_give_their_names_alone_or_one_account() {
    let mut server = demo_server(false);
    let whole = call(&mut server, "list_accounts", json!({}));
    let name = whole["structuredContent"]["accounts"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();

    let names = call(&mut server, "list_accounts", json!({ "names_only": true }));
    assert_eq!(
        names["structuredContent"]["accounts"],
        json!([{ "name": name }]),
        "no folders: {names}"
    );

    let one = call(&mut server, "list_accounts", json!({ "account": name }));
    let accounts = one["structuredContent"]["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1, "{one}");
    assert!(accounts[0]["folders"].is_array(), "{one}");

    let none = call(&mut server, "list_accounts", json!({ "account": "Nobody" }));
    assert_eq!(none["isError"], true, "not an empty list: {none}");
    assert!(
        none.to_string().contains(&name),
        "says what there is: {none}"
    );
}

#[test]
fn what_is_listed_found_or_read_says_of_which_account() {
    let mut server = demo_server(false);
    let whole = call(&mut server, "list_accounts", json!({}));
    let name = whole["structuredContent"]["accounts"][0]["name"].clone();
    let listed = call(&mut server, "list_messages", json!({}));
    assert_eq!(listed["structuredContent"]["account"], name, "{listed}");
    let uid = listed["structuredContent"]["messages"][0]["uid"].clone();
    let found = call(&mut server, "search_messages", json!({ "query": "the" }));
    assert_eq!(found["structuredContent"]["account"], name, "{found}");
    let read = call(&mut server, "read_message", json!({ "uid": uid }));
    assert_eq!(read["structuredContent"]["account"], name, "{read}");
    // As it is called, however it was asked for.
    let lower = name.as_str().unwrap().to_lowercase();
    let listed = call(&mut server, "list_messages", json!({ "account": lower }));
    assert_eq!(listed["structuredContent"]["account"], name, "{listed}");
}
