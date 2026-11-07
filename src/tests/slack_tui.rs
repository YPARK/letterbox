//! Slack in the TUI: its conversations in the sidebar, by when each was last
//! busy, with what is unread; one read in the list's place, and a thread in
//! the message's. From a server in place of Slack's.

use super::*;
use crate::slack::{Session, Slack};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

/// A server answering each request by the first of `answers` whose every
/// word is in its first line or its body; the requests it took.
fn slack(
    answers: impl FnOnce(&str) -> Vec<(&'static str, Value)>,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let answers = answers(&base);
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
    workspace_posting(json!({"ok": true, "ts": "1700000600.0"}))
}

/// The workspace, answering what is sent with `posted`.
fn workspace_posting(posted: Value) -> (String, Arc<Mutex<Vec<String>>>) {
    let mut channels = vec![json!({"id": "D1", "is_im": true, "user": "U1", "updated": 2_000_000})];
    for i in 0..14 {
        channels.push(json!({
            "id": format!("C{i:02}"), "name": format!("ch{i:02}"),
            "is_channel": true, "updated": 1_000_000 - i,
        }));
    }
    let message = |ts: &str, user: &str, text: &str| json!({"ts": ts, "user": user, "text": text});
    slack(move |base| {
        vec![
            ("chat.postMessage", posted),
            ("files-pri", json!("the file's bytes")),
            (
                "search.messages",
                json!({"ok": true, "messages": {"matches": [
                    {"channel": {"id": "C00", "name": "ch00"}, "ts": "1700000100.0", "user": "U1",
                     "text": "hello <@U2>", "permalink": "https://acme.slack.com/archives/C00/p1"},
                    {"channel": {"id": "C00", "name": "ch00"}, "ts": "1700000200.0", "user": "U2",
                     "text": "a reply from Bo",
                     "permalink": "https://acme.slack.com/archives/C00/p2?thread_ts=1700000100.0"},
                ]}}),
            ),
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
                "conversations.history latest=",
                json!({"ok": true, "messages": [message("1600000000.0", "U1", "an old one")]}),
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
                    {"ts": "1700000300.0", "user": "U2", "text": "newest note", "files": [{
                        "name": "data.csv", "size": 2048,
                        "url_private_download": format!("{base}/files-pri/T1-F1/download/data.csv"),
                    }]},
                    {"ts": "1700000100.0", "user": "U1", "text": "hello <@U2>",
                     "thread_ts": "1700000100.0", "reply_count": 1},
                ]}),
            ),
        ]
    })
}

fn app_on(base: &str) -> App {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(Vec::new(), dir.path().join("accounts.toml"));
    let session = Session {
        domain: "acme".into(),
        token: "xoxp-test".into(),
        app: None,
    };
    app.chat_add("acme".into(), Slack::at(base, session), None);
    app.settle_chat(PATIENCE);
    app.chat.shown = true;
    app
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
    app.settle_chat(PATIENCE);
}

/// The conversations in the sidebar, by name, and a row for those folded.
fn talks(app: &App) -> Vec<String> {
    use crate::app::chat::{ChatRow, Part};
    app.chat
        .rows
        .iter()
        .map(|r| match *r {
            ChatRow::Head(_) => "SLACK".to_string(),
            ChatRow::Section(_, Part::Channels) => "CHANNELS".to_string(),
            ChatRow::Section(_, Part::People) => "PEOPLE".to_string(),
            ChatRow::Talk(s, i) => app.chat.spaces[s].talks[i].channel.name.clone(),
            ChatRow::More(_) => "more".to_string(),
        })
        .collect()
}

fn pick_talk(app: &mut App, name: &str) {
    app.focus = Focus::Chat;
    app.chat.row = app
        .chat
        .rows
        .iter()
        .position(|r| {
            matches!(*r, crate::app::chat::ChatRow::Talk(s, i)
                if app.chat.spaces[s].talks[i].channel.name == name)
        })
        .unwrap();
}

#[test]
fn the_column_has_the_busiest_channels_and_people_and_any_unread() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    let mut want = vec!["SLACK".to_string(), "CHANNELS".to_string()];
    want.extend((0..11).map(|i| format!("ch{i:02}")));
    want.extend(["ch13", "PEOPLE", "Ana", "more"].map(String::from));
    assert_eq!(talks(app), want);
    let screen = app::screen(app, 120, 40);
    assert!(screen.contains("SLACK"), "{screen}");
    // Its row in the column, at the right, past every pane.
    let ch13 = screen.lines().find(|l| l.contains("ch13")).unwrap();
    let column = ch13.rsplit('│').next().unwrap();
    assert!(
        column.contains("ch13") && column.trim_end().ends_with('4'),
        "{ch13}"
    );
    assert!(screen.contains("2 more"), "{screen}");
}

