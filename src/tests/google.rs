//! Google's calendars, read from a server in place of Google's.

use crate::agenda::{Coming, upcoming};
use crate::google::{self, Hosts};
use jiff::{Timestamp, tz::TimeZone};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

fn at(s: &str) -> Timestamp {
    s.parse().unwrap()
}

/// What a page of events holds, over the whole of the month it is in.
fn coming(page: &serde_json::Value) -> (Vec<Coming>, usize) {
    let (entries, unread) = google::events(page);
    let (from, to) = (at("2026-11-01T00:00:00Z"), at("2026-12-01T00:00:00Z"));
    (upcoming(&entries, from, to, &TimeZone::UTC), unread)
}

/// A server answering each request by the first of `answers` whose key is in
/// its path, or in its body; the requests it took, each its first line, its
/// headers and its body.
fn google(answers: Vec<(&'static str, String)>) -> (Hosts, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let taken = asked.clone();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let answers = answers.clone();
            let taken = taken.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(&tcp);
                let mut request = String::new();
                let mut length = 0;
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
                let answer = answers
                    .iter()
                    .find(|(key, _)| request.contains(key))
                    .map_or("{}".to_string(), |(_, a)| a.clone());
                taken.lock().unwrap().push(request);
                let _ = write!(
                    &tcp,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{answer}",
                    answer.len()
                );
            });
        }
    });
    let hosts = Hosts {
        auth: format!("{base}/auth"),
        token: format!("{base}/token"),
        api: format!("{base}/api"),
    };
    (hosts, asked)
}

#[test]
fn only_the_calendars_chosen_to_be_seen_are_read_and_by_the_name_given_them() {
    let page = json!({"items": [
        {"id": "mine", "summary": "Mine", "selected": true},
        {"id": "sub@group", "summary": "Long name", "summaryOverride": "Short", "selected": true},
        {"id": "off", "summary": "Hidden by its eye"},
        {"id": "gone", "summary": "Hidden", "selected": true, "hidden": true},
    ]});
    assert_eq!(
        google::shown(&page),
        [
            ("mine".to_string(), "Mine".to_string()),
            ("sub@group".to_string(), "Short".to_string()),
        ]
    );
}

#[test]
fn events_are_times_or_days_and_what_leaves_the_user_free_is_not_among_them() {
    let page = json!({"items": [
        {"summary": "Talk", "start": {"dateTime": "2026-11-10T09:00:00-08:00"},
         "end": {"dateTime": "2026-11-10T10:00:00-08:00"}},
        {"summary": "Away", "start": {"date": "2026-11-11"}, "end": {"date": "2026-11-13"}},
        {"start": {"dateTime": "2026-11-12T09:00:00Z"}, "end": {"dateTime": "2026-11-12T10:00:00Z"}},
        {"summary": "Struck out", "status": "cancelled", "start": {"date": "2026-11-14"},
         "end": {"date": "2026-11-15"}},
        {"summary": "Said no", "start": {"date": "2026-11-16"}, "end": {"date": "2026-11-17"},
         "attendees": [{"self": true, "responseStatus": "declined"}]},
        {"summary": "Said yes", "start": {"date": "2026-11-18"}, "end": {"date": "2026-11-19"},
         "attendees": [{"self": true, "responseStatus": "accepted"},
                       {"responseStatus": "declined"}]},
        {"summary": "No time"},
    ]});
    let (coming, unread) = coming(&page);
    let titles: Vec<&str> = coming.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["Talk", "Away", "Busy", "Said yes"]);
    assert_eq!(unread, 1);
    assert_eq!(coming[0].start, at("2026-11-10T17:00:00Z"));
    assert!(!coming[0].all_day);
    assert!(coming[1].all_day);
    assert_eq!(coming[1].start, at("2026-11-11T00:00:00Z"));
    assert_eq!(coming[1].end, at("2026-11-13T00:00:00Z"));
}

#[test]
fn what_an_event_or_a_calendar_says_has_no_control_character() {
    let page = json!({"items": [{"summary": "Bad\u{1b}]52;c;eA==\u{7} one",
        "start": {"date": "2026-11-11"}, "end": {"date": "2026-11-12"}}]});
    let (coming, _) = coming(&page);
    assert!(!coming[0].title.chars().any(char::is_control));
    let list = json!({"items": [{"id": "a", "summary": "Bad\u{1b}]52 one", "selected": true}]});
    assert!(!google::shown(&list)[0].1.chars().any(char::is_control));
}

