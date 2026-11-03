//! Signing in with Microsoft, and the mail served to an AI agent over MCP.

use super::*;

// ---- Microsoft, as letterbox signs in with it ----

/// A stand-in for Microsoft's sign-in, on this machine, which letterbox is
/// pointed at in every test: an address at `uni.example` is its own; a code
/// is taken from the browser if its verifier answers the challenge the
/// browser was given; a device's code is taken on the second time it is
/// asked; tokens are renewed.
pub(super) fn microsoft() -> &'static str {
    static AT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    AT.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        // SAFETY: set once, before any test signs in.
        unsafe { std::env::set_var("LETTERBOX_MS_LOGIN", &at) };
        std::thread::spawn(move || {
            for tcp in listener.incoming().flatten() {
                std::thread::spawn(move || answer_as_microsoft(tcp));
            }
        });
        at
    })
}

/// The pages the tests' browser was shown, coming back to letterbox.
pub(super) static PAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether the browser was shown `said`, within a second or two: it reads
/// the page on a thread of its own.
pub(super) fn shown(said: &str) -> bool {
    let until = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if PAGES.lock().unwrap().iter().any(|p| p.contains(said)) {
            return true;
        }
        if std::time::Instant::now() > until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The challenge the test's browser was given, to check the verifier by.
pub(super) static CHALLENGE: Mutex<Option<String>> = Mutex::new(None);
pub(super) static DEVICES_ASKED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub(super) fn answer_as_microsoft(tcp: TcpStream) {
    use std::io::Read;
    let mut reader = BufReader::new(tcp.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header.trim().is_empty() {
            break;
        }
        if let Some(v) = header.to_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let form: std::collections::HashMap<String, String> = String::from_utf8_lossy(&body)
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.replace("%3A", ":").replace("%2F", "/")))
        .collect();
    let target = first.split_whitespace().nth(1).unwrap_or_default();
    let tokens = r#"{"access_token":"access-1","refresh_token":"refresh-1","expires_in":3600}"#;
    let json = if target.starts_with("/getuserrealm.srf") {
        let ours = target.contains("uni.example");
        format!(
            r#"{{"NameSpaceType":"{}"}}"#,
            if ours { "Managed" } else { "Unknown" }
        )
    } else if target.ends_with("/devicecode") {
        let n = DEVICES_ASKED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!(
            r#"{{"device_code":"device-{n}","user_code":"LBX-CODE","verification_uri":"https://microsoft.example/device","interval":1,"expires_in":900}}"#
        )
    } else if target.ends_with("/token") {
        match form.get("grant_type").map(String::as_str) {
            Some("authorization_code") => {
                use base64::Engine;
                use sha2::Digest;
                let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                let answered = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(sha2::Sha256::digest(verifier.as_bytes()));
                let challenge = CHALLENGE.lock().unwrap().clone();
                if form.get("code").map(String::as_str) == Some("code-1")
                    && challenge.as_deref() == Some(answered.as_str())
                {
                    tokens.to_string()
                } else {
                    r#"{"error":"invalid_grant","error_description":"AADSTS70000: bad code"}"#
                        .to_string()
                }
            }
            Some(grant) if grant.contains("device_code") => {
                // Asked first while no one has signed in, then signed in.
                static POLLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
                let code = form.get("device_code").cloned().unwrap_or_default();
                let mut polled = POLLED.lock().unwrap();
                if polled.contains(&code) {
                    tokens.to_string()
                } else {
                    polled.push(code);
                    r#"{"error":"authorization_pending"}"#.to_string()
                }
            }
            Some("refresh_token") => r#"{"access_token":"access-2","expires_in":3600}"#.to_string(),
            _ => r#"{"error":"unsupported_grant_type"}"#.to_string(),
        }
    } else {
        r#"{"error":"not_found"}"#.to_string()
    };
    let mut out = tcp;
    let _ = write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
}

/// The query of `url`, by name.
pub(super) fn query(url: &str) -> std::collections::HashMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default()
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.replace("%3A", ":").replace("%2F", "/")))
        .collect()
}

/// A browser in which you sign in on Microsoft's page: it comes back to
/// letterbox with `back` added to what letterbox asked to have back.
pub(super) fn come_back(url: &str, back: &str) -> String {
    let q = query(url);
    *CHALLENGE.lock().unwrap() = q.get("code_challenge").cloned();
    let redirect = q.get("redirect_uri").cloned().unwrap();
    let state = q.get("state").cloned().unwrap();
    let to = redirect.trim_start_matches("http://localhost:").to_string();
    let request = format!("GET /?{back}&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n");
    std::thread::spawn(move || {
        let mut tcp = TcpStream::connect(("127.0.0.1", to.parse::<u16>().unwrap())).unwrap();
        tcp.write_all(request.as_bytes()).unwrap();
        let mut page = String::new();
        use std::io::Read;
        let _ = tcp.read_to_string(&mut page);
        page
    })
    .join()
    .unwrap()
}