#[test]
fn every_conversation_is_shown_once_asked_and_folded_again() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app.focus = Focus::Chat;
    app.chat.row = app
        .chat
        .rows
        .iter()
        .position(|r| matches!(r, crate::app::chat::ChatRow::More(_)))
        .unwrap();
    press(app, KeyCode::Enter);
    assert_eq!(talks(app).len(), 3 + 15 + 1);
    assert!(app::screen(app, 120, 40).contains("fewer"));
    press(app, KeyCode::Enter);
    assert_eq!(talks(app).len(), 3 + 13 + 1);
}

#[test]
fn k_shows_the_slack_column_beside_the_mail_and_hides_it() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app.chat.shown = false;
    app.focus = Focus::Side;
    assert!(
        !app::screen(app, 120, 40).contains("SLACK"),
        "hidden at first"
    );
    press(app, KeyCode::Char('K'));
    assert_eq!(app.focus, Focus::Chat);
    let screen = app::screen(app, 120, 40);
    assert!(
        screen.contains("SLACK · ACME") && screen.contains("CHANNELS") && screen.contains("PEOPLE")
    );
    // In its own column, not in the mail's sidebar.
    let ch00 = screen.lines().find(|l| l.contains("ch00")).unwrap();
    assert!(ch00.rsplit('│').next().unwrap().contains("ch00"), "{ch00}");
    press(app, KeyCode::Char('K'));
    assert_eq!(app.focus, Focus::Side);
    assert!(!app::screen(app, 120, 40).contains("CHANNELS"));
}

#[test]
fn tab_goes_round_the_panes_and_the_slack_column() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app.focus = Focus::Side;
    let mut went = Vec::new();
    for _ in 0..3 {
        press(app, KeyCode::Tab);
        went.push(app.focus);
    }
    assert_eq!(went, [Focus::List, Focus::Chat, Focus::Side]);
}

#[test]
fn a_narrow_window_gives_the_column_the_screen_while_it_has_the_focus() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app.focus = Focus::Chat;
    assert!(app::screen(app, 60, 30).contains("CHANNELS"));
    app.focus = Focus::List;
    assert!(!app::screen(app, 60, 30).contains("CHANNELS"));
}

#[test]
fn a_click_on_a_conversation_reads_it() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app::screen(app, 120, 40);
    let at = app
        .chat
        .rows
        .iter()
        .position(|r| {
            matches!(*r, crate::app::chat::ChatRow::Talk(s, i)
                if app.chat.spaces[s].talks[i].channel.name == "ch00")
        })
        .unwrap();
    let rect = app
        .targets
        .iter()
        .find(|(_, t)| *t == app::Target::ChatRow(at))
        .map(|(r, _)| *r)
        .expect("a target for the row");
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + 2,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    });
    app.settle_chat(PATIENCE);
    assert!(app.chat.open.as_ref().is_some_and(|o| o.id == "C00"));
    assert_eq!(app.focus, Focus::List);
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

fn live(channel: &str, ts: &str, thread: Option<&str>, new: bool) -> crate::slack::live::Event {
    crate::slack::live::Event::Message(crate::slack::live::Live {
        channel: channel.into(),
        ts: ts.into(),
        user: "U2".into(),
        thread: thread.map(String::from),
        new,
    })
}

fn heard(app: &mut App, event: crate::slack::live::Event, mine: bool) {
    app.chat_hear(0, crate::slack::worker::Answer::Live(event, mine));
    app.settle_chat(PATIENCE);
}

fn unread(app: &App, name: &str) -> Option<u32> {
    app.chat.spaces[0]
        .talks
        .iter()
        .find(|t| t.channel.name == name)
        .and_then(|t| t.unread)
}

#[test]
fn a_message_come_elsewhere_is_counted_and_shown() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    assert!(!talks(app).contains(&"ch12".to_string()), "folded at first");
    heard(app, live("C12", "9.0", None, true), false);
    assert_eq!(unread(app, "ch12"), Some(1));
    assert!(
        talks(app).contains(&"ch12".to_string()),
        "shown once unread"
    );
    heard(app, live("C12", "9.5", None, true), false);
    assert_eq!(unread(app, "ch12"), Some(2));
}

#[test]
fn ones_own_messages_edits_and_replies_are_not_counted() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    heard(app, live("C12", "9.0", None, true), true);
    heard(app, live("C12", "8.0", None, false), false);
    heard(app, live("C12", "9.1", Some("8.0"), true), false);
    assert_eq!(unread(app, "ch12"), Some(0));
}

