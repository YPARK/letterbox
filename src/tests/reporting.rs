//! An agent of your own, started as letterbox opens, and what it reports:
//! shown in letterbox, and only what you choose there done, and learnt.

use super::*;
use crate::report::{Action, Proposal, Report};

/// What an agent would report of the demo's mail: the first of the Inbox
/// thrown away, the message `subject` filed under `label`, and a question.
fn reported(app: &App, subject: &str, label: &str) -> Report {
    let ident = app.accounts[0].ident.clone();
    let s = app.map.snapshot.as_ref().unwrap();
    let place = |subject: &str| {
        let d = s.dots.iter().find(|d| d.subject == subject).unwrap();
        let p = d.places.iter().find(|p| p.folder == "INBOX").unwrap();
        (p.uid, d.from.clone(), d.message_id.clone())
    };
    let first = app.shown[0].subject.clone();
    let (uid, from, message_id) = place(&first);
    let (filed, filed_from, filed_id) = place(subject);
    Report {
        at: 0,
        summary: "Two need an answer this week.\nOne is only to know.".to_string(),
        proposals: vec![
            Proposal {
                account: ident.clone(),
                folder: "INBOX".to_string(),
                uid,
                message_id,
                from,
                subject: first,
                action: Action::Trash,
                why: "a newsletter".to_string(),
                ..Default::default()
            },
            Proposal {
                account: ident,
                folder: "INBOX".to_string(),
                uid: filed,
                message_id: filed_id,
                from: filed_from,
                subject: subject.to_string(),
                action: Action::File,
                to: label.to_string(),
                ..Default::default()
            },
        ],
        questions: vec!["What is due before Friday?".to_string()],
    }
}

/// Shown, and long enough for a key to act on it.
fn shown(app: &mut App, report: Report) {
    app.reports.push(report);
    assert!(app.show_reports());
    let app::Dialog::Report(view) = &mut app.dialog else {
        panic!("not shown");
    };
    view.shown = std::time::Instant::now() - app::GRACE;
}

fn thrown(app: &App) -> (u32, u32) {
    let index = app.index.lock().unwrap();
    (index.filter.thrown, index.filter.kept)
}

#[test]
fn what_an_agent_reported_is_shown_and_only_what_you_choose_is_done_and_learnt() {
    let mut app = demo_map();
    let subject = "Aims page, third pass";
    let label = app
        .map
        .snapshot
        .as_ref()
        .unwrap()
        .model
        .label_vectors
        .values()
        .next()
        .unwrap()
        .0
        .clone();
    let first = app.shown[0].subject.clone();
    let before = thrown(&app);
    let report = reported(&app, subject, &label);

    // Not over what is being written, nor another dialog.
    app.reports.push(report.clone());
    app.dialog = app::Dialog::Help(0);
    assert!(!app.show_reports());
    app.dialog = app::Dialog::Closed;
    assert!(app.show_reports());
    let screen = app::screen(&mut app, 140, 40);
    if std::env::var_os("SHOW_REPORT").is_some() {
        eprintln!("{screen}");
    }
    assert!(screen.contains("Two need an answer"), "{screen}");
    assert!(screen.contains("What is due before Friday?"), "{screen}");
    // A key pressed as it came does nothing.
    press(&mut app, &["<esc>"]);
    assert!(
        matches!(app.dialog, app::Dialog::Report(_)),
        "{}",
        app.status
    );
    app.dialog = app::Dialog::Closed;

    // Left, nothing is done.
    shown(&mut app, report.clone());
    press(&mut app, &["<enter>"]);
    assert!(app.status.contains("nothing chosen"), "{}", app.status);
    press(&mut app, &["x"]);
    assert!(!app.dialog.is_open());
    assert_eq!(app.shown[0].subject, first);
    assert_eq!(thrown(&app), before);

    // A question copied, to paste to the agent.
    shown(&mut app, report.clone());
    press(&mut app, &["j", "j", "c"]);
    assert_eq!(
        app.launch,
        Some(app::Launch::Copy("What is due before Friday?".to_string()))
    );
    app.launch = None;
    app.dialog = app::Dialog::Closed;

    // Both chosen, both done, as you would have done them, and learnt.
    shown(&mut app, report.clone());
    press(&mut app, &["<space>", "<space>", "<enter>"]);
    // How it went on the server, said as it settles, as for any batch.
    assert!(!matches!(app.dialog, app::Dialog::Report(_)));
    assert!(app.status.contains("2 done"), "{}", app.status);
    assert!(app.shown.iter().all(|s| s.subject != first), "trashed");
    let (labels, inbox) = dot_labels(&app, subject);
    assert!(labels.contains(&label) && !inbox, "{labels:?}");
    assert_eq!(thrown(&app), (before.0 + 1, before.1 + 1));
}

