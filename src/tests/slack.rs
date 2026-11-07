//! Slack, as letterbox reads it with the token of your own app, from a server
//! in place of Slack's.

use crate::secret::Keeper;
use crate::slack::{self, Kind, Session, Slack, Workspace};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// A server giving `answers` in turn, each whole, status line and all; the
/// requests it took, each its head and its body.
fn server(answers: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let taken = asked.clone();
    std::thread::spawn(move || {
        for answer in answers {
            let (mut tcp, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(tcp.try_clone().unwrap());
            let (mut request, mut length) = (String::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            request.push_str(&String::from_utf8(body).unwrap());
            taken.lock().unwrap().push(request);
            tcp.write_all(answer.as_bytes()).unwrap();
        }
    });
    (base, asked)
}

fn ok(body: serde_json::Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn session() -> Session {
    Session {
        domain: "acme".into(),
        token: "xoxp-secret".into(),
    }
}

#[test]
fn slack_is_asked_with_the_token_and_says_who_and_where() {
    let (base, asked) = server(vec![ok(json!({
        "ok": true, "user": "ypp", "team": "Acme", "url": "https://acme-lab.slack.com/"
    }))]);
    let who = Slack::at(&base, session()).whoami().unwrap();
    assert_eq!(
        (who.user.as_str(), who.team.as_str(), who.domain.as_str()),
        ("ypp", "Acme", "acme-lab")
    );
    let asked = asked.lock().unwrap();
    assert!(asked[0].starts_with("POST /auth.test "), "{}", asked[0]);
    assert!(asked[0].contains("token=xoxp-secret"), "{}", asked[0]);
}

#[test]
fn only_a_user_token_is_taken() {
    assert!(slack::user_token("  xoxp-1-2-3\n").is_ok());
    for bad in ["", "xoxb-1", "xapp-1", "xoxc-1", "hello"] {
        let said = format!("{:#}", slack::user_token(bad).unwrap_err());
        assert!(said.contains("xoxp-"), "{said}");
        assert!(bad.is_empty() || !said.contains(bad), "{said}");
    }
}

#[test]
fn channels_are_read_to_the_last_page() {
    let (base, asked) = server(vec![
        ok(json!({"ok": true,
            "channels": [
                {"id": "C1", "name": "general", "is_channel": true},
                {"id": "G1", "name": "lab", "is_group": true, "is_private": true},
            ],
            "response_metadata": {"next_cursor": "more"}})),
        ok(json!({"ok": true,
            "channels": [
                {"id": "D1", "is_im": true, "user": "U9"},
                {"id": "M1", "name": "mpdm-a--b-1", "is_mpim": true},
            ],
            "response_metadata": {"next_cursor": ""}})),
    ]);
    let channels = Slack::at(&base, session()).channels().unwrap();
    let got: Vec<_> = channels
        .iter()
        .map(|c| (c.id.as_str(), c.name.as_str(), c.kind))
        .collect();
    assert_eq!(
        got,
        [
            ("C1", "general", Kind::Public),
            ("G1", "lab", Kind::Private),
            ("D1", "U9", Kind::Direct),
            ("M1", "mpdm-a--b-1", Kind::Group),
        ]
    );
    assert!(asked.lock().unwrap()[1].contains("cursor=more"));
}

#[test]
fn what_slack_refuses_is_said_and_the_token_is_not() {
    let (base, _) = server(vec![ok(json!({"ok": false, "error": "invalid_auth"}))]);
    let said = format!("{:#}", Slack::at(&base, session()).whoami().unwrap_err());
    assert!(said.contains("invalid_auth"), "{said}");
    assert!(said.contains("letterbox slack add"), "{said}");
    assert!(!said.contains("xoxp-secret"), "{said}");
}

#[test]
fn slack_is_waited_for_when_it_says_to_slow_down() {
    let (base, asked) = server(vec![
        "HTTP/1.1 429 Too Many Requests\r\nretry-after: 0\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            .into(),
        ok(json!({"ok": true, "user": "y", "team": "A"})),
    ]);
    let who = Slack::at(&base, session()).whoami().unwrap();
    assert_eq!(who.user, "y");
    assert_eq!(asked.lock().unwrap().len(), 2);
}

#[test]
fn a_session_never_prints_its_token() {
    let shown = format!("{:?}", session());
    assert!(shown.contains("acme"));
    assert!(!shown.contains("xoxp-secret"));
}

#[test]
fn workspaces_are_remembered_with_no_secret_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = dir.path().join("accounts.toml");
    assert!(slack::workspaces(&accounts).unwrap().is_empty());
    let acme = Workspace {
        domain: "acme".into(),
        keep: Keeper::Vault,
    };
    slack::remember(&accounts, &acme).unwrap();
    slack::remember(&accounts, &acme).unwrap();
    assert_eq!(slack::workspaces(&accounts).unwrap(), [acme]);
    let file = std::fs::read_to_string(dir.path().join("slack.toml")).unwrap();
    assert!(!file.contains("xox"));
    slack::forget_workspace(&accounts, "acme").unwrap();
    assert!(slack::workspaces(&accounts).unwrap().is_empty());
}

fn people() -> std::collections::HashMap<String, String> {
    [("U1".to_string(), "Ana".to_string())].into()
}

#[test]
fn people_go_by_the_name_they_show() {
    let (base, asked) = server(vec![
        ok(json!({"ok": true,
            "members": [
                {"id": "U1", "name": "ana.k", "profile": {"display_name": "Ana", "real_name": "Ana K"}},
                {"id": "U2", "name": "bo", "profile": {"display_name": "", "real_name": "Bo L"}},
            ],
            "response_metadata": {"next_cursor": "more"}})),
        ok(json!({"ok": true,
            "members": [{"id": "U3", "name": "cy", "profile": {}}],
            "response_metadata": {"next_cursor": ""}})),
    ]);
    let people = Slack::at(&base, session()).people().unwrap();
    assert_eq!(people["U1"], "Ana");
    assert_eq!(people["U2"], "Bo L");
    assert_eq!(people["U3"], "cy");
    assert!(asked.lock().unwrap()[1].contains("cursor=more"));
}

#[test]
fn a_direct_message_goes_by_the_persons_name() {
    let channels = vec![
        slack::Channel {
            id: "D1".into(),
            name: "U1".into(),
            kind: Kind::Direct,
            updated: 0,
        },
        slack::Channel {
            id: "D2".into(),
            name: "U404".into(),
            kind: Kind::Direct,
            updated: 0,
        },
        slack::Channel {
            id: "C1".into(),
            name: "U1".into(),
            kind: Kind::Public,
            updated: 0,
        },
    ];
    let named = slack::reading::named(channels, &people());
    let names: Vec<_> = named.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Ana", "U404", "U1"]);
}

#[test]
fn what_slack_writes_is_read_as_plain_text() {
    let text = "hi <@U1> and <@U9>, see <#C1|lab> and <#C2>, <https://a.org/x|the page> \
                <https://b.org> <!here> <!subteam^S1|@team> 1 &lt; 2 &amp;&amp; 3 &gt; 2 <mailto:a@b.org|a@b.org>";
    assert_eq!(
        slack::reading::plain(text, &people()),
        "hi @Ana and @U9, see #lab and #C2, the page (https://a.org/x) \
         https://b.org @here @team 1 < 2 && 3 > 2 a@b.org"
    );
    assert_eq!(slack::reading::plain("a < b", &people()), "a < b");
}

#[test]
fn a_channels_messages_come_oldest_first_with_who_and_their_threads() {
    let (base, asked) = server(vec![ok(json!({"ok": true, "messages": [
        {"ts": "300.0", "user": "U1", "text": "third <@U1>"},
        {"ts": "200.0", "bot_profile": {"name": "Paging"}, "text": "second"},
        {"ts": "100.0", "user": "U7", "text": "first", "thread_ts": "100.0", "reply_count": 2},
    ]}))]);
    let messages = Slack::at(&base, session())
        .messages("C1", 3, &people())
        .unwrap();
    let got: Vec<_> = messages
        .iter()
        .map(|m| (m.ts.as_str(), m.who.as_str(), m.text.as_str(), m.replies))
        .collect();
    assert_eq!(
        got,
        [
            ("100.0", "U7", "first", 2),
            ("200.0", "Paging", "second", 0),
            ("300.0", "Ana", "third @Ana", 0),
        ]
    );
    let asked = asked.lock().unwrap();
    assert!(asked[0].starts_with("POST /conversations.history "));
    assert!(asked[0].contains("channel=C1") && asked[0].contains("limit=3"));
}

#[test]
fn a_thread_is_read_to_its_end_in_its_order() {
    let (base, asked) = server(vec![
        ok(
            json!({"ok": true, "messages": [{"ts": "1.0", "user": "U1", "text": "start"}],
            "response_metadata": {"next_cursor": "more"}}),
        ),
        ok(
            json!({"ok": true, "messages": [{"ts": "2.0", "user": "U1", "text": "reply"}],
            "response_metadata": {"next_cursor": ""}}),
        ),
    ]);
    let thread = Slack::at(&base, session())
        .thread("C1", "1.0", &people())
        .unwrap();
    let texts: Vec<_> = thread.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["start", "reply"]);
    let asked = asked.lock().unwrap();
    assert!(asked[0].starts_with("POST /conversations.replies "));
    assert!(asked[0].contains("ts=1.0") && asked[1].contains("cursor=more"));
}

#[test]
fn a_conversation_is_found_by_its_name_or_its_id() {
    let channels = slack::reading::named(
        vec![
            slack::Channel {
                id: "C1".into(),
                name: "lab".into(),
                kind: Kind::Public,
                updated: 0,
            },
            slack::Channel {
                id: "D1".into(),
                name: "U1".into(),
                kind: Kind::Direct,
                updated: 0,
            },
        ],
        &people(),
    );
    let id = |asked| slack::reading::find(&channels, asked).map(|c| c.id.clone());
    assert_eq!(id("lab").unwrap(), "C1");
    assert_eq!(id("#lab").unwrap(), "C1");
    assert_eq!(id("@Ana").unwrap(), "D1");
    assert_eq!(id("D1").unwrap(), "D1");
    assert!(id("nobody").is_err());
}

#[test]
fn what_is_written_can_hold_no_terminal_controls() {
    let text = "safe\u{1b}[31m red\u{7}\nnext\tline";
    assert_eq!(
        slack::reading::plain(text, &people()),
        "safe[31m red\nnext\tline"
    );
}

#[test]
fn a_name_is_found_by_any_part_of_it_and_never_guessed() {
    let channel = |id: &str, name: &str, kind| slack::Channel {
        id: id.into(),
        name: name.into(),
        kind,
        updated: 0,
    };
    let channels = vec![
        channel("D1", "Ana K", Kind::Direct),
        channel("D2", "Ana Lee", Kind::Direct),
        channel("D3", "Bo Chen", Kind::Direct),
        channel("C1", "lab", Kind::Public),
        channel("C2", "lab-tools", Kind::Public),
    ];
    let id = |asked| slack::reading::find(&channels, asked).map(|c| c.id.clone());
    assert_eq!(id("@bo").unwrap(), "D3");
    assert_eq!(id("@BO CHEN").unwrap(), "D3");
    assert_eq!(
        id("#LAB").unwrap(),
        "C1",
        "a whole name beats a part of one"
    );
    assert_eq!(id("tools").unwrap(), "C2");
    let said = format!("{:#}", id("@ana").unwrap_err());
    assert!(said.contains("Ana K") && said.contains("Ana Lee"), "{said}");
    assert!(id("#ana").is_err(), "# is not a direct message");
    assert!(id("@").is_err(), "nothing named is nobody");
}

#[test]
fn ones_own_conversations_are_read_with_when_each_was_last_busy() {
    let (base, asked) = server(vec![
        ok(json!({"ok": true,
            "channels": [{"id": "C1", "name": "lab", "is_channel": true, "updated": 1790000000000_i64}],
            "response_metadata": {"next_cursor": "more"}})),
        ok(json!({"ok": true,
            "channels": [{"id": "D1", "is_im": true, "user": "U1", "updated": 1795000000000_i64}],
            "response_metadata": {"next_cursor": ""}})),
    ]);
    let mine = Slack::at(&base, session()).mine().unwrap();
    let got: Vec<_> = mine.iter().map(|c| (c.id.as_str(), c.updated)).collect();
    assert_eq!(got, [("C1", 1790000000000), ("D1", 1795000000000)]);
    let asked = asked.lock().unwrap();
    assert!(
        asked[0].starts_with("POST /users.conversations "),
        "{}",
        asked[0]
    );
    assert!(asked[1].contains("cursor=more"));
}

#[test]
fn a_direct_messages_unread_are_as_slack_counts_them() {
    let (base, asked) = server(vec![ok(json!({"ok": true,
        "channel": {"id": "D1", "is_im": true, "last_read": "5.0", "unread_count_display": 3}}))]);
    let direct = slack::Channel {
        id: "D1".into(),
        name: "Ana".into(),
        kind: Kind::Direct,
        updated: 0,
    };
    assert_eq!(Slack::at(&base, session()).unread(&direct).unwrap(), 3);
    assert_eq!(asked.lock().unwrap().len(), 1, "no messages fetched for it");
}

#[test]
fn a_channels_unread_are_those_since_it_was_last_read() {
    let (base, asked) = server(vec![
        ok(json!({"ok": true, "channel": {"id": "C1", "last_read": "5.000100"}})),
        ok(json!({"ok": true, "messages": [{"ts": "7.0"}, {"ts": "6.0"}]})),
    ]);
    let lab = slack::Channel {
        id: "C1".into(),
        name: "lab".into(),
        kind: Kind::Public,
        updated: 0,
    };
    assert_eq!(Slack::at(&base, session()).unread(&lab).unwrap(), 2);
    let asked = asked.lock().unwrap();
    assert!(asked[0].starts_with("POST /conversations.info "));
    assert!(asked[1].starts_with("POST /conversations.history "));
    assert!(asked[1].contains("oldest=5.000100"), "{}", asked[1]);
}

#[test]
fn emoji_names_are_the_emoji() {
    let plain = |text| slack::reading::plain(text, &people());
    assert_eq!(plain("cool :smiley: :tada:"), "cool 😃 🎉");
    assert_eq!(plain("ok :+1::skin-tone-3:"), "ok 👍");
    // A workspace's own emoji, and what only looks like one, stay as written.
    assert_eq!(
        plain(":party-parrot: at 10:30:45"),
        ":party-parrot: at 10:30:45"
    );
    assert_eq!(plain("a :: b"), "a :: b");
    // Not within a link's address.
    assert_eq!(plain("<https://a.org/:smile:>"), "https://a.org/:smile:");
}
