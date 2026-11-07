//! Slack in the TUI: its conversations in the sidebar, by when each was last
//! busy, with what is unread; one read in the list's place, and a thread in
//! the message's. From a server in place of Slack's.

use super::*;
use crate::slack::{Session, Slack};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

/// A server answering each request by the first of `answers` whose every
/// word is in its first line or its body; the requests it took.
fn slack(answers: Vec<(&'static str, Value)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let taken = asked.clone();
    let answers = Arc::new(answers);
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (answers, taken) = (answers.clone(), taken.clone());
            std::thread::spawn(move || {
                let mut reader = BufReader::new(&tcp);
                let (mut request, mut length) = (String::new(), 0);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap_or(0);
                    }
                    request.push_str(&line);
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                let body = String::from_utf8_lossy(&body).to_string();
                let first = request.lines().next().unwrap_or("").to_string();
                let said = format!("{first} {body}");
                let answer = answers
                    .iter()
                    .find(|(key, _)| key.split(' ').all(|w| said.contains(w)))
                    .map_or(json!({"ok": false, "error": "unknown_method"}), |(_, a)| {
                        a.clone()
                    });
                taken.lock().unwrap().push(said);
                let answer = answer.to_string();
                let _ = write!(
                    &tcp,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                     connection: close\r\n\r\n{answer}",
                    answer.len()
                );
            });
        }
    });
    (base, asked)
}

/// A workspace of one direct message, with Ana, and fourteen channels, each
/// busy longer ago than the one before; of those only the last, `ch13`, has
/// anything unread, four messages.
fn workspace() -> (String, Arc<Mutex<Vec<String>>>) {
    let mut channels = vec![json!({"id": "D1", "is_im": true, "user": "U1", "updated": 2_000_000})];
    for i in 0..14 {
        channels.push(json!({
            "id": format!("C{i:02}"), "name": format!("ch{i:02}"),
            "is_channel": true, "updated": 1_000_000 - i,
        }));
    }
    let message = |ts: &str, user: &str, text: &str| json!({"ts": ts, "user": user, "text": text});
    slack(vec![
        (
            "users.list",
            json!({"ok": true, "members": [
                {"id": "U1", "name": "ana", "profile": {"display_name": "Ana"}},
                {"id": "U2", "name": "bo", "profile": {"display_name": "Bo"}},
            ]}),
        ),
        (
            "users.conversations",
            json!({"ok": true, "channels": channels}),
        ),
        (
            "conversations.info channel=D1",
            json!({"ok": true, "channel": {"id": "D1", "is_im": true, "unread_count_display": 0}}),
        ),
        (
            "conversations.info channel=C13",
            json!({"ok": true, "channel": {"id": "C13", "last_read": "1.0"}}),
        ),
        (
            "conversations.info",
            json!({"ok": true, "channel": {"last_read": "9.0"}}),
        ),
        (
            "conversations.history oldest=1.0",
            json!({"ok": true, "messages": [{"ts": "5.0"}, {"ts": "4.0"}, {"ts": "3.0"}, {"ts": "2.0"}]}),
        ),
        (
            "conversations.history oldest=",
            json!({"ok": true, "messages": []}),
        ),
        (
            "conversations.replies",
            json!({"ok": true, "messages": [
                message("1700000100.0", "U1", "hello <@U2>"),
                message("1700000200.0", "U2", "a reply from Bo"),
            ]}),
        ),
        (
            "conversations.history",
            json!({"ok": true, "messages": [
                message("1700000300.0", "U2", "newest note"),
                {"ts": "1700000100.0", "user": "U1", "text": "hello <@U2>",
                 "thread_ts": "1700000100.0", "reply_count": 1},
            ]}),
        ),
    ])
}

fn app_on(base: &str) -> App {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(Vec::new(), dir.path().join("accounts.toml"));
    let session = Session {
        domain: "acme".into(),
        token: "xoxp-test".into(),
    };
    app.chat_add("acme".into(), Slack::at(base, session));
    app.settle_chat(PATIENCE);
    app
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
    app.settle_chat(PATIENCE);
}

