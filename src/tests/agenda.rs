//! A calendar read from its address, and what is coming up in it.

use crate::agenda::{Coming, Unread, parse, upcoming};
use jiff::{Timestamp, tz::TimeZone};

fn at(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn ics(events: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}END:VCALENDAR\r\n",
        events.replace('\n', "\r\n")
    )
}

fn coming(events: &str, from: &str, to: &str) -> Vec<Coming> {
    let (entries, _) = parse(&ics(events));
    upcoming(&entries, at(from), at(to), &TimeZone::UTC)
}

fn titles(c: &[Coming]) -> Vec<&str> {
    c.iter().map(|c| c.title.as_str()).collect()
}

fn starts(c: &[Coming]) -> Vec<String> {
    c.iter().map(|c| c.start.to_string()).collect()
}

const WEEK: (&str, &str) = ("2026-11-02T00:00:00Z", "2026-11-09T00:00:00Z");

#[test]
fn an_event_is_read_at_the_moment_it_says_in_the_zone_it_names() {
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART;TZID=America/New_York:20261105T140000\nDTEND;TZID=America/New_York:20261105T150000\nSUMMARY:Lab meeting\nEND:VEVENT\n",
        WEEK.0,
        WEEK.1,
    );
    assert_eq!(titles(&c), ["Lab meeting"]);
    assert_eq!(starts(&c), ["2026-11-05T19:00:00Z"]);
    assert!(!c[0].all_day);
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261105T140000Z\nSUMMARY:In UTC\nEND:VEVENT\n",
        WEEK.0,
        WEEK.1,
    );
    assert_eq!(starts(&c), ["2026-11-05T14:00:00Z"]);
}

#[test]
fn a_time_with_no_zone_is_in_the_users() {
    let (entries, _) = parse(&ics(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261105T140000\nSUMMARY:Floating\nEND:VEVENT\n",
    ));
    let zone = TimeZone::fixed(jiff::tz::offset(-8));
    let c = upcoming(&entries, at(WEEK.0), at(WEEK.1), &zone);
    assert_eq!(starts(&c), ["2026-11-05T22:00:00Z"]);
}

#[test]
fn a_day_is_all_of_it_and_one_already_begun_is_still_coming_up() {
    let event = "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261101\nDTEND;VALUE=DATE:20261104\nSUMMARY:Retreat\nEND:VEVENT\n";
    let c = coming(event, "2026-11-03T12:00:00Z", WEEK.1);
    assert_eq!(titles(&c), ["Retreat"], "its last day");
    assert!(c[0].all_day);
    assert!(
        coming(event, "2026-11-04T00:00:00Z", WEEK.1).is_empty(),
        "its end is not in it"
    );
    let timed = "BEGIN:VEVENT\nUID:a\nDTSTART:20261103T120000Z\nDTEND:20261103T130000Z\nSUMMARY:Now\nEND:VEVENT\n";
    assert_eq!(
        coming(timed, "2026-11-03T12:30:00Z", WEEK.1).len(),
        1,
        "going on"
    );
    assert!(
        coming(timed, "2026-11-03T13:00:00Z", WEEK.1).is_empty(),
        "over"
    );
}

#[test]
fn soonest_first_outside_the_window_left_out_and_cancelled_not_shown() {
    let c = coming(
        "BEGIN:VEVENT\nUID:l\nDTSTART:20261106T090000Z\nSUMMARY:Later\nEND:VEVENT\nBEGIN:VEVENT\nUID:s\nDTSTART:20261103T090000Z\nSUMMARY:Sooner\nEND:VEVENT\nBEGIN:VEVENT\nUID:o\nDTSTART:20261201T090000Z\nSUMMARY:Outside\nEND:VEVENT\nBEGIN:VEVENT\nUID:c\nDTSTART:20261104T090000Z\nSTATUS:CANCELLED\nSUMMARY:Cancelled\nEND:VEVENT\n",
        WEEK.0,
        WEEK.1,
    );
    assert_eq!(titles(&c), ["Sooner", "Later"]);
}

#[test]
fn what_it_says_is_read_folded_escaped_and_never_a_control_character() {
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261105T140000Z\nSUMMARY:Lunch\u{5c}, talk\u{5c}; and\n  more \u{5c}n next\u{1b}[31m\nEND:VEVENT\n",
        WEEK.0,
        WEEK.1,
    );
    assert_eq!(titles(&c), ["Lunch, talk; and more next[31m"]);
}

#[test]
fn every_week_on_its_days_over_the_clocks_changing_at_the_same_time_of_day() {
    let event = "BEGIN:VEVENT\nUID:a\nDTSTART;TZID=America/New_York:20261026T100000\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\nSUMMARY:Standup\nEND:VEVENT\n";
    let c = coming(event, "2026-10-28T00:00:00Z", "2026-11-05T00:00:00Z");
    assert_eq!(
        starts(&c),
        [
            "2026-10-28T14:00:00Z", // Wed, EDT
            "2026-11-02T15:00:00Z", // Mon, EST: still 10 in the morning
            "2026-11-04T15:00:00Z",
        ]
    );
}

#[test]
fn a_rule_ends_as_it_says_and_goes_by_its_interval() {
    let start = "DTSTART:20261102T090000Z\nSUMMARY:R\nEND:VEVENT\n";
    let rule = |r: &str| format!("BEGIN:VEVENT\nUID:a\nRRULE:{r}\n{start}");
    let n = |r: &str| coming(&rule(r), "2026-11-02T00:00:00Z", "2026-12-31T00:00:00Z").len();
    assert_eq!(n("FREQ=DAILY;COUNT=3"), 3);
    assert_eq!(
        n("FREQ=DAILY;UNTIL=20261104T090000Z"),
        3,
        "its end within it"
    );
    assert_eq!(n("FREQ=DAILY;UNTIL=20261104"), 3);
    assert_eq!(n("FREQ=WEEKLY;COUNT=20"), 9);
    assert_eq!(n("FREQ=WEEKLY;INTERVAL=2;COUNT=3"), 3);
    let c = coming(
        &rule("FREQ=WEEKLY;INTERVAL=2"),
        "2026-11-03T00:00:00Z",
        "2026-12-01T00:00:00Z",
    );
    assert_eq!(starts(&c), ["2026-11-16T09:00:00Z", "2026-11-30T09:00:00Z"]);
}

#[test]
fn count_counts_from_the_first_though_it_was_long_ago() {
    let event = "BEGIN:VEVENT\nUID:a\nDTSTART:20200106T090000Z\nRRULE:FREQ=WEEKLY;COUNT=2\nSUMMARY:Over\nEND:VEVENT\n";
    assert!(coming(event, WEEK.0, WEEK.1).is_empty());
    let event = "BEGIN:VEVENT\nUID:a\nDTSTART:20100104T090000Z\nRRULE:FREQ=DAILY\nSUMMARY:Daily\nEND:VEVENT\n";
    assert_eq!(
        coming(event, WEEK.0, WEEK.1).len(),
        7,
        "years of it passed over"
    );
}

#[test]
fn a_day_a_month_lacks_is_passed_over() {
    let e = "BEGIN:VEVENT\nUID:a\nDTSTART:20260131T090000Z\nRRULE:FREQ=MONTHLY\nSUMMARY:M\nEND:VEVENT\n";
    let c = coming(e, "2026-11-01T00:00:00Z", "2027-02-01T00:00:00Z");
    assert_eq!(starts(&c), ["2026-12-31T09:00:00Z", "2027-01-31T09:00:00Z"]);
}

