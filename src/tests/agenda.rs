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