/// The conversations in the sidebar, by name, and a row for those folded.
fn talks(app: &App) -> Vec<String> {
    app.rows
        .iter()
        .filter_map(|r| match *r {
            app::Row::SlackHead(_) => Some("SLACK".to_string()),
            app::Row::Talk(s, i) => Some(app.chat.spaces[s].talks[i].channel.name.clone()),
            app::Row::TalkMore(_) => Some("more".to_string()),
            _ => None,
        })
        .collect()
}

fn pick_talk(app: &mut App, name: &str) {
    app.focus = Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(|r| {
            matches!(*r, app::Row::Talk(s, i) if app.chat.spaces[s].talks[i].channel.name == name)
        })
        .unwrap();
}

#[test]
fn the_sidebar_has_the_busiest_conversations_and_any_unread() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    let mut want = vec!["SLACK".to_string(), "Ana".to_string()];
    want.extend((0..11).map(|i| format!("ch{i:02}")));
    want.extend(["ch13".to_string(), "more".to_string()]);
    assert_eq!(talks(app), want);
    let screen = app::screen(app, 120, 40);
    assert!(screen.contains("SLACK"), "{screen}");
    // Its row in the sidebar, not the panes beside it.
    let ch13 = screen.lines().find(|l| l.contains("ch13")).unwrap();
    let side = ch13.split('│').next().unwrap();
    assert!(side.trim_end().ends_with('4'), "{ch13}");
    assert!(screen.contains("2 more"), "{screen}");
}

#[test]
fn every_conversation_is_shown_once_asked_and_folded_again() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app.focus = Focus::Side;
    app.row = app
        .rows
        .iter()
        .position(|r| matches!(r, app::Row::TalkMore(_)))
        .unwrap();
    press(app, KeyCode::Enter);
    assert_eq!(talks(app).len(), 1 + 15 + 1);
    assert!(app::screen(app, 120, 40).contains("fewer"));
    press(app, KeyCode::Enter);
    assert_eq!(talks(app).len(), 1 + 13 + 1);
}

#[test]
fn a_conversation_is_read_in_the_lists_place_the_newest_last() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::List);
    let screen = app::screen(app, 120, 40);
    let hello = screen.find("hello @Bo").expect(&screen);
    let newest = screen.find("newest note").expect(&screen);
    assert!(hello < newest, "{screen}");
    assert!(screen.contains("1 reply"), "{screen}");
    assert!(screen.contains("#ch00"), "{screen}");
}

#[test]
fn a_thread_is_read_in_the_messages_place() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
    // The newest is picked; the one before it began the thread.
    press(app, KeyCode::Char('k'));
    press(app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Reader);
    let screen = app::screen(app, 120, 40);
    assert!(screen.contains("a reply from Bo"), "{screen}");
    press(app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::List);
}

#[test]
fn reading_slack_marks_nothing_read_and_posts_nothing() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
    press(app, KeyCode::Char('k'));
    press(app, KeyCode::Enter);
    for key in ['d', 'e', 'r', 'f', 'u', '!'] {
        press(app, KeyCode::Char(key));
    }
    let asked = asked.lock().unwrap();
    let methods: std::collections::BTreeSet<_> = asked
        .iter()
        .filter_map(|r| r.split(' ').nth(1))
        .map(|p| p.trim_start_matches('/').to_string())
        .collect();
    let read_only = [
        "users.list",
        "users.conversations",
        "conversations.info",
        "conversations.history",
        "conversations.replies",
    ];
    assert!(
        methods.iter().all(|m| read_only.contains(&m.as_str())),
        "{methods:?}"
    );
    assert!(app.chat.open.is_some(), "the conversation is still read");
    assert!(
        !app.dialog.is_open(),
        "no mail key acted on the hidden list"
    );
}

#[test]
fn a_conversation_opened_from_the_map_is_read_in_the_list() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app.map.mode = crate::map::Mode::Map;
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
    assert_eq!(app.map.mode, crate::map::Mode::List);
    assert!(app::screen(app, 120, 40).contains("newest note"));
}