#[test]
fn nth_weekday_and_last_weekday_of_a_month() {
    let m = |rule: &str| {
        let e = format!(
            "BEGIN:VEVENT\nUID:a\nDTSTART:20261001T090000Z\nRRULE:{rule}\nSUMMARY:M\nEND:VEVENT\n"
        );
        starts(&coming(&e, "2026-11-01T00:00:00Z", "2026-12-01T00:00:00Z"))
    };
    assert_eq!(m("FREQ=MONTHLY;BYDAY=2TU"), ["2026-11-10T09:00:00Z"]);
    assert_eq!(m("FREQ=MONTHLY;BYDAY=-1FR"), ["2026-11-27T09:00:00Z"]);
    assert_eq!(m("FREQ=MONTHLY;BYMONTHDAY=15"), ["2026-11-15T09:00:00Z"]);
}

#[test]
fn every_year_and_a_birthday_all_day() {
    let e = "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20101105\nRRULE:FREQ=YEARLY\nSUMMARY:Birthday\nEND:VEVENT\n";
    let c = coming(e, WEEK.0, WEEK.1);
    assert_eq!(titles(&c), ["Birthday"]);
    assert!(c[0].all_day);
}

#[test]
fn a_time_struck_out_or_moved_is_not_shown_where_it_was() {
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T090000Z\nRRULE:FREQ=DAILY;COUNT=4\nEXDATE:20261103T090000Z\nSUMMARY:Daily\nEND:VEVENT\nBEGIN:VEVENT\nUID:a\nRECURRENCE-ID:20261104T090000Z\nDTSTART:20261104T170000Z\nSUMMARY:Moved\nEND:VEVENT\nBEGIN:VEVENT\nUID:a\nRECURRENCE-ID:20261105T090000Z\nDTSTART:20261105T090000Z\nSTATUS:CANCELLED\nSUMMARY:Off\nEND:VEVENT\n",
        WEEK.0,
        WEEK.1,
    );
    assert_eq!(titles(&c), ["Daily", "Moved"]);
    assert_eq!(starts(&c), ["2026-11-02T09:00:00Z", "2026-11-04T17:00:00Z"]);
}

#[test]
fn a_rule_not_understood_is_counted_never_shown_wrongly() {
    let (entries, unread) = parse(&ics(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T090000Z\nRRULE:FREQ=MONTHLY;BYDAY=MO,TU;BYSETPOS=-1\nSUMMARY:Odd\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nSUMMARY:No start\nEND:VEVENT\n",
    ));
    assert_eq!(
        unread,
        Unread {
            rules: 1,
            events: 1
        }
    );
    assert!(upcoming(&entries, at(WEEK.0), at(WEEK.1), &TimeZone::UTC).is_empty());
}

#[test]
fn what_is_not_a_calendar_is_nothing_and_nothing_falls_over() {
    for text in [
        "",
        "<html>sign in</html>",
        "BEGIN:VEVENT",
        "END:VEVENT\nBEGIN:VCALENDAR",
        "DTSTART:\nSUMMARY",
    ] {
        let (entries, _) = parse(text);
        assert!(entries.is_empty(), "{text}");
    }
    let e = "BEGIN:VEVENT\nUID:a\nDTSTART:99991231T235959Z\nRRULE:FREQ=DAILY\nSUMMARY:Far\nEND:VEVENT\n";
    assert!(coming(e, WEEK.0, WEEK.1).is_empty());
    let e = "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T090000Z\nRRULE:FREQ=DAILY;INTERVAL=0\nSUMMARY:Zero\nEND:VEVENT\n";
    let (entries, unread) = parse(&ics(e));
    assert_eq!(unread.rules, 1, "not a rule to loop on");
    assert!(upcoming(&entries, at(WEEK.0), at(WEEK.1), &TimeZone::UTC).is_empty());
}

#[test]
fn the_strip_says_today_tomorrow_or_the_day_and_cuts_to_the_width() {
    let zone = TimeZone::fixed(jiff::tz::offset(-8));
    let (entries, _) = parse(&ics(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261104T220000Z\nSUMMARY:Lab meeting\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART;VALUE=DATE:20261105\nSUMMARY:Report due\nEND:VEVENT\nBEGIN:VEVENT\nUID:c\nDTSTART:20261106T180000Z\nSUMMARY:Committee\nEND:VEVENT\n",
    ));
    let now = at("2026-11-04T20:00:00Z");
    let c = upcoming(&entries, now, at("2026-11-11T00:00:00Z"), &zone);
    let parts = crate::agenda::strip(&c, now, &zone);
    assert_eq!(
        parts,
        [
            ("Today 2:00 PM".to_string(), "Lab meeting".to_string()),
            ("Tomorrow".to_string(), "Report due".to_string()),
            ("Fri 10:00 AM".to_string(), "Committee".to_string()),
        ]
    );
    let later = at("2026-11-04T23:30:00Z");
    assert_eq!(
        crate::agenda::strip(&c, later, &zone)[0].1,
        "Report due",
        "what is over goes"
    );
}

#[test]
fn only_a_secure_address_is_asked_and_webcal_is_the_same_thing() {
    assert_eq!(
        crate::agenda::address("webcal://x.example/a.ics").unwrap(),
        "https://x.example/a.ics"
    );
    assert_eq!(
        crate::agenda::address(" https://x.example/a.ics\n").unwrap(),
        "https://x.example/a.ics"
    );
    for wrong in [
        "http://x.example/a.ics",
        "file:///etc/passwd",
        "x.example/a.ics",
        "",
    ] {
        let e = crate::agenda::address(wrong).unwrap_err().to_string();
        assert!(!e.contains("x.example"), "the address is secret: {e}");
    }
}

// ---- In letterbox ----

fn sample() -> Vec<Coming> {
    let now = Timestamp::now();
    let hours = |n: i64| {
        now.checked_add(jiff::SignedDuration::from_hours(n))
            .unwrap()
    };
    vec![
        Coming {
            start: hours(1),
            end: hours(2),
            all_day: false,
            title: "Visitor talk".into(),
        },
        Coming {
            start: hours(50),
            end: hours(51),
            all_day: false,
            title: "Budget review".into(),
        },
    ]
}

#[test]
fn the_strip_is_under_the_top_only_with_something_to_say() {
    let mut app = super::demo();
    let without = crate::app::screen(&mut app, 120, 30);
    assert!(!without.contains("Visitor talk"));
    assert!(
        without.lines().nth(1).unwrap().contains("Inbox"),
        "no line for it: {without}"
    );
    app.agenda = sample();
    let with = crate::app::screen(&mut app, 120, 30);
    let strip = with.lines().nth(1).unwrap();
    assert!(strip.contains("Visitor talk"), "{with}");
    assert!(strip.contains("Budget review"), "{with}");
    assert!(
        with.lines().count() == without.lines().count(),
        "the mail gives up a line"
    );
    assert!(with.lines().nth(2).unwrap().contains("Inbox"), "{with}");
}

#[test]
fn a_strip_too_narrow_says_whole_events_and_nothing_past_the_edge() {
    use unicode_width::UnicodeWidthStr;
    let mut app = super::demo();
    app.agenda = sample();
    let screen = crate::app::screen(&mut app, 34, 30);
    let strip = screen.lines().nth(1).unwrap();
    assert!(strip.width() <= 34, "{strip}");
    assert!(strip.contains("Visitor talk"), "{strip}");
    assert!(!strip.contains("Budget review"), "{strip}");
}

