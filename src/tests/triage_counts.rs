//! Triage says what else is in each Inbox, older than the hours it looked
//! back over, so a summary of new mail never takes an Inbox to be empty.

use super::agents::{call, demo_server};
use serde_json::json;

#[test]
fn triage_says_how_much_older_mail_is_still_in_each_inbox() {
    let mut server = demo_server(true);
    let listed = call(&mut server, "list_messages", json!({}));
    let total = listed["structuredContent"]["total"].as_u64().unwrap();
    assert!(total > 0);
    for tool in ["triage", "recent_messages"] {
        let r = call(&mut server, tool, json!({ "hours": 1 }));
        let account = &r["structuredContent"]["accounts"][0];
        let new = account["messages"].as_array().unwrap().len() as u64;
        assert_eq!(account["inbox_total"], total, "{tool}: {account}");
        assert!(account["inbox_unread"].is_u64(), "{tool}: {account}");
        assert_eq!(account["older_in_inbox"], total - new, "{tool}: {account}");
    }
    // Only unread mail looked at: how much older is not to be told from it.
    let r = call(
        &mut server,
        "triage",
        json!({ "hours": 1, "unread_only": true }),
    );
    let account = &r["structuredContent"]["accounts"][0];
    assert_eq!(account["inbox_total"], total, "{account}");
    assert!(account["older_in_inbox"].is_null(), "{account}");
}