#[test]
fn an_outlook_account_is_signed_into_in_the_browser() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    app.browse = |url| {
        let page = come_back(url, "code=code-1");
        PAGES.lock().unwrap().push(page);
    };
    press(&mut app, &["A", "me@uni.example", "<tab>"]);
    let form = app.dialog.form().unwrap();
    assert_eq!(
        form.auth,
        crate::config::Auth::Microsoft,
        "Microsoft says so"
    );
    assert_eq!(form.fields[2], "outlook.office365.com");
    assert!(
        !form.shown().contains(&app::PASSWORD),
        "no password asked for"
    );
    let port = account.port.unwrap().to_string();
    press(
        &mut app,
        &["<tab>", "<c-u>", "127.0.0.1", "<tab>", "<c-u>", &port],
    );
    press(&mut app, &["<tab>", "<c-u>", "me", "<tab>", "<enter>"]);
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(app.status, "added uni.example");
    assert_eq!(app.shown.len(), 3, "signed in to IMAP with the token");
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("auth = \"microsoft\""), "{file}");
    assert!(
        !file.contains("refresh") && !file.contains("access"),
        "{file}"
    );
    assert!(shown("Signed in. You may close this tab"));
}

#[test]
fn an_outlook_account_is_signed_into_with_a_code_on_another_device() {
    microsoft();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let mut shown = None;
    let tokens = crate::oauth::sign_in(
        "me@uni.example",
        "common",
        crate::oauth::CLIENT_ID,
        &stop,
        |_| {},
        |w| shown = w.device.clone(),
    )
    .unwrap();
    assert_eq!(
        shown,
        Some((
            "https://microsoft.example/device".to_string(),
            "LBX-CODE".to_string()
        ))
    );
    assert_eq!(
        (tokens.access.as_str(), tokens.refresh.as_str()),
        ("access-1", "refresh-1")
    );
}

#[test]
fn a_sign_in_refused_in_the_browser_is_said() {
    microsoft();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let e = crate::oauth::sign_in(
        "me@uni.example",
        "common",
        crate::oauth::CLIENT_ID,
        &stop,
        |url| {
            let page = come_back(
                url,
                "error=access_denied&error_description=Your+admin+said+no",
            );
            PAGES.lock().unwrap().push(page);
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "Your admin said no");
    assert!(shown("<p>Your admin said no"), "the browser is told too");
}

#[test]
fn a_token_running_out_is_renewed() {
    microsoft();
    let mut a = crate::setup::account("O", "me@uni.example", "outlook.example", 993, "me");
    a.auth = crate::config::Auth::Microsoft;
    a.password_command = Some("echo unused".to_string());
    crate::oauth::remember(
        &a,
        crate::oauth::Tokens {
            access: "access-1".to_string(),
            refresh: "refresh-1".to_string(),
            until: std::time::Instant::now(),
        },
    );
    assert_eq!(crate::oauth::access(&a).unwrap(), "access-2");
    assert_eq!(
        crate::oauth::access(&a).unwrap(),
        "access-2",
        "kept, not renewed again"
    );
}

// ---- for AI agents: MCP ----

pub(super) fn call(
    server: &mut crate::mcp::Server,
    tool: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let reply = server
        .answer(&serde_json::json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": tool, "arguments": args }
        }))
        .unwrap();
    reply["result"].clone()
}