/// Tick until the agent started has ended, a few seconds at most.
fn until_ended(app: &mut App) {
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while app.agent.is_some() && std::time::Instant::now() < until {
        app.reports_looked = std::time::Instant::now() - Duration::from_secs(10);
        app.tick();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(app.agent.is_none(), "ended");
}

#[test]
fn t_starts_the_agent_set_and_it_is_said_when_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    assert_eq!(crate::config::agent(&path).unwrap(), None);
    let mut app = demo_map();
    press(&mut app, &["T"]);
    assert!(app.status.contains("no agent is set"), "{}", app.status);

    let script = if cfg!(windows) {
        "echo looked at %LETTERBOX_AGENT%& exit 3"
    } else {
        "echo looked at $LETTERBOX_AGENT; exit 3"
    };
    std::fs::write(&path, format!("[agent]\ncommand = \"{script}\"\n")).unwrap();
    let command = crate::config::agent(&path).unwrap().unwrap();
    assert!(crate::config::load(&path).unwrap().is_empty(), "no account");
    let set = app::AgentSet::beside(command, &path);
    let (log, ran) = (set.log.clone(), set.ran.clone());
    app.agent_set = Some(set);
    press(&mut app, &["T"]);
    assert!(app.status.contains("looking through"), "{}", app.status);
    press(&mut app, &["T"]);
    assert!(app.status.contains("still looking"), "{}", app.status);
    until_ended(&mut app);
    assert!(app.status.contains("failing"), "{}", app.status);
    let said = std::fs::read_to_string(&log).unwrap();
    assert_eq!(said.lines().collect::<Vec<_>>(), ["looked at 1"]);
    assert!(
        std::fs::read_to_string(&ran)
            .unwrap()
            .parse::<i64>()
            .is_ok()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "only you may read what it said");
    }
}

#[test]
fn an_agent_that_could_not_start_has_not_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = demo_map();
    let mut set = app::AgentSet::beside("exit 0".to_string(), &path);
    set.log = dir.path().to_path_buf();
    let ran = set.ran.clone();
    app.agent_set = Some(set);
    press(&mut app, &["T"]);
    assert!(app.status.contains("cannot start"), "{}", app.status);
    assert!(!ran.exists(), "the mail waits for one that does");
    assert_eq!(app.agent_set.as_ref().unwrap().last_ran, 0);
}

#[test]
fn a_quoted_word_reaches_the_shell_as_it_was_written() {
    let out = crate::config::shell("echo \"a  b\"").output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    // cmd's echo keeps the quotes, sh's does not; neither gains a backslash.
    assert_eq!(said.trim().trim_matches('"'), "a  b");
}

#[test]
fn the_agent_starts_on_its_own_only_once_new_mail_came_since_it_last_ran() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let ran = path.with_file_name("agent-ran");
    let mut app = demo_map();
    let set =
        |app: &mut App| app.agent_set = Some(app::AgentSet::beside("exit 0".to_string(), &path));
    // Last ran after all there is: nothing new as the mail is taken in.
    let later = jiff::Timestamp::now().as_second() + 3600;
    std::fs::write(&ran, later.to_string()).unwrap();
    set(&mut app);
    app.agent_mapped(0);
    assert!(app.agent.is_none());
    // Nor later, once every account's mail was taken in: not unasked,
    // mid-work, though new mail comes.
    app.agent_set.as_mut().unwrap().last_ran = 0;
    app.agent_mapped(0);
    assert!(app.agent.is_none(), "{}", app.status);
    // Never ran: the demo's unread mail is new to it, and it starts, once.
    std::fs::remove_file(&ran).unwrap();
    set(&mut app);
    app.agent_mapped(0);
    assert!(app.agent.is_some(), "{}", app.status);
    until_ended(&mut app);
    app.agent_set.as_mut().unwrap().last_ran = 0;
    app.agent_mapped(0);
    assert!(app.agent.is_none(), "once a run of letterbox; T for more");
}