#[test]
fn a_message_come_in_the_conversation_read_is_read_at_once() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
    press(app, KeyCode::Char('k'));
    press(app, KeyCode::Enter);
    let count = |what: &str| {
        let asked = asked.lock().unwrap();
        asked
            .iter()
            .filter(|r| r.contains(what) && r.contains("channel=C00") && !r.contains("oldest="))
            .count()
    };
    let (history, replies) = (
        count("/conversations.history"),
        count("/conversations.replies"),
    );
    heard(app, live("C00", "1700000400.0", None, true), false);
    assert_eq!(count("/conversations.history"), history + 1);
    heard(
        app,
        live("C00", "1700000500.0", Some("1700000100.0"), true),
        false,
    );
    assert_eq!(
        count("/conversations.replies"),
        replies + 1,
        "the thread read again"
    );
    assert_eq!(
        count("/conversations.history"),
        history + 2,
        "and its count of replies"
    );
    assert_eq!(unread(app, "ch00"), Some(0), "what is read is not counted");
}

#[test]
fn the_heading_says_whether_slack_is_heard_as_it_happens() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    assert!(!app::screen(app, 120, 40).contains("live"));
    heard(app, crate::slack::live::Event::Connected, false);
    assert!(app::screen(app, 120, 40).contains("SLACK · ACME · live"));
    heard(app, crate::slack::live::Event::Lost("gone".into()), false);
    assert!(!app::screen(app, 120, 40).contains("· live"));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    app.settle_chat(PATIENCE);
}

/// What was sent, each its request.
fn posts(asked: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    let asked = asked.lock().unwrap();
    asked
        .iter()
        .filter(|r| r.contains("/chat.postMessage"))
        .cloned()
        .collect()
}

fn open_ch00(app: &mut App) {
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
}

#[test]
fn a_message_goes_as_soon_as_it_is_sent_and_is_let_go() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('r'));
    assert!(app::screen(app, 120, 40).contains("To #ch00"));
    // What is written is written: a y is a y, not a yes.
    typed(app, "yes, hi all");
    assert!(posts(&asked).is_empty(), "nothing sent while it is written");
    ctrl(app, 'c');
    ctrl(app, 'c');
    let sent = posts(&asked);
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].contains("channel=C00") && sent[0].contains("text=yes%2C+hi+all"),
        "{}",
        sent[0]
    );
    assert!(!sent[0].contains("thread_ts"), "{}", sent[0]);
    assert!(!sent[0].contains("%0A"), "no line after it: {}", sent[0]);
    assert!(app.chat.writing.is_none());
    assert!(app.status.contains("sent to #ch00"), "{}", app.status);
}

#[test]
fn esc_lets_what_is_written_go_unsent() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('r'));
    typed(app, "draft");
    press(app, KeyCode::Esc);
    assert!(app.chat.writing.is_none());
    assert!(posts(&asked).is_empty());
}

#[test]
fn a_reply_goes_into_the_thread_read() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('k'));
    press(app, KeyCode::Enter);
    press(app, KeyCode::Char('r'));
    assert!(app::screen(app, 120, 40).contains("in its thread"));
    typed(app, "thanks");
    ctrl(app, 'c');
    ctrl(app, 'c');
    let sent = posts(&asked);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("thread_ts=1700000100.0"), "{}", sent[0]);
}

#[test]
fn what_could_not_be_sent_is_kept_and_why_said() {
    let (base, asked) = workspace_posting(json!({"ok": false, "error": "missing_scope"}));
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('r'));
    typed(app, "lost?");
    ctrl(app, 'c');
    ctrl(app, 'c');
    assert_eq!(posts(&asked).len(), 1);
    let writing = app.chat.writing.as_ref().expect("still written");
    assert_eq!(writing.buffer.text().trim_end(), "lost?");
    assert!(app::screen(app, 120, 40).contains("chat:write"));
}

#[test]
fn nothing_written_is_not_sent() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('r'));
    ctrl(app, 'c');
    ctrl(app, 'c');
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
    app.settle_chat(PATIENCE);
    assert!(app.chat.writing.is_some(), "still being written");
    assert!(posts(&asked).is_empty());
}