#[test]
fn a_calendar_is_set_in_the_file_and_gone_as_it_is_removed() {
    use crate::secret::Keeper;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    assert_eq!(crate::config::calendar(&path).unwrap(), None);
    std::fs::write(&path, "# mine\n[agent]\ncommand = \"x\"\n").unwrap();
    crate::config::set_calendar(&path, Some(Keeper::Vault)).unwrap();
    assert_eq!(crate::config::calendar(&path).unwrap(), Some(Keeper::Vault));
    assert!(
        std::fs::read_to_string(&path).unwrap().contains("# mine"),
        "kept"
    );
    assert_eq!(crate::config::agent(&path).unwrap().as_deref(), Some("x"));
    crate::config::set_calendar(&path, None).unwrap();
    assert_eq!(crate::config::calendar(&path).unwrap(), None);
}

#[test]
fn an_agent_is_offered_what_is_coming_up_and_told_when_none_is_set() {
    use serde_json::json;
    let mut server = super::demo_server(false);
    let tools = server
        .answer(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"upcoming_events"), "{names:?}");
    let r = super::call(&mut server, "upcoming_events", json!({ "days": 7 }));
    assert_eq!(r["isError"], false, "{r}");
    let events = r["structuredContent"]["events"].as_array().unwrap();
    assert!(!events.is_empty(), "the demo's: {r}");
    assert!(
        events[0]["start"].as_str().is_some() && events[0]["title"].as_str().is_some(),
        "{r}"
    );
    assert!(
        !r.to_string().contains("https://"),
        "never the address: {r}"
    );
}

#[test]
fn a_calendar_that_cannot_be_read_is_said_along_the_top_when_there_is_nothing_else() {
    let mut app = super::demo();
    app.agenda_fault = Some("the calendar could not be reached".into());
    let screen = crate::app::screen(&mut app, 120, 30);
    assert!(
        screen
            .lines()
            .nth(1)
            .unwrap()
            .contains("could not be reached"),
        "{screen}"
    );
    app.agenda = sample();
    let screen = crate::app::screen(&mut app, 120, 30);
    assert!(
        !screen.contains("could not be reached"),
        "what it read last: {screen}"
    );
}

// ---- As calendars really write them ----

#[test]
fn a_reminder_inside_an_event_is_not_taken_for_it() {
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261105T140000Z\nDTEND:20261105T150000Z\nSUMMARY:Real title\nBEGIN:VALARM\nUID:alarm\nACTION:EMAIL\nSUMMARY:Alarm notification\nDURATION:PT15M\nEND:VALARM\nEND:VEVENT\n",
        WEEK.0,
        WEEK.1,
    );
    assert_eq!(titles(&c), ["Real title"]);
    assert_eq!(c[0].end, at("2026-11-05T15:00:00Z"));
}

#[test]
fn a_length_too_long_to_be_is_cut_not_fallen_over() {
    for event in [
        "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261105\nDURATION:P99999999D\nSUMMARY:Long\nEND:VEVENT\n",
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261105T140000Z\nDURATION:PT999999999999999S\nSUMMARY:Long\nEND:VEVENT\n",
    ] {
        assert_eq!(titles(&coming(event, WEEK.0, WEEK.1)), ["Long"]);
    }
}

#[test]
fn weeks_are_counted_from_the_day_the_rule_says_they_begin() {
    let e = "BEGIN:VEVENT\nUID:a\nDTSTART:20261101T090000Z\nRRULE:FREQ=WEEKLY;INTERVAL=2;WKST=SU;BYDAY=SU,TU\nSUMMARY:W\nEND:VEVENT\n";
    let c = coming(e, "2026-11-01T00:00:00Z", "2026-11-20T00:00:00Z");
    assert_eq!(
        starts(&c),
        [
            "2026-11-01T09:00:00Z",
            "2026-11-03T09:00:00Z",
            "2026-11-15T09:00:00Z",
            "2026-11-17T09:00:00Z"
        ]
    );
    let e = "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T090000Z\nRRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,SU\nSUMMARY:W\nEND:VEVENT\n";
    let c = coming(e, "2026-11-02T00:00:00Z", "2026-11-17T00:00:00Z");
    assert_eq!(
        starts(&c),
        [
            "2026-11-02T09:00:00Z",
            "2026-11-08T09:00:00Z",
            "2026-11-16T09:00:00Z"
        ]
    );
}

#[test]
fn parts_of_a_rule_that_would_be_ignored_make_it_not_understood() {
    for rule in [
        "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13",
        "FREQ=DAILY;BYMONTHDAY=1",
        "FREQ=WEEKLY;BYMONTHDAY=1",
        "FREQ=WEEKLY;BYDAY=1MO",
        "FREQ=DAILY;BYDAY=2TU",
        "FREQ=WEEKLY;WKST=TU",
    ] {
        let e = format!(
            "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T090000Z\nRRULE:{rule}\nSUMMARY:R\nEND:VEVENT\n"
        );
        let (entries, unread) = parse(&ics(&e));
        assert_eq!(unread.rules, 1, "{rule}");
        assert!(entries.is_empty(), "{rule}");
    }
}

#[test]
fn a_zone_not_known_is_left_out_and_counted_a_colon_in_its_name_is_read() {
    let (entries, unread) = parse(&ics(
        "BEGIN:VEVENT\nUID:a\nDTSTART;TZID=Pacific Standard Time:20261105T150000\nSUMMARY:Unknown\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART;TZID=\"(UTC-05:00) Eastern\":20261105T150000\nSUMMARY:Quoted\nEND:VEVENT\nBEGIN:VEVENT\nUID:c\nDTSTART;TZID=\"America/New_York\":20261105T140000\nSUMMARY:Known\nEND:VEVENT\n",
    ));
    assert_eq!(unread.events, 2);
    let c = upcoming(&entries, at(WEEK.0), at(WEEK.1), &TimeZone::UTC);
    assert_eq!(titles(&c), ["Known"]);
    assert_eq!(starts(&c), ["2026-11-05T19:00:00Z"]);
}

#[test]
fn what_began_already_is_said_to_be_going_on_not_given_a_false_time_today() {
    let zone = TimeZone::UTC;
    let (entries, _) = parse(&ics(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T140000Z\nDTEND:20261104T170000Z\nSUMMARY:Conference\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART;VALUE=DATE:20261102\nDTEND;VALUE=DATE:20261105\nSUMMARY:Retreat\nEND:VEVENT\n",
    ));
    let now = at("2026-11-03T09:00:00Z");
    let c = upcoming(&entries, now, at("2026-11-09T00:00:00Z"), &zone);
    let parts = crate::agenda::strip(&c, now, &zone);
    assert!(
        parts.contains(&("Now".to_string(), "Conference".to_string())),
        "{parts:?}"
    );
    assert!(
        parts.contains(&("Today".to_string(), "Retreat".to_string())),
        "{parts:?}"
    );
}