#[test]
fn an_agent_lists_searches_reads_and_marks_mail() {
    use serde_json::json;
    let mut server = crate::mcp::Server::new(&PathBuf::from("/nowhere"), true).unwrap();
    let init = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" } }))
        .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "letterbox");
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("never followed")
    );
    assert!(
        server
            .answer(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none()
    );
    let tools = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_accounts",
            "list_messages",
            "search_messages",
            "read_message",
            "nearest_messages",
            "suggest_label",
            "would_throw_away",
            "recent_messages",
            "triage",
            "uncertain_messages",
            "report_triage",
            "open_letterbox",
            "mark_message"
        ]
    );

    let accounts = call(&mut server, "list_accounts", json!({}));
    let inbox = &accounts["structuredContent"]["accounts"][0]["folders"][0];
    assert_eq!(
        (inbox["path"].as_str(), inbox["total"].as_u64()),
        (Some("INBOX"), Some(INBOX as u64))
    );

    let listed = call(&mut server, "list_messages", json!({ "limit": 2 }));
    let listed = &listed["structuredContent"];
    assert_eq!(listed["shown"], 2);
    assert_eq!(listed["more"], true);
    assert_eq!(listed["messages"][0]["from"], "Ada Lovelace");
    let uid = listed["messages"][0]["uid"].as_u64().unwrap();

    let found = call(&mut server, "search_messages", json!({ "query": "helix" }));
    assert_eq!(
        found["structuredContent"]["messages"][0]["subject"],
        "Photo 51"
    );

    let read = call(
        &mut server,
        "read_message",
        json!({ "uid": uid, "max_chars": 50 }),
    );
    let read = &read["structuredContent"];
    assert_eq!(read["text"].as_str().unwrap().chars().count(), 50);
    assert_eq!(read["more"], "from_char 50 for more");
    let rest = call(
        &mut server,
        "read_message",
        json!({ "uid": uid, "from_char": 50 }),
    );
    assert!(
        rest["structuredContent"]["text"]
            .as_str()
            .unwrap()
            .contains("sets out how")
    );
    let again = call(&mut server, "list_messages", json!({ "limit": 1 }));
    assert_eq!(
        again["structuredContent"]["messages"][0]["seen"], false,
        "read, not marked"
    );

    let marked = call(
        &mut server,
        "mark_message",
        json!({ "uid": uid, "seen": true, "flagged": true }),
    );
    assert_eq!(marked["isError"], false);
    let after = call(&mut server, "list_messages", json!({ "limit": 1 }));
    let first = &after["structuredContent"]["messages"][0];
    assert_eq!(
        (first["seen"].as_bool(), first["flagged"].as_bool()),
        (Some(true), Some(true))
    );

    let wrong = call(&mut server, "list_messages", json!({ "account": "Nope" }));
    assert_eq!(wrong["isError"], true);
    assert!(
        wrong["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("there are Demo")
    );
    let none = call(&mut server, "mark_message", json!({ "uid": uid }));
    assert_eq!(none["content"][0]["text"], "say seen, flagged, or both");
}

/// What a server writes, kept to be read after.
#[derive(Clone, Default)]
pub(super) struct Written(Arc<Mutex<Vec<u8>>>);

impl Write for Written {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Written {
    pub(super) fn messages(&self) -> Vec<serde_json::Value> {
        String::from_utf8_lossy(&self.0.lock().unwrap())
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

/// The demo's server, begun by a program that can, or cannot, ask the
/// user something for it.
pub(super) fn demo_server(asks: bool) -> crate::mcp::Server {
    use serde_json::json;
    let mut server = crate::mcp::Server::new(&PathBuf::from("/nowhere"), true).unwrap();
    let capabilities = if asks {
        json!({ "elicitation": {} })
    } else {
        json!({})
    };
    server
        .answer(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": capabilities } }))
        .unwrap();
    server
}

/// The uids of the demo's Inbox, newest first, and who each is from.
fn inbox(server: &mut crate::mcp::Server) -> Vec<(u64, String)> {
    let listed = call(server, "list_messages", serde_json::json!({ "limit": 200 }));
    listed["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["uid"].as_u64().unwrap(),
                m["from"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn an_agent_opens_letterbox_in_a_terminal_as_set_and_the_demo_opens_nothing() {
    use serde_json::json;
    let lbx = PathBuf::from("/opt/it's/lbx");
    let quoted = if cfg!(windows) {
        "\"/opt/it's/lbx\""
    } else {
        "'/opt/it'\\''s/lbx'"
    };
    assert_eq!(
        crate::mcp::open_command(Some("ghostty -e {lbx}"), &lbx),
        Some(format!("ghostty -e {quoted}"))
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        crate::mcp::open_command(None, &lbx),
        Some(format!("open -a Terminal {quoted}"))
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(&path, "[agent]\nopen = \"wezterm start {lbx}\"\n").unwrap();
    assert_eq!(
        crate::config::opener(&path).unwrap().as_deref(),
        Some("wezterm start {lbx}")
    );
    assert_eq!(crate::config::agent(&path).unwrap(), None);
    let mut server = demo_server(true);
    let r = call(&mut server, "open_letterbox", json!({}));
    assert_eq!(r["structuredContent"]["opened"], false, "{r}");
}

#[test]
fn how_to_triage_mail_is_served_to_every_agent_as_a_prompt() {
    use serde_json::json;
    let mut server = demo_server(true);
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 3, "method": "prompts/list" }))
        .unwrap();
    assert_eq!(listed["result"]["prompts"][0]["name"], "mail-triage");
    let got = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 4, "method": "prompts/get",
            "params": { "name": "mail-triage" } }))
        .unwrap();
    let text = got["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("report_triage") && text.contains("started_by_letterbox"));
    let none = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 5, "method": "prompts/get",
            "params": { "name": "other" } }))
        .unwrap();
    assert_eq!(none["error"]["code"], -32602);
}

