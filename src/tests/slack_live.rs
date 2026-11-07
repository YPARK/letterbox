//! Slack's events as they happen, over Socket Mode: from a server in place of
//! Slack's API, and one in place of its socket.

use crate::slack::live::{Event, Live, Socket};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Slack's API, handing out `socket` to each that asks to connect; what each
/// asked, its head and body.
fn api(socket: String) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let taken = asked.clone();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
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
            request.push_str(&String::from_utf8_lossy(&body));
            taken.lock().unwrap().push(request);
            let answer = json!({"ok": true, "url": socket}).to_string();
            let _ = write!(
                &tcp,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{answer}",
                answer.len()
            );
        }
    });
    (base, asked)
}

/// A socket server that plays `each` connection's frames in turn, then
/// sends on `acks` each answer it is given.
fn socket(each: Vec<Vec<Value>>) -> (String, mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/link", listener.local_addr().unwrap());
    let (acks_to, acks) = mpsc::channel();
    std::thread::spawn(move || {
        for frames in each {
            let (tcp, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(tcp).unwrap();
            for frame in frames {
                ws.send(frame.to_string().into()).unwrap();
            }
            let acks_to = acks_to.clone();
            std::thread::spawn(move || {
                while let Ok(m) = ws.read() {
                    if let Ok(text) = m.to_text()
                        && let Ok(v) = serde_json::from_str::<Value>(text)
                    {
                        let _ = acks_to.send(v);
                    }
                }
            });
        }
    });
    (url, acks)
}

fn message(envelope: &str, event: Value) -> Value {
    json!({"envelope_id": envelope, "type": "events_api", "payload": {"event": event}})
}

/// The events heard until the first message, at most `most`.
fn hear(socket: &Socket, most: usize) -> Vec<Event> {
    let mut heard = Vec::new();
    socket.listen(&mut |event| {
        let done = matches!(event, Event::Message(_));
        heard.push(event);
        !done && heard.len() < most
    });
    heard
}

#[test]
fn a_message_is_heard_and_answered_as_slack_asks() {
    let (url, acks) = socket(vec![vec![
        json!({"type": "hello"}),
        message(
            "e1",
            json!({"type": "message", "channel": "C1", "user": "U1", "ts": "1.0", "text": "hi"}),
        ),
    ]]);
    let (base, asked) = api(url);
    let heard = hear(&Socket::at(&base, "xapp-secret"), 10);
    assert_eq!(
        heard,
        [
            Event::Connected,
            Event::Message(Live {
                channel: "C1".into(),
                ts: "1.0".into(),
                user: "U1".into(),
                thread: None,
                new: true,
            })
        ]
    );
    assert_eq!(
        acks.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap(),
        json!({"envelope_id": "e1"})
    );
    let asked = asked.lock().unwrap();
    assert!(
        asked[0].starts_with("POST /apps.connections.open "),
        "{}",
        asked[0]
    );
    assert!(
        asked[0]
            .to_lowercase()
            .contains("authorization: bearer xapp-secret"),
        "{}",
        asked[0]
    );
}

#[test]
fn slack_saying_to_go_is_gone_and_come_back_to() {
    let (url, _) = socket(vec![
        vec![
            json!({"type": "hello"}),
            json!({"type": "disconnect", "reason": "refresh_requested"}),
        ],
        vec![
            json!({"type": "hello"}),
            message(
                "e2",
                json!({"type": "message", "channel": "C2", "user": "U2", "ts": "2.0"}),
            ),
        ],
    ]);
    let (base, asked) = api(url);
    let heard = hear(&Socket::at(&base, "xapp-secret"), 10);
    assert_eq!(heard.first(), Some(&Event::Connected));
    assert!(matches!(heard.last(), Some(Event::Message(m)) if m.channel == "C2"));
    assert_eq!(asked.lock().unwrap().len(), 2, "connected again");
}

#[test]
fn replies_edits_and_deletions_are_heard_as_messages_of_their_conversation() {
    let reply =
        json!({"type": "message", "channel": "C1", "user": "U1", "ts": "2.0", "thread_ts": "1.0"});
    let edit = json!({"type": "message", "subtype": "message_changed", "channel": "C3",
        "message": {"user": "U3", "ts": "3.0"}});
    let gone = json!({"type": "message", "subtype": "message_deleted", "channel": "C4",
        "previous_message": {"user": "U4", "ts": "4.0", "thread_ts": "1.5"}});
    let (url, _) = socket(vec![vec![
        json!({"type": "hello"}),
        message("e1", reply),
        message("e2", edit),
        message("e3", gone),
    ]]);
    let (base, _) = api(url);
    let socket = Socket::at(&base, "xapp-secret");
    let mut heard = Vec::new();
    socket.listen(&mut |event| {
        if let Event::Message(m) = event {
            heard.push(m);
        }
        heard.len() < 3
    });
    let got: Vec<_> = heard
        .iter()
        .map(|m| {
            let at = (m.channel.as_str(), m.ts.as_str(), m.user.as_str());
            (at, m.thread.as_deref(), m.new)
        })
        .collect();
    assert_eq!(
        got,
        [
            (("C1", "2.0", "U1"), Some("1.0"), true),
            (("C3", "3.0", "U3"), None, false),
            (("C4", "4.0", "U4"), Some("1.5"), false),
        ]
    );
}

#[test]
fn only_slacks_own_secure_sockets_are_opened() {
    use crate::slack::live::slacks;
    assert!(slacks("wss://wss-primary.slack.com/link/?ticket=1"));
    assert!(slacks("wss://slack.com/link"));
    for not in [
        "ws://wss-primary.slack.com/link",
        "wss://evil.example/link",
        "wss://slack.com.evil.example/link",
        "wss://evilslack.com/link",
        "https://wss-primary.slack.com/link",
        "not a url",
    ] {
        assert!(!slacks(not), "{not}");
    }
}