#[test]
fn an_agent_is_given_times_in_the_users_zone_and_days_as_days() {
    let c = [
        Coming {
            start: at("2026-11-05T19:00:00Z"),
            end: at("2026-11-05T20:00:00Z"),
            all_day: false,
            title: "T".into(),
        },
        Coming {
            start: at("2026-11-06T08:00:00Z"),
            end: at("2026-11-07T08:00:00Z"),
            all_day: true,
            title: "D".into(),
        },
    ];
    let zone = TimeZone::fixed(jiff::tz::offset(-8));
    let json = crate::agenda::for_agent(&c, &zone);
    assert_eq!(json[0]["start"], "2026-11-05T11:00:00-08:00");
    assert_eq!(json[0]["end"], "2026-11-05T12:00:00-08:00");
    assert_eq!(json[1]["start"], "2026-11-06");
    assert_eq!(json[1]["end"], "2026-11-06", "the last day it is on");
}

#[test]
fn the_demo_never_reads_the_users_own_calendar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(&path, "[calendar]\nkeep = \"keyring\"\n").unwrap();
    let mut app = crate::app::App::new(Vec::new(), path);
    app.as_demo();
    let shown = app.agenda.clone();
    app.tick();
    assert_eq!(app.agenda, shown);
    assert_eq!(app.pending(), 0, "nothing asked");
}

#[test]
fn a_calendar_removed_is_gone_from_the_strip_as_it_is_next_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = crate::app::App::new(Vec::new(), dir.path().join("accounts.toml"));
    app.agenda = sample();
    app.tick();
    assert!(app.agenda.is_empty());
    assert_eq!(app.pending(), 0);
}

#[test]
fn a_rule_that_finds_nothing_stops_at_the_window_and_a_day_no_month_has_is_not_understood() {
    let (_, unread) = parse(&ics(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T090000Z\nRRULE:FREQ=MONTHLY;BYMONTHDAY=32\nSUMMARY:R\nEND:VEVENT\n",
    ));
    assert_eq!(unread.rules, 1);
    let e = "BEGIN:VEVENT\nUID:a\nDTSTART:20261102T090000Z\nRRULE:FREQ=YEARLY;BYMONTHDAY=31\nSUMMARY:R\nEND:VEVENT\n";
    let begun = std::time::Instant::now();
    assert!(coming(e, WEEK.0, WEEK.1).is_empty());
    assert!(begun.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn the_calendars_address_moves_with_the_other_secrets() {
    use crate::secret::Keeper;
    static KEPT: std::sync::Mutex<Vec<(Keeper, String)>> = std::sync::Mutex::new(Vec::new());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(Keeper::Keyring)).unwrap();
    let said = crate::setup::move_secrets(
        &path,
        &dir.path().join("map"),
        Keeper::Vault,
        |_, host, user| Ok(format!("{host}/{user}")),
        |k, _, _, s| {
            KEPT.lock().unwrap().push((k, s.to_string()));
            Ok(())
        },
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(crate::config::calendar(&path).unwrap(), Some(Keeper::Vault));
    assert!(said.iter().any(|l| l.contains("calendar")), "{said:?}");
    assert_eq!(KEPT.lock().unwrap().len(), 1);
}

#[test]
fn a_week_or_more_away_is_said_with_its_date_as_a_weekday_alone_would_mislead() {
    let now = "2026-11-04T12:00:00Z";
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261110\nSUMMARY:Six days\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART;VALUE=DATE:20261111\nSUMMARY:Seven days\nEND:VEVENT\n",
        now,
        "2026-11-20T00:00:00Z",
    );
    let parts = crate::agenda::strip(&c, at(now), &TimeZone::UTC);
    assert_eq!(parts[0].0, "Tue");
    assert_eq!(parts[1].0, "Wed Nov 11");
}

#[test]
fn the_strip_goes_on_to_a_second_row_and_no_third() {
    use unicode_width::UnicodeWidthStr;
    let mut app = super::demo();
    app.agenda = sample();
    let mut third = app.agenda[1].clone();
    third.title = "Third thing".into();
    app.agenda.push(third);
    let screen = crate::app::screen(&mut app, 34, 30);
    let rows: Vec<&str> = screen.lines().collect();
    assert!(rows[1].contains("Visitor talk"), "{screen}");
    assert!(rows[2].contains("Budget review"), "{screen}");
    assert!(rows[1].width() <= 34 && rows[2].width() <= 34, "{screen}");
    assert!(!screen.contains("Third thing"), "{screen}");
}

#[test]
fn a_short_window_keeps_the_strip_to_one_row() {
    let mut app = super::demo();
    app.agenda = sample();
    let screen = crate::app::screen(&mut app, 34, 10);
    assert!(screen.contains("Visitor talk"), "{screen}");
    assert!(!screen.contains("Budget review"), "{screen}");
}

#[test]
fn the_calendar_key_lists_what_is_coming_in_a_popup_and_esc_closes_it() {
    let mut app = super::demo();
    app.agenda = sample();
    super::reading::press(&mut app, &["C"]);
    let open = crate::app::screen(&mut app, 100, 30);
    assert!(open.contains("Calendar"), "{open}");
    let rows: Vec<&str> = open.lines().skip(3).collect();
    assert!(
        rows.iter().any(|r| r.contains("Visitor talk")),
        "over the mail: {open}"
    );
    assert!(rows.iter().any(|r| r.contains("Budget")), "{open}");
    super::reading::press(&mut app, &["<esc>"]);
    assert!(!app.dialog.is_open());
}

#[test]
fn a_long_calendar_scrolls_and_stays_inside_a_narrow_window() {
    use unicode_width::UnicodeWidthStr;
    let mut app = super::demo();
    let first = sample().remove(0);
    app.agenda = (0..40)
        .map(|i| Coming {
            title: format!("E{i}: a rather long title to be cut short"),
            ..first.clone()
        })
        .collect();
    super::reading::press(&mut app, &["C"]);
    let top = crate::app::screen(&mut app, 34, 14);
    assert!(top.lines().all(|l| l.width() <= 34), "{top}");
    assert!(top.contains("E0:"), "{top}");
    super::reading::press(&mut app, &["j", "j"]);
    let down = crate::app::screen(&mut app, 34, 14);
    assert!(!down.contains("E0:"), "{down}");
    assert!(app.dialog.is_open());
}

#[test]
fn the_popup_says_when_there_is_no_calendar_or_it_could_not_be_read() {
    let mut app = super::demo();
    super::reading::press(&mut app, &["C"]);
    let none = crate::app::screen(&mut app, 100, 30);
    assert!(none.contains("a adds one"), "{none}");
    super::reading::press(&mut app, &["<esc>"]);
    app.agenda_fault = Some("the address was refused".into());
    super::reading::press(&mut app, &["C"]);
    let fault = crate::app::screen(&mut app, 100, 30);
    assert!(fault.contains("the address was refused"), "{fault}");
}

#[test]
fn the_list_names_each_day_once_and_says_all_day() {
    let zone = TimeZone::UTC;
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261110T090000Z\nDTEND:20261110T100000Z\nSUMMARY:Morning\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART:20261110T140000Z\nDTEND:20261110T150000Z\nSUMMARY:Afternoon\nEND:VEVENT\nBEGIN:VEVENT\nUID:c\nDTSTART;VALUE=DATE:20261111\nSUMMARY:Holiday\nEND:VEVENT\n",
        "2026-11-04T12:00:00Z",
        "2026-11-20T00:00:00Z",
    );
    let rows = crate::agenda::listing(&c, at("2026-11-04T12:00:00Z"), &zone);
    let want = |d: &str, w: &str, t: &str| (d.to_string(), w.to_string(), t.to_string());
    assert_eq!(
        rows,
        vec![
            want("Tue", "9:00 AM", "Morning"),
            want("", "2:00 PM", "Afternoon"),
            want("Wed Nov 11", "all day", "Holiday"),
        ]
    );
}