#[test]
fn an_agent_moves_mail_only_once_the_user_said_yes_to_it() {
    use serde_json::json;
    let mut server = demo_server(true);
    // Offered where the user can be asked; not otherwise.
    let offered = |server: &mut crate::mcp::Server| {
        let listed = server
            .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .unwrap();
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "trash_messages")
    };
    assert!(offered(&mut server));
    assert!(!offered(&mut demo_server(false)));
    let before = inbox(&mut server);
    let (first, second, third) = (before[0].0, before[1].0, before[2].0);
    let label = call(&mut server, "suggest_label", json!({ "uid": third }));
    let label = label["structuredContent"]["labels"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let tool = |id: u64, name: &str, args: serde_json::Value| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": args } })
        .to_string()
    };
    let said = |id: &str, action: &str| {
        json!({ "jsonrpc": "2.0", "id": id,
            "result": { "action": action, "content": { "confirm": true } } })
        .to_string()
    };
    let script = vec![
        tool(10, "trash_messages", json!({ "uids": [first] })),
        // A ping while the user is asked is answered; what was asked is not.
        json!({ "jsonrpc": "2.0", "id": 11, "method": "ping" }).to_string(),
        said("letterbox-1", "accept"),
        tool(12, "trash_messages", json!({ "uids": [second] })),
        said("letterbox-2", "decline"),
        tool(13, "file_messages", json!({ "uids": [third], "to": label })),
        said("letterbox-3", "accept"),
        tool(
            14,
            "file_messages",
            json!({ "uids": [second], "to": "Made up" }),
        ),
        tool(
            15,
            "file_messages",
            json!({ "uids": [second], "to": "Trash" }),
        ),
        tool(16, "trash_messages", json!({ "uids": [second] })),
        said("letterbox-4", "cancel"),
    ];
    let out = Written::default();
    server
        .serve(crate::mcp::Io {
            lines: Box::new(script.into_iter().map(Ok)),
            out: Box::new(out.clone()),
        })
        .unwrap();
    let written = out.messages();
    let asked: Vec<&serde_json::Value> = written
        .iter()
        .filter(|m| m["method"] == "elicitation/create")
        .collect();
    assert_eq!(asked.len(), 4, "asked of the user, each time: {written:?}");
    let shown = asked[0]["params"]["message"].as_str().unwrap();
    assert!(shown.contains(&before[0].1), "{shown}");
    assert!(shown.contains("put it back"), "{shown}");
    let reply = |id: u64| written.iter().find(|m| m["id"] == id).unwrap().clone();
    assert_eq!(reply(11)["result"], json!({}));
    assert_eq!(reply(10)["result"]["structuredContent"]["moved"], 1);
    assert_eq!(reply(12)["result"]["structuredContent"]["moved"], 0);
    assert_eq!(reply(13)["result"]["structuredContent"]["moved"], 1);
    assert_eq!(
        reply(14)["result"]["isError"],
        true,
        "never a label made up"
    );
    assert_eq!(reply(15)["result"]["isError"], true, "nor filed in Trash");
    let unanswered = reply(16)["result"]["structuredContent"].clone();
    assert_eq!(unanswered["moved"], 0);
    assert!(
        unanswered["said"]
            .as_str()
            .unwrap()
            .contains("did not answer"),
        "{unanswered}"
    );
    assert_eq!(asked.len(), 4, "neither asked of");

    let after: Vec<u64> = inbox(&mut server).iter().map(|m| m.0).collect();
    assert!(!after.contains(&first) && !after.contains(&third));
    assert!(after.contains(&second), "the user said no");
    let trash = call(&mut server, "list_messages", json!({ "folder": "Trash" }));
    assert_eq!(
        trash["structuredContent"]["messages"][0]["from"],
        before[0].1
    );
    // What the user said yes to is learnt: thrown away, and kept.
    let learnt: Vec<(bool, bool)> = server.learnt.iter().map(|t| (t.thrown, t.agent)).collect();
    assert_eq!(learnt, [(true, true), (false, true)]);
}

#[test]
fn an_agent_whose_program_cannot_ask_the_user_moves_nothing() {
    use serde_json::json;
    let mut server = demo_server(false);
    let before = inbox(&mut server);
    let r = call(
        &mut server,
        "trash_messages",
        json!({ "uids": [before[0].0] }),
    );
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("cannot ask the user"),
        "{r}"
    );
    assert_eq!(inbox(&mut server), before);
    assert!(server.learnt.is_empty());
}

#[test]
fn an_agent_letterbox_started_is_told_so_and_moves_nothing() {
    use serde_json::json;
    let mut server = demo_server(true);
    server.background = true;
    let triaged = call(&mut server, "triage", json!({}));
    assert_eq!(triaged["structuredContent"]["started_by_letterbox"], true);
    let listed = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for hidden in ["trash_messages", "file_messages", "open_letterbox"] {
        assert!(!names.contains(&hidden), "{hidden} offered: {names:?}");
        // Nor taken, if called all the same.
        let r = call(&mut server, hidden, json!({ "uids": [1], "to": "x" }));
        assert!(
            r["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("letterbox started"),
            "{r}"
        );
    }
    let before = inbox(&mut server);
    let r = call(
        &mut server,
        "trash_messages",
        json!({ "uids": [before[0].0] }),
    );
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("report_triage"),
        "{r}"
    );
    assert_eq!(inbox(&mut server), before);
}