/// The demo's mail, and the workspace beside it.
fn mail_and_slack(base: &str) -> App {
    let mut app = App::new(
        vec![(
            "Demo".to_string(),
            Box::new(crate::demo::Demo::new()) as Box<dyn Store>,
        )],
        PathBuf::from("/nowhere/accounts.toml"),
    );
    app.ledger = scratch_ledger();
    app.settle(PATIENCE);
    let session = Session {
        domain: "acme".into(),
        token: "xoxp-test".into(),
        app: None,
    };
    app.chat_add("acme".into(), Slack::at(base, session), None);
    app.settle_chat(PATIENCE);
    app.chat.shown = true;
    app
}

#[test]
fn no_key_from_any_pane_acts_on_the_mail_behind_a_conversation() {
    let (base, _) = workspace();
    let app = &mut mail_and_slack(&base);
    assert!(!app.shown.is_empty(), "mail is listed behind it");
    app.list.select(Some(0));
    let seen = app.shown[0].summary.flags.seen;
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
    for focus in [Focus::Side, Focus::List, Focus::Reader, Focus::Chat] {
        app.focus = focus;
        for key in ['d', 'e', 'u', '!', 'f'] {
            press(app, KeyCode::Char(key));
            app.settle(PATIENCE);
        }
    }
    assert!(app.batch.jobs.is_empty(), "nothing moved");
    assert_eq!(app.shown[0].summary.flags.seen, seen, "nothing marked");
    assert!(app.compose.is_none(), "nothing answered");
}

#[test]
fn mail_searched_for_is_shown_in_the_conversations_place() {
    let (base, _) = workspace();
    let app = &mut mail_and_slack(&base);
    pick_talk(app, "ch00");
    press(app, KeyCode::Enter);
    app.focus = Focus::Side;
    press(app, KeyCode::Char('/'));
    typed(app, "a");
    press(app, KeyCode::Enter);
    app.settle(PATIENCE);
    assert!(
        app.chat.open.is_none(),
        "the conversation let go for what was found"
    );
}

#[test]
fn ctrl_or_alt_enter_sends_and_enter_is_a_new_line() {
    for held in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        let (base, asked) = workspace();
        let app = &mut app_on(&base);
        open_ch00(app);
        press(app, KeyCode::Char('r'));
        typed(app, "one");
        press(app, KeyCode::Enter);
        typed(app, "two");
        let writing = app.chat.writing.as_ref().unwrap();
        assert_eq!(
            writing.buffer.text().trim_end(),
            "one\ntwo",
            "a plain enter is a new line"
        );
        assert!(posts(&asked).is_empty());
        app.key(KeyEvent::new(KeyCode::Enter, held));
        app.settle_chat(PATIENCE);
        let sent = posts(&asked);
        assert_eq!(sent.len(), 1, "{held:?}");
        assert!(sent[0].contains("text=one%0Atwo"), "{}", sent[0]);
        assert!(app.chat.writing.is_none());
    }
}

fn jump(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    app.settle_chat(PATIENCE);
}

#[test]
fn ctrl_k_finds_any_conversation_by_part_of_its_name() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    app.focus = Focus::Side;
    jump(app);
    assert!(app.chat.jump.is_some());
    typed(app, "CH1");
    let screen = app::screen(app, 120, 40);
    assert!(screen.contains("Go to"), "{screen}");
    // Folded away, it is found all the same.
    assert!(screen.contains("ch12"), "{screen}");
    // The one with anything unread first, of those that fit as well.
    press(app, KeyCode::Enter);
    assert!(app.chat.jump.is_none());
    assert!(app.chat.open.as_ref().is_some_and(|o| o.id == "C13"));
    assert_eq!(app.focus, Focus::List);
}

#[test]
fn the_arrows_pick_among_those_found_and_esc_lets_it_go() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    jump(app);
    typed(app, "ch0");
    press(app, KeyCode::Down);
    press(app, KeyCode::Enter);
    assert!(
        app.chat.open.as_ref().is_some_and(|o| o.id == "C01"),
        "the second found"
    );
    jump(app);
    typed(app, "ana");
    press(app, KeyCode::Esc);
    assert!(app.chat.jump.is_none());
    assert!(
        app.chat.open.as_ref().is_some_and(|o| o.id == "C01"),
        "nothing else opened"
    );
}

#[test]
fn nothing_found_opens_nothing() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    jump(app);
    typed(app, "nobody here");
    assert!(app::screen(app, 120, 40).contains("nothing fits"));
    press(app, KeyCode::Enter);
    assert!(app.chat.jump.is_some(), "still asked");
    assert!(app.chat.open.is_none());
}

#[test]
fn ctrl_k_while_writing_is_the_editors_not_a_jump() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('r'));
    typed(app, "keep this");
    jump(app);
    assert!(app.chat.jump.is_none());
    assert!(app.chat.writing.is_some());
}