// ---- More than one ----

/// One more calendar set in the file at `path`: what it is kept under.
fn one_more(path: &std::path::Path) -> String {
    let mut ids = crate::config::calendar_ids(path).unwrap();
    let id = crate::config::next_calendar_id(&ids);
    ids.push(id.clone());
    crate::config::set_calendars(path, &ids).unwrap();
    id
}

#[test]
fn calendars_are_added_one_by_one_and_all_go_as_they_are_removed() {
    use crate::secret::Keeper;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    assert!(crate::config::calendar_ids(&path).unwrap().is_empty());
    crate::config::set_calendar(&path, Some(Keeper::Vault)).unwrap();
    assert_eq!(crate::config::calendar_ids(&path).unwrap(), ["ical"]);
    let second = one_more(&path);
    let third = one_more(&path);
    assert_ne!(second, third);
    assert_eq!(
        crate::config::calendar_ids(&path).unwrap(),
        ["ical".to_string(), second, third]
    );
    crate::config::set_calendar(&path, Some(Keeper::Keyring)).unwrap();
    assert_eq!(crate::config::calendar_ids(&path).unwrap().len(), 3, "kept");
    crate::config::set_calendar(&path, None).unwrap();
    assert!(crate::config::calendar_ids(&path).unwrap().is_empty());
}

#[test]
fn a_calendar_is_called_what_it_says_it_is() {
    assert_eq!(
        crate::agenda::named("BEGIN:VCALENDAR\r\nX-WR-CALNAME:Some\\, Team\r\nEND:VCALENDAR\r\n")
            .as_deref(),
        Some("Some, Team")
    );
    assert_eq!(
        crate::agenda::named("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n"),
        None
    );
}

fn feed(name: &str, events: &str) -> crate::agenda::Feed {
    let (entries, unread) = parse(&ics(events));
    crate::agenda::Feed {
        entries,
        unread,
        name: Some(name.to_string()),
    }
}

#[test]
fn calendars_are_read_together_soonest_first_each_event_once() {
    let a = "BEGIN:VEVENT\nUID:a\nDTSTART:20261110T090000Z\nDTEND:20261110T100000Z\nSUMMARY:Shared\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART:20261112T090000Z\nDTEND:20261112T100000Z\nSUMMARY:Later\nEND:VEVENT\n";
    let b = "BEGIN:VEVENT\nUID:c\nDTSTART:20261110T090000Z\nDTEND:20261110T100000Z\nSUMMARY:Shared\nEND:VEVENT\nBEGIN:VEVENT\nUID:d\nDTSTART:20261111T090000Z\nDTEND:20261111T100000Z\nSUMMARY:Between\nEND:VEVENT\n";
    let read = crate::agenda::together(
        vec![
            crate::agenda::Reading::of("x", Ok(feed("One", a))),
            crate::agenda::Reading::of("y", Ok(feed("Two", b))),
        ],
        at("2026-11-04T12:00:00Z"),
        at("2026-11-20T00:00:00Z"),
        &TimeZone::UTC,
    );
    assert_eq!(titles(&read.coming), ["Shared", "Between", "Later"]);
    assert_eq!(crate::agenda::names(&read.calendars), ["One", "Two"]);
    assert!(crate::agenda::faults(&read.calendars).is_empty());
}

#[test]
fn a_calendar_that_cannot_be_read_leaves_the_others_and_is_said() {
    let a = "BEGIN:VEVENT\nUID:a\nDTSTART:20261110T090000Z\nDTEND:20261110T100000Z\nSUMMARY:Still here\nEND:VEVENT\n";
    let read = crate::agenda::together(
        vec![
            crate::agenda::Reading::of("x", Err("the calendar could not be reached".to_string())),
            crate::agenda::Reading::of("y", Ok(feed("One", a))),
        ],
        at("2026-11-04T12:00:00Z"),
        at("2026-11-20T00:00:00Z"),
        &TimeZone::UTC,
    );
    assert_eq!(titles(&read.coming), ["Still here"]);
    assert_eq!(crate::agenda::names(&read.calendars), ["One"]);
    assert_eq!(
        crate::agenda::faults(&read.calendars),
        ["calendar 1: the calendar could not be reached"]
    );
}

#[test]
fn every_calendars_address_moves_with_the_other_secrets() {
    use crate::secret::Keeper;
    static KEPT: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(Keeper::Keyring)).unwrap();
    let second = one_more(&path);
    crate::setup::move_secrets(
        &path,
        &dir.path().join("map"),
        Keeper::Vault,
        |_, host, user| Ok(format!("{host}/{user}")),
        |_, _, user, _| {
            KEPT.lock().unwrap().push(user.to_string());
            Ok(())
        },
        |_, _, _| Ok(()),
    )
    .unwrap();
    let kept = KEPT.lock().unwrap().clone();
    assert!(
        kept.contains(&"ical".to_string()) && kept.contains(&second),
        "{kept:?}"
    );
}

#[test]
fn a_calendar_is_added_from_the_popup_its_address_never_on_the_screen() {
    let mut app = super::demo();
    app.agenda = sample();
    super::reading::press(&mut app, &["C"]);
    assert!(crate::app::screen(&mut app, 100, 30).contains("a adds"));
    super::reading::press(&mut app, &["a"]);
    app.paste("https://calendar.example/secret-part/basic.ics");
    let asking = crate::app::screen(&mut app, 100, 30);
    assert!(asking.contains("Add a calendar"), "{asking}");
    assert!(!asking.contains("secret-part"), "{asking}");
    assert!(!asking.contains("https"), "{asking}");
    super::reading::press(&mut app, &["<esc>"]);
    assert!(
        matches!(app.dialog, crate::app::Dialog::Calendar(_)),
        "back"
    );
}