#[test]
fn every_calendar_chosen_is_read_over_every_page_with_the_sign_in_kept_out_of_the_address() {
    let (hosts, asked) = google(vec![
        ("/token", json!({"access_token": "now-token"}).to_string()),
        (
            "/users/me/calendarList",
            json!({"items": [
                {"id": "one", "summary": "One", "selected": true},
                {"id": "two#x@group", "summary": "Two", "selected": true},
            ]})
            .to_string(),
        ),
        (
            "pageToken=more",
            json!({"items": [{"summary": "Second page", "start": {"date": "2026-11-12"},
                "end": {"date": "2026-11-13"}}]})
            .to_string(),
        ),
        (
            "/calendars/one/",
            json!({"nextPageToken": "more", "items": [{"summary": "First page",
                "start": {"date": "2026-11-11"}, "end": {"date": "2026-11-12"}}]})
            .to_string(),
        ),
        (
            "/calendars/two%23x%40group/",
            json!({"items": []}).to_string(),
        ),
    ]);
    let feeds = google::read(
        &hosts,
        &app("client", "secret"),
        "later-token",
        at("2026-11-05T00:00:00Z"),
        at("2026-11-20T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(feeds.len(), 2);
    assert_eq!(feeds[0].id, "google:one");
    let one = feeds[0].feed.as_ref().unwrap();
    assert_eq!(feeds[0].name.as_deref(), Some("One"));
    let found = upcoming(
        &one.entries,
        at("2026-11-05T00:00:00Z"),
        at("2026-11-20T00:00:00Z"),
        &TimeZone::UTC,
    );
    let titles: Vec<&str> = found.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["First page", "Second page"]);
    assert!(feeds[1].feed.is_ok());
    let asked = asked.lock().unwrap();
    let token = asked.iter().find(|r| r.contains("/token")).unwrap();
    assert!(token.contains("refresh_token=later-token"), "{token}");
    let first_lines: Vec<&str> = asked.iter().filter_map(|r| r.lines().next()).collect();
    assert!(
        first_lines.iter().all(|l| !l.contains("now-token")),
        "{first_lines:?}"
    );
    assert!(first_lines.iter().any(|l| l.contains("singleEvents=true")));
    let reads = first_lines.iter().filter(|l| !l.contains("/token"));
    assert!(reads.clone().count() >= 4);
    assert!(
        reads.clone().all(|l| l.contains("fields=")),
        "{first_lines:?}"
    );
    let list = asked.iter().find(|r| r.contains("calendarList")).unwrap();
    assert!(
        list.to_lowercase()
            .contains("authorization: bearer now-token"),
        "{list}"
    );
}

#[test]
fn one_calendar_google_will_not_give_leaves_the_others_and_a_sign_in_refused_says_what_to_do() {
    let (hosts, _) = google(vec![
        ("/token", json!({"access_token": "t"}).to_string()),
        (
            "/users/me/calendarList",
            json!({"items": [{"id": "ok", "summary": "Ok", "selected": true},
                             {"id": "no", "summary": "No", "selected": true}]})
            .to_string(),
        ),
        (
            "/calendars/no/",
            json!({"error": {"message": "Not allowed"}}).to_string(),
        ),
        ("/calendars/ok/", json!({"items": []}).to_string()),
    ]);
    let read = |hosts: &Hosts| {
        google::read(
            hosts,
            &app("c", "s"),
            "r",
            at("2026-11-05T00:00:00Z"),
            at("2026-11-06T00:00:00Z"),
        )
    };
    let feeds = read(&hosts).unwrap();
    assert!(feeds[0].feed.is_ok());
    assert!(
        feeds[1]
            .feed
            .as_ref()
            .err()
            .unwrap()
            .contains("Not allowed")
    );
    let (refused, _) = google(vec![(
        "/token",
        json!({"error": "invalid_grant", "error_description": "Token has been expired"})
            .to_string(),
    )]);
    let why = format!("{:#}", read(&refused).unwrap_err());
    assert!(
        why.contains("expired") && why.contains("lbx calendar google"),
        "{why}"
    );
}

/// Where the browser is to come back to, from the sign-in page `page`.
fn way_back(page: &str) -> String {
    let back = query(page, "redirect_uri")
        .replace("%3A", ":")
        .replace("%2F", "/");
    back.strip_prefix("http://").unwrap().to_string()
}

/// The browser coming back to `address`, with `code` for the sign-in `state`.
fn comes_back(address: &str, code: &str, state: &str) {
    let mut tcp = std::net::TcpStream::connect(address).unwrap();
    write!(tcp, "GET /?code={code}&state={state} HTTP/1.1\r\n\r\n").unwrap();
    let mut said = String::new();
    let _ = tcp.read_to_string(&mut said);
}

fn app(client: &str, secret: &str) -> google::App {
    google::App {
        client: client.to_string(),
        secret: secret.to_string(),
    }
}

/// The value of `name` in the query of `url`.
fn query(url: &str, name: &str) -> String {
    url.split(['?', '&'])
        .find_map(|p| p.strip_prefix(&format!("{name}=")))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn signing_in_goes_by_the_browser_and_back_and_asks_only_to_read() {
    let (hosts, asked) = google(vec![(
        "/token",
        json!({"access_token": "t", "refresh_token": "later"}).to_string(),
    )]);
    let pages = Arc::new(Mutex::new(String::new()));
    let seen = pages.clone();
    let later = google::sign_in(&hosts, &app("client id", "shh"), move |page| {
        *seen.lock().unwrap() = page.to_string();
        comes_back(&way_back(page), "the-code", &query(page, "state"));
    })
    .unwrap();
    assert_eq!(later, "later");
    let page = pages.lock().unwrap().clone();
    assert_eq!(query(&page, "client_id"), "client%20id");
    assert_eq!(
        query(&page, "scope"),
        "https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.readonly"
    );
    assert_eq!(query(&page, "access_type"), "offline");
    assert_eq!(query(&page, "code_challenge_method"), "S256");
    assert!(query(&page, "redirect_uri").starts_with("http%3A%2F%2F127.0.0.1%3A"));
    let asked = asked.lock().unwrap();
    assert!(asked[0].contains("code=the-code"), "{}", asked[0]);
    assert!(asked[0].contains("client_secret=shh"), "{}", asked[0]);
    assert!(asked[0].contains("code_verifier="), "{}", asked[0]);
}

#[test]
fn a_sign_in_for_another_is_not_taken_for_this_one() {
    let (hosts, asked) = google(vec![(
        "/token",
        json!({"refresh_token": "later"}).to_string(),
    )]);
    let later = google::sign_in(&hosts, &app("c", "s"), move |page| {
        let (address, state) = (way_back(page), query(page, "state"));
        comes_back(&address, "c-wrong", "wrong");
        comes_back(&address, "c-right", &state);
    })
    .unwrap();
    assert_eq!(later, "later");
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert!(!asked[0].contains("c-wrong"), "{}", asked[0]);
}

#[test]
fn google_is_set_in_the_file_beside_addresses_and_their_secrets_move_with_them() {
    use crate::secret::Keeper;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    assert_eq!(crate::config::google_client(&path).unwrap(), None);
    crate::config::set_google(&path, Keeper::Vault, "client").unwrap();
    assert_eq!(
        crate::config::google_client(&path).unwrap().as_deref(),
        Some("client")
    );
    assert_eq!(crate::config::calendar(&path).unwrap(), Some(Keeper::Vault));
    assert!(
        crate::config::calendar_ids(&path).unwrap().is_empty(),
        "no address"
    );
    let secrets = crate::config::calendar_secrets(&path).unwrap();
    assert!(secrets.contains(&google::SECRET.to_string()));
    assert!(secrets.contains(&google::REFRESH.to_string()));

    // The last address forgotten, Google stays.
    let id = crate::config::next_calendar_id(&[]);
    crate::config::set_calendars(&path, std::slice::from_ref(&id)).unwrap();
    crate::agenda::keep_only(&path, std::slice::from_ref(&id), |_| Ok(())).unwrap();
    assert_eq!(
        crate::config::google_client(&path).unwrap().as_deref(),
        Some("client")
    );
    assert_eq!(crate::config::calendar(&path).unwrap(), Some(Keeper::Vault));

    crate::config::set_calendar(&path, None).unwrap();
    assert_eq!(crate::config::google_client(&path).unwrap(), None);
    assert!(crate::config::calendar_secrets(&path).unwrap().is_empty());
}

#[test]
fn a_file_without_its_secret_is_refused_before_the_browser() {
    let why = format!(
        "{:#}",
        google::App::from_json(r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com"}}"#)
            .err()
            .unwrap()
    );
    assert!(why.contains("no secret"), "{why}");
}

#[test]
fn where_someone_says_they_work_is_not_an_event() {
    let page = json!({"items": [
        {"summary": "Office", "eventType": "workingLocation", "start": {"date": "2026-11-11"},
         "end": {"date": "2026-11-12"}},
        {"summary": "A reminder", "transparency": "transparent", "start": {"date": "2026-11-11"},
         "end": {"date": "2026-11-12"}},
    ]});
    let (coming, _) = coming(&page);
    let titles: Vec<&str> = coming.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["A reminder"], "marked free, it is still shown");
}

#[test]
fn a_calendar_google_will_not_give_is_said_by_its_name() {
    let (hosts, _) = google(vec![
        ("/token", json!({"access_token": "t"}).to_string()),
        (
            "/users/me/calendarList",
            json!({"items": [{"id": "no", "summary": "Shared one", "selected": true}]}).to_string(),
        ),
        (
            "/calendars/no/",
            json!({"error": {"message": "Not allowed"}}).to_string(),
        ),
    ]);
    let feeds = google::read(
        &hosts,
        &app("c", "s"),
        "r",
        at("2026-11-05T00:00:00Z"),
        at("2026-11-06T00:00:00Z"),
    )
    .unwrap();
    assert!(feeds[0].feed.is_err());
    let read = crate::agenda::together(
        feeds,
        at("2026-11-05T00:00:00Z"),
        at("2026-11-06T00:00:00Z"),
        &TimeZone::UTC,
    );
    assert_eq!(
        crate::agenda::faults(&read.calendars),
        ["Shared one: Not allowed"]
    );
}

#[test]
fn a_connection_that_says_nothing_does_not_end_the_sign_in() {
    let (hosts, _) = google(vec![(
        "/token",
        json!({"refresh_token": "later"}).to_string(),
    )]);
    let later = google::sign_in(&hosts, &app("c", "s"), move |page| {
        let address = way_back(page);
        // Open, and closed with nothing said, as a browser may before it asks.
        drop(std::net::TcpStream::connect(&address).unwrap());
        comes_back(&address, "c", &query(page, "state"));
    })
    .unwrap();
    assert_eq!(later, "later");
}

#[test]
fn a_store_that_fails_midway_leaves_the_sign_in_as_it_was() {
    use crate::secret::Keeper;
    use std::cell::RefCell;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_google(&path, Keeper::Vault, "old").unwrap();
    let kept = RefCell::new(vec![
        (google::SECRET.to_string(), "old secret".to_string()),
        (google::REFRESH.to_string(), "old later".to_string()),
    ]);
    let app = app("new", "new secret");
    let tried = RefCell::new(0);
    let done = crate::agenda::keep_sign_in(
        &path,
        Keeper::Vault,
        &app,
        "new later",
        |what| {
            kept.borrow()
                .iter()
                .find(|(k, _)| k == what)
                .map(|(_, v)| v.clone())
        },
        |what, value| {
            *tried.borrow_mut() += 1;
            // The first of the new is kept; the second, and only it, is refused.
            if value == "new later" {
                anyhow::bail!("the store said no");
            }
            let mut kept = kept.borrow_mut();
            kept.retain(|(k, _)| k != what);
            kept.push((what.to_string(), value.to_string()));
            Ok(())
        },
        |what| {
            kept.borrow_mut().retain(|(k, _)| k != what);
            Ok(())
        },
    );
    assert!(done.is_err());
    assert!(*tried.borrow() >= 2);
    let mut kept = kept.into_inner();
    kept.sort();
    assert_eq!(
        kept,
        [
            (google::REFRESH.to_string(), "old later".to_string()),
            (google::SECRET.to_string(), "old secret".to_string()),
        ]
    );
    assert_eq!(
        crate::config::google_client(&path).unwrap().as_deref(),
        Some("old")
    );
}

#[test]
fn forgetting_tries_everything_and_what_could_not_be_forgotten_may_be_tried_again() {
    use crate::secret::Keeper;
    use std::cell::RefCell;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(Keeper::Vault)).unwrap();
    crate::config::set_google(&path, Keeper::Vault, "client").unwrap();
    crate::config::set_calendars(&path, &["ical".to_string()]).unwrap();
    let tried = RefCell::new(Vec::new());
    let done = crate::agenda::forget_all_by(&path, |what| {
        tried.borrow_mut().push(what.to_string());
        if what == "ical" {
            anyhow::bail!("the store said no");
        }
        Ok(())
    });
    assert!(done.is_err());
    assert_eq!(tried.borrow().len(), 3, "the rest are tried all the same");
    assert!(
        crate::config::calendar(&path).unwrap().is_some(),
        "still set, to be tried again"
    );
    crate::agenda::forget_all_by(&path, |_| Ok(())).unwrap();
    assert!(crate::config::calendar(&path).unwrap().is_none());
}

#[test]
fn only_a_sign_in_that_has_died_is_told_to_sign_in_again() {
    let read = |hosts: &Hosts| {
        google::read(
            hosts,
            &app("c", "s"),
            "r",
            at("2026-11-05T00:00:00Z"),
            at("2026-11-06T00:00:00Z"),
        )
    };
    let (other, _) = google(vec![(
        "/token",
        json!({"error": "server_error"}).to_string(),
    )]);
    let why = format!("{:#}", read(&other).unwrap_err());
    assert!(why.contains("server_error"), "{why}");
    assert!(!why.contains("signs in again"), "{why}");
}

#[test]
fn the_app_a_build_comes_with_is_one_or_none_never_a_broken_one() {
    let text = include_str!("../google_app.json");
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    if v["installed"]["client_id"].as_str() != Some("") {
        google::App::from_json(text).unwrap();
    }
}