#[test]
fn a_messages_files_are_shown_under_it() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    let screen = app::screen(app, 120, 40);
    // The clip is two cells wide, as the screen draws it.
    let line = screen
        .lines()
        .find(|l| l.contains("data.csv"))
        .expect(&screen);
    assert!(
        line.contains("📎") && line.contains("data.csv (2 KB)"),
        "{line}"
    );
}

#[test]
fn s_saves_the_files_of_the_message_picked_where_mail_is_saved() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    let dir = tempfile::tempdir().unwrap();
    app.save_dir = Some(dir.path().to_path_buf());
    open_ch00(app);
    // The newest is picked, and has a file.
    press(app, KeyCode::Char('s'));
    let saved = std::fs::read_to_string(dir.path().join("data.csv")).unwrap();
    assert_eq!(saved, "\"the file's bytes\"");
    assert!(app.status.contains("data.csv"), "{}", app.status);
    let fetched = asked
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.contains("files-pri"))
        .count();
    assert_eq!(fetched, 1);
    // Saved again, it is saved beside the first, not over it.
    press(app, KeyCode::Char('s'));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    // One with none says so, and fetches nothing.
    press(app, KeyCode::Char('k'));
    press(app, KeyCode::Char('s'));
    assert!(app.status.contains("no files"), "{}", app.status);
}

fn search(app: &mut App, words: &str) {
    press(app, KeyCode::Char('/'));
    typed(app, words);
    press(app, KeyCode::Enter);
}

#[test]
fn slash_in_slack_searches_it_and_lists_what_it_finds() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('/'));
    assert!(app::screen(app, 120, 40).contains("Search Slack"));
    typed(app, "hello");
    press(app, KeyCode::Enter);
    let screen = app::screen(app, 120, 40);
    assert!(screen.contains("Found: hello"), "{screen}");
    assert!(
        screen.contains("Ana · #ch00") && screen.contains("a reply from Bo"),
        "{screen}"
    );
    let searched = asked
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.contains("/search.messages"))
        .count();
    assert_eq!(searched, 1);
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("query=hello"))
    );
}

#[test]
fn a_hit_opens_its_conversation_at_it_and_a_replys_its_thread() {
    let (base, _) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    search(app, "hello");
    // The first found, the message that began the thread.
    press(app, KeyCode::Char('g'));
    press(app, KeyCode::Enter);
    let open = app.chat.open.as_ref().unwrap();
    assert_eq!(open.id, "C00");
    let picked = open.picked.map(|i| open.messages[i].ts.clone());
    assert_eq!(picked.as_deref(), Some("1700000100.0"));
    // The reply: its thread is read too.
    search(app, "hello");
    press(app, KeyCode::Char('G'));
    press(app, KeyCode::Enter);
    let open = app.chat.open.as_ref().unwrap();
    assert_eq!(
        open.thread.as_ref().map(|t| t.ts.as_str()),
        Some("1700000100.0")
    );
}

#[test]
fn esc_lets_a_search_go_and_slash_elsewhere_is_mails() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('/'));
    typed(app, "x");
    press(app, KeyCode::Esc);
    assert!(app.chat.search.is_none());
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .all(|r| !r.contains("/search.messages"))
    );
    app.focus = Focus::Side;
    press(app, KeyCode::Char('/'));
    assert!(app.searching, "mail's search, from the mail's sidebar");
    assert!(app.chat.search.is_none());
}

#[test]
fn up_past_the_oldest_shown_reads_older_and_they_stay() {
    let (base, asked) = workspace();
    let app = &mut app_on(&base);
    open_ch00(app);
    press(app, KeyCode::Char('k'));
    press(app, KeyCode::Char('k'));
    let open = app.chat.open.as_ref().unwrap();
    assert_eq!(open.messages.len(), 3);
    assert_eq!(open.messages[0].text, "an old one");
    assert_eq!(open.picked, Some(1), "still on the one it was on");
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("latest=1700000100.0")),
        "asked for those before the oldest"
    );
    press(app, KeyCode::Char('k'));
    assert!(app::screen(app, 120, 40).contains("an old one"));
    // Slack has none older than what it gave: the beginning, said.
    press(app, KeyCode::Char('k'));
    assert!(app.status.contains("beginning"), "{}", app.status);
    assert_eq!(app.chat.open.as_ref().unwrap().messages.len(), 3);
    // Read again, what was read before stays.
    press(app, KeyCode::Char('U'));
    let open = app.chat.open.as_ref().unwrap();
    assert_eq!(
        open.messages.len(),
        3,
        "the older kept as the newest are read again"
    );
}