#[test]
fn an_address_refused_is_said_without_repeating_it_and_the_demo_keeps_none() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = super::demo();
    app.config = dir.path().join("accounts.toml");
    super::reading::press(&mut app, &["C", "a"]);
    app.paste("not-an-address-secret");
    super::reading::press(&mut app, &["<enter>"]);
    assert!(app.status.contains("https://"), "{}", app.status);
    assert!(!app.status.contains("not-an-address"), "{}", app.status);
    assert!(app.dialog.is_open(), "to try again");
    let screen = crate::app::screen(&mut app, 120, 30);
    assert!(screen.contains("begins with https://"), "seen: {screen}");

    let mut demo = super::demo();
    demo.as_demo();
    demo.config = dir.path().join("accounts.toml");
    super::reading::press(&mut demo, &["C", "a"]);
    demo.paste("https://calendar.example/x.ics");
    super::reading::press(&mut demo, &["<enter>"]);
    assert!(demo.status.contains("demo"), "{}", demo.status);
    assert!(
        crate::config::calendar_ids(&demo.config)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_popup_names_the_calendars_read_and_what_could_not_be() {
    let mut app = super::demo();
    app.agenda = sample();
    app.agenda_calendars = vec![calendar("a", "Some Team"), calendar("b", "Other Cal")];
    app.agenda_fault = Some("calendar 3: it could not be reached".into());
    super::reading::press(&mut app, &["C"]);
    let screen = crate::app::screen(&mut app, 100, 30);
    assert!(screen.contains("Some Team, Other Cal"), "{screen}");
    assert!(screen.contains("calendar 3"), "{screen}");
    assert!(screen.contains("Visitor talk"), "{screen}");
}

// ---- Ticked, to keep ----

fn calendar(id: &str, name: &str) -> crate::agenda::CalendarRead {
    crate::agenda::CalendarRead {
        id: id.to_string(),
        name: Some(name.to_string()),
        fault: None,
        coming: Vec::new(),
    }
}

/// `gone` taken from those set in a file, addresses in memory: the ids left,
/// and each kept address by id.
fn left_after(extra: usize, gone: &[&str]) -> (Vec<String>, Vec<(String, String)>) {
    use crate::secret::Keeper;
    use std::cell::RefCell;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(Keeper::Vault)).unwrap();
    for _ in 0..extra {
        one_more(&path);
    }
    let kept = RefCell::new(
        crate::config::calendar_ids(&path)
            .unwrap()
            .into_iter()
            .map(|id| (id.clone(), format!("address of {id}")))
            .collect::<Vec<_>>(),
    );
    let gone: Vec<String> = gone.iter().map(|g| g.to_string()).collect();
    crate::agenda::keep_only(&path, &gone, |id| {
        kept.borrow_mut().retain(|(k, _)| k != id);
        Ok(())
    })
    .unwrap();
    let mut kept = kept.into_inner();
    kept.sort();
    (crate::config::calendar_ids(&path).unwrap(), kept)
}

#[test]
fn one_beside_the_first_is_forgotten_and_the_rest_stay() {
    let (ids, kept) = left_after(2, &["ical-2"]);
    assert_eq!(ids, ["ical", "ical-3"]);
    assert_eq!(
        kept,
        [
            ("ical".to_string(), "address of ical".to_string()),
            ("ical-3".to_string(), "address of ical-3".to_string()),
        ]
    );
}

#[test]
fn the_first_forgotten_the_others_stay_each_with_its_own_address() {
    let (ids, kept) = left_after(2, &["ical"]);
    assert_eq!(ids, ["ical-2", "ical-3"]);
    assert_eq!(
        kept,
        [
            ("ical-2".to_string(), "address of ical-2".to_string()),
            ("ical-3".to_string(), "address of ical-3".to_string()),
        ]
    );
}

#[test]
fn all_forgotten_none_is_set() {
    let (ids, kept) = left_after(1, &["ical", "ical-2"]);
    assert!(ids.is_empty());
    assert!(kept.is_empty());
}

#[test]
fn calendars_are_ticked_in_a_menu_and_esc_keeps_them_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(crate::secret::Keeper::Vault)).unwrap();
    one_more(&path);
    let mut app = super::demo();
    app.read_no_calendar_yet();
    app.config = path.clone();
    let ids = crate::config::calendar_ids(&path).unwrap();
    app.agenda_calendars = vec![
        calendar(&ids[0], "Some Team"),
        calendar(&ids[1], "Other Cal"),
    ];
    super::reading::press(&mut app, &["C"]);
    assert!(crate::app::screen(&mut app, 100, 30).contains("r removes"));
    super::reading::press(&mut app, &["r"]);
    let ticked = crate::app::screen(&mut app, 100, 30);
    assert!(ticked.contains("[x] Some Team"), "{ticked}");
    assert!(ticked.contains("[x] Other Cal"), "{ticked}");
    super::reading::press(&mut app, &["j", " "]);
    let one = crate::app::screen(&mut app, 100, 30);
    assert!(one.contains("[x] Some Team"), "{one}");
    assert!(one.contains("[ ] Other Cal"), "{one}");
    super::reading::press(&mut app, &[" "]);
    assert!(crate::app::screen(&mut app, 100, 30).contains("[x] Other Cal"));
    super::reading::press(&mut app, &[" ", "<esc>"]);
    assert!(
        matches!(app.dialog, crate::app::Dialog::Calendar(_)),
        "back"
    );
    assert_eq!(crate::config::calendar_ids(&path).unwrap().len(), 2);
}

#[test]
fn the_demo_forgets_no_calendar_and_nothing_unticked_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(crate::secret::Keeper::Vault)).unwrap();
    let mut app = super::demo();
    app.as_demo();
    app.config = path.clone();
    super::reading::press(&mut app, &["C", "r", " ", "<enter>"]);
    assert!(app.status.contains("demo"), "{}", app.status);
    assert_eq!(crate::config::calendar_ids(&path).unwrap().len(), 1);
    app.demo = false;
    app.read_no_calendar_yet();
    super::reading::press(&mut app, &["r", "<enter>"]);
    assert!(matches!(app.dialog, crate::app::Dialog::Calendar(_)));
    assert_eq!(crate::config::calendar_ids(&path).unwrap().len(), 1);
}

#[test]
fn a_calendar_is_called_what_it_says_or_by_its_place() {
    let read = crate::agenda::together(
        vec![
            crate::agenda::Reading::of("x", Err("no".to_string())),
            crate::agenda::Reading::of("y", Ok(feed("One", ""))),
        ],
        at("2026-11-04T12:00:00Z"),
        at("2026-11-20T00:00:00Z"),
        &TimeZone::UTC,
    );
    assert_eq!(read.calendars[0].called(1), "calendar 1 (not read)");
    assert_eq!(read.calendars[1].called(2), "One");
    assert_eq!(read.calendars[0].id, "x");
}

// ---- As a table ----

#[test]
fn days_hold_what_is_on_them_an_all_day_event_on_each_and_a_timed_one_on_its_own() {
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261105\nDTEND;VALUE=DATE:20261107\nSUMMARY:Away\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART:20261106T140000Z\nDTEND:20261106T150000Z\nSUMMARY:Talk\nEND:VEVENT\n",
        "2026-11-05T01:00:00Z",
        "2026-11-20T00:00:00Z",
    );
    let first: jiff::civil::Date = "2026-11-05".parse().unwrap();
    let days = crate::agenda::on_each_day(&c, at("2026-11-05T01:00:00Z"), first, 3, &TimeZone::UTC);
    let on = |i: usize| -> Vec<&str> { days[i].iter().map(|(_, t)| t.as_str()).collect() };
    assert_eq!(days.len(), 3);
    assert_eq!(on(0), ["Away"]);
    assert_eq!(on(1), ["Away", "Talk"]);
    assert!(on(2).is_empty(), "the day it ends is not one it is on");
    assert_eq!(days[1][0].0, "");
    assert_eq!(days[1][1].0, "2:00p");
}

#[test]
fn something_begun_before_the_days_shown_is_on_the_first_of_them() {
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261104T230000Z\nDTEND:20261105T030000Z\nSUMMARY:Late\nEND:VEVENT\n",
        "2026-11-05T01:00:00Z",
        "2026-11-20T00:00:00Z",
    );
    let first: jiff::civil::Date = "2026-11-05".parse().unwrap();
    let days = crate::agenda::on_each_day(&c, at("2026-11-05T01:00:00Z"), first, 2, &TimeZone::UTC);
    assert_eq!(days[0].len(), 1);
    assert!(days[1].is_empty());
}