#[test]
fn new_mail_is_found_though_an_old_message_came_back_to_the_inbox_after_it() {
    use serde_json::json;
    let mut server = demo_server(false);
    let found = |server: &mut crate::mcp::Server| {
        let r = call(server, "recent_messages", json!({ "hours": 24 }));
        r["structuredContent"]["accounts"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["uid"].clone())
            .collect::<Vec<_>>()
    };
    let before = found(&mut server);
    assert!(!before.is_empty());
    // An old one put back from the Archive: the newest to come, dated long ago.
    let store = server.first_store();
    let old = store.page("Archive", 0, 1).unwrap().summaries[0].uid;
    store.relocate("Archive", old, "INBOX", "").unwrap();
    assert_eq!(found(&mut server), before);
}

#[test]
fn an_agent_is_given_new_mail_with_what_letterbox_has_learnt_of_it() {
    use serde_json::json;
    let mut server = demo_server(false);
    let recent = call(&mut server, "recent_messages", json!({ "hours": 24 }));
    let messages = recent["structuredContent"]["accounts"][0]["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!messages.is_empty(), "{recent}");
    assert!(
        messages.len() < inbox(&mut server).len(),
        "only the last day's"
    );
    let unread = call(
        &mut server,
        "recent_messages",
        json!({ "hours": 24, "unread_only": true }),
    );
    let unread = unread["structuredContent"]["accounts"][0]["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert!(unread.iter().all(|m| m["seen"] == false));
    assert!(unread.len() < messages.len());
    let triaged = call(&mut server, "triage", json!({ "hours": 24 }));
    let first = &triaged["structuredContent"]["accounts"][0]["messages"][0];
    assert_eq!(first["uid"], messages[0]["uid"]);
    assert!(first["junk_odds"].is_number(), "{first}");
    assert!(first.get("would_go_to").is_some(), "{first}");
}

#[test]
fn an_agent_is_told_a_vaults_account_is_not_for_it() {
    let _held = VAULT_AT.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    // A vault of its own, never made: not one beside a key file.
    // SAFETY: tests that read it hold VAULT_AT.
    unsafe { std::env::set_var("LETTERBOX_VAULT", dir.path().join("vault.age")) };
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "[[account]]\nname = \"Safe\"\nemail = \"a@b.c\"\nhost = \"imap.b.c\"\nkeep = \"vault\"\n",
    )
    .unwrap();
    crate::vault::lock();
    let mut server = crate::mcp::Server::new(&path, false).unwrap();
    let r = call(&mut server, "list_messages", serde_json::json!({}));
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("keep it in the keyring")
    );
    unsafe { std::env::remove_var("LETTERBOX_VAULT") };
}

#[test]
fn deleting_moves_to_trash_and_back() {
    let mut app = demo();
    assert_eq!(app.shown[0].from, "Ada Lovelace");
    press(&mut app, &["d"]);
    assert!(
        app.shown.iter().all(|s| s.from != "Ada Lovelace"),
        "{}",
        app.status
    );
    assert!(app.status.contains("puts it back"), "{}", app.status);
    assert!(
        !app.status.contains("7 days"),
        "kept longer: {}",
        app.status
    );
    press(&mut app, &["h", "j", "j", "j", "j", "j"]);
    assert_eq!(app.folder_name(), "Trash", "{}", app.status);
    assert_eq!(app.shown[0].from, "Ada Lovelace");
    press(&mut app, &["d"]);
    assert!(app.status.contains("Inbox"), "{}", app.status);
    assert!(app.shown.iter().all(|s| s.from != "Ada Lovelace"));
    press(&mut app, &["h", "k", "k", "k", "k", "k"]);
    assert_eq!(app.folder_name(), "Inbox");
    assert_eq!(app.shown[0].from, "Ada Lovelace");
}