#[test]
fn what_an_agent_reports_is_read_as_letterbox_reads_it_never_as_the_agent_says() {
    use serde_json::json;
    let mut server = crate::mcp::Server::new(&PathBuf::from("/nowhere"), true).unwrap();
    let listed = call(&mut server, "list_messages", json!({ "limit": 1 }));
    let first = &listed["structuredContent"]["messages"][0];
    let uid = first["uid"].as_u64().unwrap();
    let r = call(
        &mut server,
        "report_triage",
        json!({
            "summary": "Quiet day.\u{1b}[2J",
            "questions": ["What did Ada ask?"],
            "proposals": [{ "uid": uid, "action": "trash", "subject": "Something else", "why": "old" }]
        }),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["proposed"], 1);
    let report = &server.reports[0];
    assert_eq!(
        report.proposals[0].subject,
        first["subject"].as_str().unwrap()
    );
    assert_eq!(report.proposals[0].from, first["from"].as_str().unwrap());
    assert!(!report.summary.contains('\u{1b}'), "{:?}", report.summary);
    assert_eq!(report.questions, ["What did Ada ask?"]);
    // Nothing moved by reporting it.
    let again = call(&mut server, "list_messages", json!({ "limit": 1 }));
    assert_eq!(again["structuredContent"]["messages"][0]["uid"], uid);
    let archived = call(
        &mut server,
        "list_messages",
        json!({ "folder": "Archive", "limit": 1 }),
    );
    let archived = archived["structuredContent"]["messages"][0]["uid"]
        .as_u64()
        .unwrap();
    // Never a label made up, nor a message there is not, nor where it is.
    for wrong in [
        json!({ "uid": uid, "action": "file", "to": "Made up" }),
        json!({ "uid": 99999, "action": "trash" }),
        json!({ "uid": uid, "action": "delete" }),
        json!({ "folder": "Archive", "uid": archived, "action": "file", "to": "Archive" }),
    ] {
        let r = call(
            &mut server,
            "report_triage",
            json!({ "summary": "x", "proposals": [wrong] }),
        );
        assert_eq!(r["isError"], true, "{r}");
    }
    assert_eq!(server.reports.len(), 1);
}

#[test]
fn a_report_written_for_letterbox_is_taken_in_by_the_one_that_writes_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity();
    let report = Report {
        summary: "Quiet day.".to_string(),
        ..Default::default()
    };
    index::journal::append(
        dir.path(),
        crate::report::JOURNAL,
        &id.to_public(),
        vec![report.clone()],
    )
    .unwrap();
    let taken = |index: &Index| {
        let mut got: Vec<Report> = Vec::new();
        index
            .take_written(crate::report::JOURNAL, |r| {
                got = r;
                Ok(())
            })
            .unwrap();
        got
    };
    let reader = Index::read_only(dir.path(), &id);
    assert!(taken(&reader).is_empty());
    let index = Index::open_with(dir.path(), &id);
    assert_eq!(taken(&index), [report]);
    assert!(taken(&index).is_empty(), "once");
}

#[test]
fn a_long_report_goes_along_with_the_one_picked_never_out_of_sight() {
    let mut app = demo_map();
    let one = reported(&app, "Aims page, third pass", "Grant").proposals[0].clone();
    let proposals = (0..40)
        .map(|n| Proposal {
            subject: format!("Notice {n}"),
            why: "bulk".to_string(),
            ..one.clone()
        })
        .collect();
    shown(
        &mut app,
        Report {
            summary: "Many.".to_string(),
            proposals,
            questions: vec!["What is left?".to_string()],
            ..Default::default()
        },
    );
    let screen = app::screen(&mut app, 120, 30);
    assert!(
        screen.contains("Notice 0 ") && !screen.contains("Notice 39"),
        "{screen}"
    );
    assert!(screen.contains("↓ "), "{screen}");
    for _ in 0..39 {
        press(&mut app, &["j"]);
    }
    let screen = app::screen(&mut app, 120, 30);
    assert!(
        screen.contains("Notice 39") && !screen.contains("Notice 0 "),
        "{screen}"
    );
    assert!(screen.contains("↑ "), "{screen}");
}

#[test]
fn a_report_of_a_summary_alone_is_shown() {
    let mut app = demo_map();
    shown(
        &mut app,
        Report {
            summary: "Quiet day.".to_string(),
            ..Default::default()
        },
    );
    let screen = app::screen(&mut app, 120, 30);
    assert!(screen.contains("Quiet day."), "{screen}");
    press(&mut app, &["<enter>", "c"]);
    assert!(app.status.contains("nothing chosen"), "{}", app.status);
}