#[test]
fn tab_goes_from_the_week_to_the_month_to_the_list_and_round() {
    let mut app = super::demo();
    app.agenda = sample();
    super::reading::press(&mut app, &["C"]);
    let title = |app: &mut crate::app::App| crate::app::screen(app, 120, 40);
    assert!(title(&mut app).contains("Calendar: Week"));
    super::reading::press(&mut app, &["<tab>"]);
    assert!(title(&mut app).contains("Calendar: Month"));
    super::reading::press(&mut app, &["<tab>"]);
    assert!(title(&mut app).contains("Calendar: List"));
    super::reading::press(&mut app, &["<tab>"]);
    assert!(title(&mut app).contains("Calendar: Week"));
    assert!(app.dialog.is_open());
}

#[test]
fn the_week_is_seven_columns_from_today_with_what_is_on_each() {
    let mut app = super::demo();
    app.agenda = sample();
    super::reading::press(&mut app, &["C"]);
    let screen = crate::app::screen(&mut app, 120, 40);
    let today = Timestamp::now().to_zoned(TimeZone::system());
    let head = today.strftime("%a %-d").to_string();
    let row = screen.lines().find(|l| l.contains(&head)).expect("today");
    // Eight rules, and the popup's own two edges.
    assert_eq!(row.matches('│').count(), 10, "{row}");
    assert!(screen.contains("Visitor"), "{screen}");
    assert!(screen.contains("Budget"), "{screen}");
}

#[test]
fn the_month_is_five_weeks_and_says_how_many_more_a_full_day_holds() {
    let mut app = super::demo();
    let first = sample().remove(0);
    app.agenda = (0..9)
        .map(|i| Coming {
            title: format!("E{i}"),
            ..first.clone()
        })
        .collect();
    super::reading::press(&mut app, &["C", "<tab>"]);
    let screen = crate::app::screen(&mut app, 120, 40);
    assert!(screen.contains("Sun"), "{screen}");
    assert!(screen.contains("Sat"), "{screen}");
    assert!(screen.contains("E0"), "{screen}");
    assert!(screen.contains("more"), "{screen}");
}

#[test]
fn every_view_stays_inside_the_window_and_a_narrow_one_is_the_list() {
    use unicode_width::UnicodeWidthStr;
    let mut app = super::demo();
    app.agenda = sample();
    super::reading::press(&mut app, &["C"]);
    for _ in 0..3 {
        for (w, h) in [(34, 14), (60, 12), (80, 24), (120, 40), (200, 60), (16, 6)] {
            let screen = crate::app::screen(&mut app, w, h);
            assert!(
                screen.lines().all(|l| l.width() <= usize::from(w)),
                "{w}x{h}: {screen}"
            );
        }
        super::reading::press(&mut app, &["<tab>"]);
    }
    let narrow = crate::app::screen(&mut app, 40, 20);
    assert!(narrow.contains("Calendar: List"), "{narrow}");
}

#[test]
fn the_popup_closes_on_esc_or_its_own_key_and_no_other_and_says_so() {
    let mut app = super::demo();
    app.agenda = sample();
    super::reading::press(&mut app, &["C"]);
    let open = crate::app::screen(&mut app, 120, 40);
    assert!(open.contains("esc closes"), "{open}");
    assert!(!open.contains("any other key"), "{open}");
    for key in ["x", "1", "<enter>", "<left>"] {
        super::reading::press(&mut app, &[key]);
        assert!(app.dialog.is_open(), "{key} left it open");
    }
    super::reading::press(&mut app, &["<esc>"]);
    assert!(!app.dialog.is_open());
    super::reading::press(&mut app, &["C", "C"]);
    assert!(!app.dialog.is_open());
}

#[test]
fn the_page_keys_scroll_the_popup_both_ways() {
    let mut app = super::demo();
    let first = sample().remove(0);
    app.agenda = (0..40)
        .map(|i| Coming {
            title: format!("E{i}:"),
            ..first.clone()
        })
        .collect();
    super::reading::press(&mut app, &["C", "<tab>", "<tab>"]);
    assert!(crate::app::screen(&mut app, 100, 14).contains("E0:"));
    super::reading::press(&mut app, &["<pgdn>"]);
    let _ = crate::app::screen(&mut app, 100, 14);
    assert!(!crate::app::screen(&mut app, 100, 14).contains("E0:"));
    super::reading::press(&mut app, &["<pgup>"]);
    assert!(app.dialog.is_open());
    assert!(crate::app::screen(&mut app, 100, 14).contains("E0:"));
}

// ---- As it may fail ----

/// Addresses in memory, by what each is kept under, `failing` to keep any
/// if so; what `keep_address` then leaves.
fn kept_after(
    set: &[&str],
    address: &str,
    alone: bool,
    failing: bool,
) -> (anyhow::Result<()>, Vec<String>, Vec<(String, String)>) {
    use crate::secret::Keeper;
    use std::cell::RefCell;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let kept = RefCell::new(Vec::new());
    for (i, a) in set.iter().enumerate() {
        let id = if i == 0 {
            crate::config::set_calendar(&path, Some(Keeper::Vault)).unwrap();
            crate::agenda::USER.to_string()
        } else {
            one_more(&path)
        };
        kept.borrow_mut().push((id, a.to_string()));
    }
    let done = crate::agenda::keep_address(
        &path,
        address,
        Keeper::Vault,
        alone,
        |id| {
            kept.borrow()
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, a)| a.clone())
                .ok_or_else(|| anyhow::anyhow!("none"))
        },
        |id, a| {
            if failing {
                anyhow::bail!("the store said no");
            }
            kept.borrow_mut().push((id.to_string(), a.to_string()));
            Ok(())
        },
        |id| {
            kept.borrow_mut().retain(|(k, _)| k != id);
            Ok(())
        },
    );
    let mut kept = kept.into_inner();
    kept.sort();
    (done, crate::config::calendar_ids(&path).unwrap(), kept)
}

#[test]
fn set_alone_keeps_the_new_one_before_it_forgets_the_others() {
    let (done, ids, kept) = kept_after(&["one", "two"], "new", true, false);
    done.unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0], (ids[0].clone(), "new".to_string()));
    let (done, ids, kept) = kept_after(&["one", "two"], "new", true, true);
    assert!(done.is_err());
    assert_eq!(ids.len(), 2, "still set");
    assert_eq!(kept.len(), 2, "nothing forgotten");
    assert_eq!(kept[0].1, "one");
}

#[test]
fn a_calendar_that_cannot_be_kept_is_not_set_and_one_set_already_is_refused() {
    let (done, ids, _) = kept_after(&["one"], "new", false, true);
    assert!(done.is_err());
    assert_eq!(ids, ["ical"], "no id left with nothing kept under it");
    let (done, ids, _) = kept_after(&["one"], "one", false, false);
    assert!(format!("{:#}", done.unwrap_err()).contains("already"));
    assert_eq!(ids.len(), 1);
    let (done, ids, kept) = kept_after(&["one"], "two", false, false);
    done.unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(kept.len(), 2);
    let (done, ids, kept) = kept_after(&[], "first", false, false);
    done.unwrap();
    assert_eq!(ids, ["ical"]);
    assert_eq!(kept, [("ical".to_string(), "first".to_string())]);
}