#[test]
fn a_folder_is_chosen_as_trash_when_the_server_names_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "[[account]]\nname = \"Demo\"\nemail = \"you@example.com\"\nhost = \"imap.example.com\"\n",
    )
    .unwrap();
    let mut app = demo();
    app.config = path.clone();
    // A server that marks nothing as Trash: the folder of that name is
    // only a name, until one is chosen.
    for folder in &mut app.accounts[0].folders {
        if folder.role == Role::Trash {
            folder.role = Role::Other;
        }
    }
    press(&mut app, &["d", "<esc>"]);
    assert!(!matches!(app.dialog, app::Dialog::Trash(_)));
    assert!(app.status.contains("not moved"), "{}", app.status);
    assert!(app.shown.iter().any(|s| s.from == "Ada Lovelace"));

    press(&mut app, &["d"]);
    let screen = app::screen(&mut app, 100, 40);
    assert!(
        screen.contains("Which of these folders") && screen.contains("Archive"),
        "{screen}"
    );
    // Never the Inbox, Sent or Drafts: ⏎ on the first must not throw mail
    // into one of them.
    let offered: Vec<String> = app
        .dialog
        .trash_mut()
        .unwrap()
        .folders
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    for kept in ["INBOX", "Sent", "Drafts"] {
        assert!(
            !offered.iter().any(|p| p == kept),
            "{kept} offered: {offered:?}"
        );
    }
    let at = app
        .dialog
        .trash_mut()
        .unwrap()
        .folders
        .iter()
        .position(|(_, name)| name == "Archive")
        .unwrap();
    app.dialog.trash_mut().unwrap().at = at;
    // ⏎ at the foot, clicked, chooses it as ⏎ does.
    click_at(
        &mut app,
        app::Target::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(
        !matches!(app.dialog, app::Dialog::Trash(_)),
        "{}",
        app.status
    );
    assert!(app.status.contains("moved to Archive"), "{}", app.status);
    assert!(app.shown.iter().all(|s| s.from != "Ada Lovelace"));
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("trash = \"Archive\""), "{file}");
    assert!(file.contains("host = \"imap.example.com\""), "{file}");

    press(&mut app, &["h", "j", "j", "j"]);
    assert_eq!(app.folder_name(), "Archive", "{}", app.status);
    assert!(app.shown.iter().any(|s| s.from == "Ada Lovelace"));
    press(&mut app, &["d"]);
    assert!(app.status.contains("put back in Inbox"), "{}", app.status);
}

