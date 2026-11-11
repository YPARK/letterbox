//! Folders of nearly one name side by side are pointed out, and one is
//! merged into the other once the user said yes to it.

use super::agents::{asked, call, demo_server, paths, reply, said, served, tool, total};
use serde_json::json;

#[test]
fn folders_of_nearly_one_name_side_by_side_are_pointed_out_and_merged() {
    let mut server = demo_server(true);
    served(
        &mut server,
        vec![
            tool(8, "create_folder", json!({ "path": "Grant" })),
            said("letterbox-1", "accept"),
            tool(9, "create_folder", json!({ "path": "Projects/Grants" })),
            said("letterbox-2", "accept"),
            // Kept for Trash by its role, and a word longer by more than an
            // ending: neither paired.
            tool(6, "create_folder", json!({ "path": "Trash Old" })),
            said("letterbox-3", "accept"),
            tool(7, "create_folder", json!({ "path": "Projectsmanagement" })),
            said("letterbox-4", "accept"),
        ],
    );
    let listed = call(&mut server, "list_accounts", json!({}));
    let alike = listed["structuredContent"]["accounts"][0]["look_alike"].clone();
    assert_eq!(
        alike,
        json!([["Projects/Grant", "Projects/Grants"]]),
        "side by side only, never of two parents: {listed}"
    );
    let n = total(&mut server, "Projects/Grant");
    assert!(n > 0);
    let written = served(
        &mut server,
        vec![
            tool(
                10,
                "merge_folders",
                json!({ "from": "Projects/Grant", "into": "Projects/Grants" }),
            ),
            said("letterbox-5", "decline"),
            tool(
                11,
                "merge_folders",
                json!({ "from": "Projects/Grant", "into": "Projects/Grants" }),
            ),
            said("letterbox-6", "accept"),
            // Never of a folder kept for mail as it comes, goes or is thrown away.
            tool(
                12,
                "merge_folders",
                json!({ "from": "INBOX", "into": "Archive" }),
            ),
            tool(
                13,
                "merge_folders",
                json!({ "from": "Projects/Grants", "into": "Trash" }),
            ),
            tool(
                14,
                "merge_folders",
                json!({ "from": "Archive", "into": "Archive" }),
            ),
            // A folder with one under it is never emptied to be left.
            tool(
                16,
                "merge_folders",
                json!({ "from": "Projects", "into": "Projects/Grants" }),
            ),
            // Archive is kept for its role: mail goes into it, never out of it.
            tool(
                15,
                "merge_folders",
                json!({ "from": "Archive", "into": "Projects/Grants" }),
            ),
        ],
    );
    let asked = asked(&written);
    assert_eq!(asked.len(), 2, "{written:?}");
    let shown = &asked[0];
    assert!(shown.contains(&n.to_string()), "how many said: {shown}");
    assert_eq!(reply(&written, 10)["structuredContent"]["moved"], 0);
    let merged = reply(&written, 11)["structuredContent"].clone();
    assert_eq!(merged["moved"], n, "{merged}");
    assert_eq!(merged["removed"], true, "{merged}");
    for id in [12, 13, 14, 15, 16] {
        assert_eq!(reply(&written, id)["isError"], true, "{id}: {written:?}");
    }
    assert!(!paths(&mut server).iter().any(|p| p == "Projects/Grant"));
    assert_eq!(total(&mut server, "Projects/Grants"), n);
    let listed = call(&mut server, "list_accounts", json!({}));
    assert_eq!(
        listed["structuredContent"]["accounts"][0]["look_alike"],
        json!([]),
        "{listed}"
    );
}

#[test]
fn folders_are_merged_by_no_agent_that_cannot_ask_or_that_letterbox_started() {
    for asks in [false, true] {
        let mut server = demo_server(asks);
        server.background = asks;
        let n = total(&mut server, "Projects/Grant");
        let r = call(
            &mut server,
            "merge_folders",
            json!({ "from": "Projects/Grant", "into": "Archive" }),
        );
        assert_eq!(r["isError"], true, "{r}");
        assert_eq!(total(&mut server, "Projects/Grant"), n);
    }
}

#[test]
fn imap_moves_a_whole_folder_in_one_command_mail_without_a_message_id_too() {
    use crate::store::Store;
    let (account, _) = super::imap_server::server();
    let mut imap = crate::imap::Imap::new(account);
    imap.create("Old").unwrap();
    imap.create("New").unwrap();
    let page = imap.page("INBOX", 0, 10).unwrap();
    for s in page.summaries.iter().take(2) {
        imap.relocate("INBOX", s.uid, "Old", &s.message_id).unwrap();
    }
    imap.append(
        "Old",
        b"From: me@example.com\r\nSubject: no id\r\n\r\nhi\r\n",
        true,
    )
    .unwrap();
    assert_eq!(imap.counts("Old").unwrap().0, 3);
    assert_eq!(imap.move_every("Old", "New").unwrap(), 3);
    assert_eq!(imap.counts("Old").unwrap().0, 0);
    assert_eq!(imap.counts("New").unwrap().0, 3);
    imap.delete_folder("Old").unwrap();
    // An empty folder moves nothing, and is no error.
    imap.create("Empty").unwrap();
    assert_eq!(imap.move_every("Empty", "New").unwrap(), 0);
}

#[test]
fn folders_a_server_keeps_of_itself_are_never_filed_in_nor_merged() {
    use crate::store::Store;
    let (account, _) = super::serve_boxes(super::gmail_boxes());
    let folders = crate::imap::Imap::new(account).folders().unwrap();
    for path in ["[Gmail]/Starred", "[Gmail]/Important"] {
        let f = folders.iter().find(|f| f.path == path).unwrap();
        assert!(f.kept && !f.is_filed(), "{path}: {f:?}");
    }
    let grant = folders.iter().find(|f| f.path == "Projects/Grant").unwrap();
    assert!(grant.is_filed() && !grant.kept);
}

#[test]
fn names_of_any_script_are_compared_without_a_fault() {
    let folder = |path: &str| crate::mail::Folder::new(path, Some("/"), None, true);
    // Fewer letters, more bytes: never taken the other way round.
    let folders = [folder("éééé"), folder("abcdef"), folder("Ééééé")];
    assert_eq!(
        crate::mail::look_alike(&folders),
        [["éééé".to_string(), "Ééééé".to_string()]]
    );
}

#[test]
fn a_folder_the_server_keeps_of_itself_is_no_label_in_the_index() {
    use crate::index::labels::of_folder;
    use crate::index::record::FolderState;
    let filed = FolderState {
        role: Some(crate::mail::Role::Other),
        delimiter: Some("/".to_string()),
        ..FolderState::default()
    };
    assert_eq!(
        of_folder("Projects/Grant", Some(&filed)),
        ["Projects", "Grant"]
    );
    let kept = FolderState {
        kept: true,
        ..filed
    };
    assert!(of_folder("Flagged", Some(&kept)).is_empty());
}