#[test]
fn what_a_calendar_calls_itself_is_cleaned_as_an_event_is_and_may_be_folded() {
    let name = crate::agenda::named(
        "BEGIN:VCALENDAR\r\nX-WR-CALNAME:Bad\u{1b}]52;c;eA==\u{7} one\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    assert!(!name.chars().any(char::is_control), "{name:?}");
    assert_eq!(
        crate::agenda::named("X-WR-CALNAME;VALUE=TEXT:Long\r\n  name\r\n").as_deref(),
        Some("Long name")
    );
}

#[test]
fn what_has_ended_is_on_no_day_shown() {
    let c = coming(
        "BEGIN:VEVENT\nUID:a\nDTSTART:20261105T090000Z\nDTEND:20261105T100000Z\nSUMMARY:Over\nEND:VEVENT\nBEGIN:VEVENT\nUID:b\nDTSTART:20261105T150000Z\nDTEND:20261105T160000Z\nSUMMARY:Later\nEND:VEVENT\n",
        "2026-11-05T01:00:00Z",
        "2026-11-20T00:00:00Z",
    );
    let first: jiff::civil::Date = "2026-11-05".parse().unwrap();
    let now = at("2026-11-05T12:00:00Z");
    let days = crate::agenda::on_each_day(&c, now, first, 1, &TimeZone::UTC);
    assert_eq!(days[0].len(), 1);
    assert_eq!(days[0][0].1, "Later");
}

#[test]
fn t_in_the_demo_starts_no_agent_of_the_users() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_agent(&path, Some("exit 0")).unwrap();
    let mut app = super::demo();
    app.as_demo();
    app.config = path;
    super::reading::press(&mut app, &["T"]);
    assert!(app.agent_set.is_none());
    assert!(app.agent.is_none());
}

#[test]
fn claude_is_set_by_its_whole_path_where_it_is_found() {
    let found = crate::config::claude_agent(Some("/home/a b/bin/claude"));
    assert!(found.starts_with("'/home/a b/bin/claude' -p "), "{found}");
    assert!(crate::config::claude_agent(None).starts_with("claude -p "));
}

#[test]
fn the_strip_says_a_calendar_was_not_read_though_it_has_others_to_show() {
    let mut app = super::demo();
    app.agenda = sample();
    app.agenda_fault = Some("calendar 2: it could not be reached".into());
    let screen = crate::app::screen(&mut app, 120, 30);
    let strip = screen.lines().nth(1).unwrap();
    assert!(strip.contains("not read"), "{screen}");
    assert!(screen.contains("Visitor talk"), "{screen}");
}

#[test]
fn a_name_stands_only_beside_the_calendar_it_was_read_from() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(crate::secret::Keeper::Vault)).unwrap();
    let mut app = super::demo();
    app.read_no_calendar_yet();
    app.config = path;
    app.agenda_calendars = vec![calendar("gone-since", "Was another")];
    super::reading::press(&mut app, &["C", "r"]);
    let screen = crate::app::screen(&mut app, 100, 30);
    assert!(screen.contains("[x] calendar 1"), "{screen}");
    assert!(!screen.contains("Was another"), "{screen}");
}

#[test]
fn what_is_pasted_in_a_line_typed_is_one_line() {
    let mut app = super::demo();
    app.read_no_calendar_yet();
    super::reading::press(&mut app, &["C", "a"]);
    app.paste("  https://example.test/a\r\nb.ics\n");
    match &app.dialog {
        crate::app::Dialog::AddCalendar(typed) => {
            assert_eq!(typed, "https://example.test/ab.ics");
        }
        _ => panic!("still asking"),
    }
}

#[test]
fn one_that_could_not_be_forgotten_stays_set_and_the_others_go() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    crate::config::set_calendar(&path, Some(crate::secret::Keeper::Vault)).unwrap();
    let second = one_more(&path);
    let third = one_more(&path);
    let gone = vec![second.clone(), third.clone()];
    let done = crate::agenda::keep_only(&path, &gone, |id| {
        if id == second {
            anyhow::bail!("the store said no");
        }
        Ok(())
    });
    assert!(done.is_err());
    assert_eq!(
        crate::config::calendar_ids(&path).unwrap(),
        ["ical".to_string(), second]
    );
}

#[test]
fn a_calendars_id_is_never_one_in_use_and_a_file_without_ids_has_the_one() {
    use crate::config::next_calendar_id;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(&path, "[calendar]\nkeep = \"vault\"\n").unwrap();
    assert_eq!(crate::config::calendar_ids(&path).unwrap(), ["ical"]);
    assert_eq!(next_calendar_id(&[]), "ical");
    assert_eq!(next_calendar_id(&["ical".to_string()]), "ical-2");
    assert_eq!(next_calendar_id(&["ical-2".to_string()]), "ical");
}

fn not_read(id: &str) -> crate::agenda::CalendarRead {
    crate::agenda::CalendarRead {
        id: id.to_string(),
        name: None,
        fault: Some("it could not be reached".to_string()),
        coming: Vec::new(),
    }
}

fn holding(id: &str, coming: Vec<Coming>) -> crate::agenda::CalendarRead {
    crate::agenda::CalendarRead {
        coming,
        ..calendar(id, id)
    }
}

fn answer(app: &mut crate::app::App, calendars: Vec<crate::agenda::CalendarRead>) {
    app.given_reply(crate::worker::Reply {
        account: usize::MAX,
        answer: crate::worker::Answer::Agenda(Ok(crate::agenda::Read {
            calendars,
            ..Default::default()
        })),
    });
}

#[test]
fn a_read_that_reaches_no_calendar_leaves_what_was_read_last() {
    let mut app = super::demo();
    app.agenda_calendars = vec![holding("x", sample())];
    answer(&mut app, vec![not_read("x")]);
    assert_eq!(titles(&app.agenda), ["Visitor talk", "Budget review"]);
    assert!(app.agenda_fault.is_some());
    answer(&mut app, vec![calendar("x", "One")]);
    assert!(app.agenda.is_empty(), "read, and nothing there");
    assert!(app.agenda_fault.is_none());
}

#[test]
fn what_could_not_be_read_stays_and_what_could_replaces_its_own_again_and_again() {
    let mut app = super::demo();
    let events = sample();
    let moved = Coming {
        title: "Moved".into(),
        ..events[1].clone()
    };
    app.agenda_calendars = vec![
        holding("well", vec![events[1].clone()]),
        holding("ill", vec![events[0].clone()]),
        holding("forgotten", vec![moved.clone()]),
    ];
    for _ in 0..3 {
        answer(
            &mut app,
            vec![holding("well", vec![moved.clone()]), not_read("ill")],
        );
        let mut seen = titles(&app.agenda);
        seen.sort_unstable();
        assert_eq!(seen, ["Moved", "Visitor talk"], "no more of them each time");
    }
    assert!(app.agenda_fault.is_some());
}

#[test]
fn google_not_read_at_all_keeps_each_of_its_calendars() {
    let mut app = super::demo();
    let events = sample();
    app.agenda_calendars = vec![
        holding("google:a", vec![events[0].clone()]),
        holding("google:b", vec![events[1].clone()]),
        holding("ical", Vec::new()),
    ];
    answer(&mut app, vec![calendar("ical", "One"), not_read("google")]);
    assert_eq!(titles(&app.agenda), ["Visitor talk", "Budget review"]);
}

#[test]
fn every_calendar_read_is_named_though_two_share_a_name() {
    let read = |id: &str| calendar(id, "Team");
    assert_eq!(
        crate::agenda::names(&[read("ical"), read("google:a")]),
        ["Team", "Team"]
    );
}