#[test]
fn a_trash_record_that_cannot_be_read_is_left_as_it_is() {
    let path = std::env::temp_dir().join(format!(
        "letterbox-trash-broken-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let text = "this is not a ledger\n";
    std::fs::write(&path, text).unwrap();
    let mut ledger = crate::trash::Ledger::open(path.clone());
    assert!(ledger.fault().is_some());
    assert!(ledger.stale("a@b.c", i64::MAX).is_empty());
    let err = ledger
        .record(crate::trash::Entry {
            account: "a@b.c".into(),
            message_id: "<x@x>".into(),
            folder: "Trash".into(),
            uid: 1,
            origin: "INBOX".into(),
            at: 0,
        })
        .unwrap_err();
    assert!(err.to_string().contains("cannot read"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn what_another_letterbox_recorded_in_the_trash_ledger_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trash.toml");
    let entry = |uid: u32| crate::trash::Entry {
        account: "a@b.c".to_string(),
        message_id: format!("<{uid}@b.c>"),
        folder: "Trash".to_string(),
        uid,
        origin: "INBOX".to_string(),
        at: 1,
    };
    // Open in letterbox, and an agent's recording beside it.
    let mut open = crate::trash::Ledger::open(path.clone());
    crate::trash::Ledger::open(path.clone())
        .record(entry(1))
        .unwrap();
    open.record(entry(2)).unwrap();
    let both = crate::trash::Ledger::open(path.clone());
    assert_eq!(both.between(0, 9, None), [entry(1), entry(2)]);
    // And what the other recorded later is found, as letterbox looks.
    crate::trash::Ledger::open(path).record(entry(3)).unwrap();
    assert_eq!(
        open.origin("a@b.c", "Trash", "<3@b.c>", 3).as_deref(),
        Some("INBOX")
    );
}

#[test]
fn letterbox_never_removes_mail_from_trash_for_good() {
    let mut ledger = scratch_ledger();
    let now = jiff::Timestamp::now().as_second();
    let entry = |uid, days| crate::trash::Entry {
        account: "a@b.c".into(),
        message_id: format!("<{uid}@x>"),
        folder: "Trash".into(),
        uid,
        origin: "INBOX".into(),
        at: now - i64::from(days) * 24 * 60 * 60,
    };
    ledger.record(entry(3, 6)).unwrap();
    ledger.record(entry(4, 31)).unwrap();
    // Past a month, a record is let go: only the record.
    let stale: Vec<u32> = ledger
        .stale("a@b.c", now)
        .into_iter()
        .map(|e| e.uid)
        .collect();
    assert_eq!(stale, [4]);

    let mut app = demo();
    press(&mut app, &["d"]);
    app.ledger
        .backdate(jiff::Timestamp::now().as_second() - 40 * 24 * 60 * 60);
    press(&mut app, &["h", "j", "j", "j", "j", "j"]);
    assert_eq!(app.folder_name(), "Trash");
    // Still in Trash, as long as its server keeps it.
    let ada = app
        .shown
        .iter()
        .find(|s| s.from == "Ada Lovelace")
        .expect("never removed for good")
        .summary
        .message_id
        .clone();
    // Its record let go: d there puts it in the Inbox.
    assert!(app.ledger.find("you@example.com", &ada).is_none());
}

#[test]
fn emacs_keys_kill_and_yank() {
    use crate::edit::{self, Buffer, Done};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut buffer = Buffer::from_text("hello");
    let mut prefix = None;
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    assert_eq!(
        edit::handle(&mut buffer, &mut prefix, ctrl('e')),
        Done::Stay
    );
    assert_eq!(buffer.col(), 5);
    edit::handle(&mut buffer, &mut prefix, ctrl('a'));
    assert_eq!(buffer.col(), 0);
    edit::handle(&mut buffer, &mut prefix, ctrl('k'));
    assert_eq!(buffer.text(), "\n");
    edit::handle(&mut buffer, &mut prefix, ctrl('y'));
    assert_eq!(buffer.text(), "hello\n");
}

#[test]
fn a_reply_is_addressed_to_the_sender() {
    let raw = b"From: Ada Lovelace <ada@analytical.example>\r\n\
        To: You <you@example.com>\r\nSubject: Notes on the engine\r\n\
        Message-ID: <1@demo>\r\nReferences: <0@demo>\r\n\
        \r\nHello\r\n"
        .to_vec();
    let message = mail::message(1, raw).unwrap();
    let (text, row) = crate::compose::reply(&message);
    assert!(
        text.starts_with("To: Ada Lovelace <ada@analytical.example>\n"),
        "{text}"
    );
    assert!(
        text.contains("\nSubject: Re: Notes on the engine\n"),
        "{text}"
    );
    assert!(text.contains("> Hello\n"), "{text}");
    assert_eq!(row, 4);
    assert_eq!(crate::compose::reply_subject("éé"), "Re: éé");
    assert_eq!(crate::compose::reply_subject("😀 notes"), "Re: 😀 notes");
    assert_eq!(crate::compose::reply_subject("Re: éé"), "Re: éé");
    let subject = format!("{}é{}", "a".repeat(44), "b".repeat(10));
    let draft = crate::compose::Draft {
        to: vec!["a@b.c".into()],
        cc: Vec::new(),
        subject: subject.clone(),
        attach: Vec::new(),
        body: "hi".into(),
    };
    let raw = crate::compose::bytes("me@example.com", &draft, None, None, &[]);
    assert_eq!(mail::message(1, raw).unwrap().subject, subject);
    let (id, refs) = crate::compose::thread(&message);
    assert_eq!(id.as_deref(), Some("<1@demo>"));
    assert!(refs.unwrap().contains("<0@demo>"));
}

#[test]
fn replying_opens_the_message_and_the_demo_keeps_what_is_sent() {
    let mut app = demo();
    press(&mut app, &["<c-r>"]);
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(
        text.contains("To: Ada Lovelace <ada@analytical.example>"),
        "{text}"
    );
    assert!(text.contains("Subject: Re: Notes on the engine"), "{text}");
    press(&mut app, &["<c-c>", "<c-k>"]);
    assert!(app.compose.is_none(), "unchanged, it goes at once");
    assert!(app.status.contains("not sent"), "{}", app.status);

    press(
        &mut app,
        &[
            "c",
            "ada@analytical.example",
            "<c-n>",
            "<c-n>",
            "Hello",
            "<c-n>",
            "<c-n>",
            "Hi",
            "<c-c>",
            "<c-c>",
        ],
    );
    assert!(app.compose.is_none(), "{}", app.status);
    assert!(app.status.contains("not delivered"), "{}", app.status);
    press(&mut app, &["h", "j", "j"]);
    assert_eq!(app.folder_name(), "Sent");
    assert!(
        app.shown.iter().any(|s| s.subject == "Hello"),
        "{:?}",
        app.shown
            .iter()
            .map(|s| s.subject.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn imap_moves_and_appends() {
    let (account, _) = server();
    let mut imap = Imap::new(account);
    let page = imap.page("INBOX", 0, 10).unwrap();
    let summary = page
        .summaries
        .iter()
        .find(|s| s.from.contains("Rosalind"))
        .unwrap();
    let uid = summary.uid;
    let new_uid = imap
        .relocate("INBOX", uid, "Deleted Items", &summary.message_id)
        .unwrap();
    assert!(
        imap.page("INBOX", 0, 10)
            .unwrap()
            .summaries
            .iter()
            .all(|s| s.uid != uid)
    );
    let trash = imap.page("Deleted Items", 0, 10).unwrap();
    assert!(trash.summaries.iter().any(|s| s.uid == new_uid));
    imap.append(
        "INBOX",
        b"From: me@example.com\r\nSubject: no id\r\n\r\nhi\r\n",
        true,
    )
    .unwrap();
    let bare = imap
        .page("INBOX", 0, 20)
        .unwrap()
        .summaries
        .into_iter()
        .find(|s| s.subject == "no id")
        .unwrap();
    let err = imap
        .relocate("INBOX", bare.uid, "Deleted Items", "")
        .unwrap_err();
    assert!(err.to_string().contains("no Message-ID"), "{err}");
    assert!(
        imap.page("INBOX", 0, 20)
            .unwrap()
            .summaries
            .iter()
            .any(|s| s.uid == bare.uid)
    );
    imap.append("Sent Items", b"From: me@example.com\r\n\r\nhi\r\n", true)
        .unwrap();
    assert_eq!(imap.page("Sent Items", 0, 10).unwrap().total, 1);
    imap.create("Later").unwrap();
    assert!(imap.folders().unwrap().iter().any(|f| f.path == "Later"));
}

pub(super) struct SmtpLog {
    pub(super) auth: String,
    pub(super) data: Vec<u8>,
}

pub(super) fn smtp_server() -> (u16, Arc<Mutex<SmtpLog>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(SmtpLog {
        auth: String::new(),
        data: Vec::new(),
    }));
    let shared = log.clone();
    std::thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let mut tcp = tcp.try_clone().unwrap();
        let mut lines = BufReader::new(tcp.try_clone().unwrap());
        tcp.write_all(b"220 ready\r\n").unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            if lines.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let command = line.trim();
            let upper = command.to_ascii_uppercase();
            if upper.starts_with("EHLO") {
                tcp.write_all(b"250-ready\r\n250-8BITMIME\r\n250 AUTH PLAIN\r\n")
                    .unwrap();
            } else if upper.starts_with("AUTH PLAIN") {
                shared.lock().unwrap().auth = command.to_string();
                tcp.write_all(b"235 ok\r\n").unwrap();
            } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                tcp.write_all(b"250 ok\r\n").unwrap();
            } else if upper == "DATA" {
                tcp.write_all(b"354 go\r\n").unwrap();
                let mut data = Vec::new();
                let mut data_line = String::new();
                loop {
                    data_line.clear();
                    if lines.read_line(&mut data_line).unwrap_or(0) == 0 {
                        return;
                    }
                    if data_line.trim_end() == "." {
                        break;
                    }
                    let mut stored = data_line.as_str();
                    if let Some(rest) = stored.strip_prefix("..") {
                        data.extend(b".");
                        stored = rest;
                    }
                    data.extend(stored.as_bytes());
                }
                shared.lock().unwrap().data = data;
                tcp.write_all(b"250 ok\r\n").unwrap();
            } else if upper.starts_with("QUIT") {
                tcp.write_all(b"221 bye\r\n").unwrap();
                return;
            } else {
                tcp.write_all(b"500 no\r\n").unwrap();
            }
        }
    });
    (port, log)
}

#[test]
fn smtp_sends_what_was_written_and_keeps_it_in_sent() {
    let (port, log) = smtp_server();
    let (mut account, boxes) = server();
    account.smtp_host = Some("127.0.0.1".into());
    account.smtp_port = Some(port);
    let mut imap = Imap::new(account);
    let raw = b"From: me@example.com\r\nTo: ada@example.com\r\nSubject: hi\r\n\
        Date: Tue, 20 Oct 2026 10:00:00 +0000\r\nMessage-ID: <1@example>\r\n\
        MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\
        Content-Transfer-Encoding: 8bit\r\n\r\nHello\r\n.dot\r\n";
    assert_eq!(imap.send(raw).unwrap(), "sent");
    let got = log.lock().unwrap();
    assert!(got.auth.contains("PLAIN"), "{}", got.auth);
    let text = String::from_utf8_lossy(&got.data);
    assert!(text.contains(".dot"), "{text}");
    assert!(!text.contains("..dot"), "{text}");
    drop(got);
    let boxes = boxes.lock().unwrap();
    let sent = boxes.iter().find(|b| b.name == "Sent Items").unwrap();
    assert_eq!(sent.letters.len(), 1);
    assert!(String::from_utf8_lossy(&sent.letters[0].raw).contains(".dot"));
}

#[test]
fn mail_is_not_sent_in_the_clear_off_this_machine() {
    let (mut account, _) = server();
    account.smtp_host = Some("smtp.example.com".into());
    account.security = Security::None;
    let err = crate::smtp::submit(&account, b"From: me@example.com\r\n\r\n\r\n").unwrap_err();
    assert!(err.to_string().contains("unencrypted"), "{err}");
}

#[test]
fn a_body_that_is_not_ascii_is_quoted_when_the_server_cannot_take_it() {
    let raw = b"Content-Transfer-Encoding: 8bit\r\n\r\nCaf\xc3\xa9\r\n";
    let quoted = crate::smtp::for_transfer(raw, false);
    let text = String::from_utf8_lossy(&quoted);
    assert!(text.contains("quoted-printable"), "{text}");
    assert!(text.contains("=C3=A9"), "{text}");
    assert_eq!(crate::smtp::for_transfer(raw, true), raw);
}
