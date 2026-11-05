//! A calendar read from its address, as an iCalendar file (RFC 5545), and
//! what is coming up in it, each repeat worked out: to look at, never written
//! to. What a rule says that is not understood is counted, not guessed at, as
//! an event at the wrong time is worse than one not shown.

use anyhow::{Context, bail};
use jiff::{
    Timestamp, ToSpan, Zoned,
    civil::{Date, DateTime, Time, Weekday},
    tz::TimeZone,
};
use std::collections::{BTreeSet, HashMap};

/// When something starts, as the file says it.
#[derive(Debug, Clone, PartialEq)]
enum Start {
    Day(Date),
    /// Its time on the clock, in its zone, or in the user's if it names none.
    Clock(DateTime, Option<TimeZone>),
}

/// How often it comes.
#[derive(Debug, Clone, PartialEq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq)]
struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<u32>,
    until: Option<Start>,
    /// Which weekdays, each the nth of its month if it says, from the end if
    /// less than none.
    by_day: Vec<(Option<i8>, Weekday)>,
    by_month_day: Vec<i8>,
    /// Whether weeks begin on Sunday, not Monday.
    sunday_first: bool,
}

/// An event in the file.
#[derive(Debug, Clone)]
pub struct Entry {
    uid: String,
    title: String,
    start: Start,
    /// How long it goes on, in seconds, or days if it is all day.
    length: i64,
    rule: Option<Rule>,
    except: Vec<Start>,
    /// The one of its repeats this stands for instead.
    instead_of: Option<Start>,
    cancelled: bool,
}

/// How much of the file could not be read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unread {
    pub rules: usize,
    pub events: usize,
}

/// Something coming up.
#[derive(Debug, Clone, PartialEq)]
pub struct Coming {
    pub start: Timestamp,
    pub end: Timestamp,
    pub all_day: bool,
    pub title: String,
}

impl Coming {
    /// Whether it is yet to begin, or going on, at `t`.
    pub(crate) fn on_at(&self, t: Timestamp) -> bool {
        if self.end == self.start {
            self.start >= t
        } else {
            self.end > t
        }
    }
}

/// The largest calendar read.
const LIMIT: u64 = 32 << 20;
/// The longest an event is taken to go on.
const LONGEST_DAYS: i64 = 3660;
/// How many times a rule is gone along before it is given up on.
const STEPS: u32 = 200_000;

/// The events in `text`, and how many could not be read.
pub fn parse(text: &str) -> (Vec<Entry>, Unread) {
    let unfolded = unfold(text);
    let mut entries = Vec::new();
    let mut unread = Unread::default();
    let mut event: Option<Vec<&str>> = None;
    // What is inside an event, as its reminders, says things of its own.
    let mut inside = 0usize;
    for line in unfolded.lines() {
        match line.trim_end() {
            "BEGIN:VEVENT" => {
                event = Some(Vec::new());
                inside = 0;
            }
            l if event.is_some() && l.starts_with("BEGIN:") => inside += 1,
            l if event.is_some() && inside > 0 && l.starts_with("END:") => inside -= 1,
            _ if inside > 0 => {}
            "END:VEVENT" => {
                if let Some(lines) = event.take() {
                    entry(&lines, &mut entries, &mut unread);
                }
            }
            _ => {
                if let Some(lines) = &mut event {
                    lines.push(line);
                }
            }
        }
    }
    (entries, unread)
}

fn entry(lines: &[&str], entries: &mut Vec<Entry>, unread: &mut Unread) {
    let (mut uid, mut title, mut start, mut end) = (String::new(), String::new(), None, None);
    let (mut rule_text, mut except, mut instead_of, mut cancelled) =
        (None, Vec::new(), None, false);
    let mut duration = None;
    for line in lines {
        let Some((name, value)) = name_and_value(line) else {
            continue;
        };
        let (name, params) = name.split_once(';').unwrap_or((name, ""));
        match name.to_ascii_uppercase().as_str() {
            "UID" => uid = value.to_string(),
            "SUMMARY" => title = text(value),
            "DTSTART" => start = when(value, params),
            "DTEND" => end = when(value, params),
            "DURATION" => duration = seconds(value),
            "RRULE" => rule_text = Some(value),
            "EXDATE" => except.extend(value.split(',').filter_map(|v| when(v, params))),
            "RECURRENCE-ID" => instead_of = when(value, params),
            "STATUS" => cancelled = value.eq_ignore_ascii_case("CANCELLED"),
            _ => {}
        }
    }
    let Some(start) = start else {
        unread.events += 1;
        return;
    };
    let rule = match rule_text {
        Some(r) => match rule(r) {
            Some(r) => Some(r),
            None => {
                unread.rules += 1;
                return;
            }
        },
        None => None,
    };
    let length = length(&start, end.as_ref(), duration);
    entries.push(Entry {
        uid,
        title,
        start,
        length,
        rule,
        except,
        instead_of,
        cancelled,
    });
}

/// `line` parted at its first colon outside quotes.
fn name_and_value(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some((&line[..at], &line[at + 1..])),
            _ => {}
        }
    }
    None
}

/// `s`, what is escaped read, and no control character, which a terminal
/// might act on.
fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n' | 'N') => out.push(' '),
                Some(c) => out.push(c),
                None => {}
            },
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn when(value: &str, params: &str) -> Option<Start> {
    let value = value.trim();
    if value.len() == 8 {
        return date(value).map(Start::Day);
    }
    let (day, time) = value.split_once('T')?;
    let day = date(day)?;
    let utc = time.ends_with('Z');
    let time = time.trim_end_matches('Z');
    if time.len() != 6 || !time.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |i: usize| time[i..i + 2].parse::<i8>().ok();
    let time = Time::new(n(0)?, n(2)?, n(4)?.min(59), 0).ok()?;
    let named = params.split(';').find_map(|p| p.strip_prefix("TZID="));
    let zone = match (utc, named) {
        (true, _) => Some(TimeZone::UTC),
        // A zone not known is not guessed at: left out, and counted.
        (false, Some(name)) => Some(TimeZone::get(name.trim_matches('"')).ok()?),
        (false, None) => None,
    };
    Some(Start::Clock(day.to_datetime(time), zone))
}

fn date(s: &str) -> Option<Date> {
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Date::new(
        s[0..4].parse().ok()?,
        s[4..6].parse().ok()?,
        s[6..8].parse().ok()?,
    )
    .ok()
}

/// A duration as the file writes it, as `PT1H30M` or `P1D`, in seconds.
fn seconds(s: &str) -> Option<i64> {
    let s = s.trim().trim_start_matches('+').strip_prefix('P')?;
    let (mut total, mut n) = (0i64, 0i64);
    for c in s.chars() {
        match c {
            '0'..='9' => n = n.checked_mul(10)?.checked_add(i64::from(c.to_digit(10)?))?,
            'T' => {}
            'W' | 'D' | 'H' | 'M' | 'S' => {
                let unit = match c {
                    'W' => 604_800,
                    'D' => 86_400,
                    'H' => 3_600,
                    'M' => 60,
                    _ => 1,
                };
                total = total.checked_add(n.checked_mul(unit)?)?;
                n = 0;
            }
            _ => return None,
        }
    }
    Some(total)
}

/// How long it goes on: in days if all day, else in seconds; a day, or none,
/// if the file does not say.
fn length(start: &Start, end: Option<&Start>, duration: Option<i64>) -> i64 {
    let length = match (start, end) {
        (Start::Day(first), Some(Start::Day(last))) => (*last - *first).get_days().max(1).into(),
        (Start::Day(_), _) => duration.map_or(1, |d| (d / 86_400).max(1)),
        (Start::Clock(..), Some(end @ Start::Clock(..))) => {
            let zone = TimeZone::UTC;
            match (moment(start, &zone), moment(end, &zone)) {
                (Some(a), Some(b)) => (b.as_second() - a.as_second()).max(0),
                _ => 0,
            }
        }
        (Start::Clock(..), _) => duration.unwrap_or(0).max(0),
    };
    // Nothing goes on longer: what says so must not make a date fall over.
    length.min(match start {
        Start::Day(_) => LONGEST_DAYS,
        Start::Clock(..) => LONGEST_DAYS * 86_400,
    })
}

/// The rule the file writes, if all it says is understood.
fn rule(s: &str) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        sunday_first: false,
    };
    let mut freq = false;
    for part in s.trim().split(';').filter(|p| !p.is_empty()) {
        let (name, value) = part.split_once('=')?;
        match name.to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = true;
                rule.freq = match value.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                };
            }
            "INTERVAL" => {
                rule.interval = value.parse().ok().filter(|n| (1..=10_000).contains(n))?
            }
            "COUNT" => rule.count = Some(value.parse().ok()?),
            "UNTIL" => rule.until = Some(when(value, "")?),
            "BYDAY" => {
                for d in value.split(',') {
                    rule.by_day.push(by_day(d)?);
                }
            }
            "BYMONTHDAY" => {
                for d in value.split(',') {
                    let d: i8 = d.parse().ok()?;
                    if d == 0 || d.unsigned_abs() > 31 {
                        return None;
                    }
                    rule.by_month_day.push(d);
                }
            }
            "WKST" if value.eq_ignore_ascii_case("MO") => {}
            "WKST" if value.eq_ignore_ascii_case("SU") => rule.sunday_first = true,
            _ => return None,
        }
    }
    // What the working out would ignore: not understood, rather than wrong.
    let by_the_month = matches!(rule.freq, Freq::Monthly | Freq::Yearly);
    let ignored = (!rule.by_month_day.is_empty() && !rule.by_day.is_empty())
        || (!rule.by_month_day.is_empty() && !by_the_month)
        || (!by_the_month && rule.by_day.iter().any(|(n, _)| n.is_some()));
    (freq && !ignored).then_some(rule)
}

fn by_day(s: &str) -> Option<(Option<i8>, Weekday)> {
    let s = s.trim();
    let split = s.len().checked_sub(2)?;
    let (n, day) = s.split_at_checked(split)?;
    let day = match day.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    };
    let n = if n.is_empty() {
        None
    } else {
        Some(n.parse().ok()?)
    };
    Some((n, day))
}

fn moment(start: &Start, zone: &TimeZone) -> Option<Timestamp> {
    match start {
        Start::Day(d) => Some(d.to_zoned(zone.clone()).ok()?.timestamp()),
        Start::Clock(at, z) => Some(
            z.as_ref()
                .unwrap_or(zone)
                .to_ambiguous_zoned(*at)
                .compatible()
                .ok()?
                .timestamp(),
        ),
    }
}

impl Entry {
    fn once(title: String, start: Start, end: &Start) -> Entry {
        Entry {
            uid: String::new(),
            title,
            length: length(&start, Some(end), None),
            start,
            rule: None,
            except: Vec::new(),
            instead_of: None,
            cancelled: false,
        }
    }

    /// Something that comes once, from `start` to `end`.
    pub fn timed(title: String, start: Timestamp, end: Timestamp) -> Entry {
        let clock = |at: Timestamp| {
            let at = at.to_zoned(TimeZone::UTC).datetime();
            Start::Clock(at, Some(TimeZone::UTC))
        };
        Entry::once(title, clock(start), &clock(end))
    }

    /// Something that takes the days from `first` until `after`.
    pub fn days(title: String, first: Date, after: Date) -> Entry {
        Entry::once(title, Start::Day(first), &Start::Day(after))
    }

    /// Where it comes, from its first, up to `last`, while `go` says to go on.
    fn repeats(&self, last: Date, mut go: impl FnMut(Date) -> bool) {
        let first = match &self.start {
            Start::Day(d) => *d,
            Start::Clock(at, _) => at.date(),
        };
        let Some(rule) = &self.rule else {
            go(first);
            return;
        };
        let from_week_start = |w: Weekday| {
            if rule.sunday_first {
                i64::from(w.to_sunday_zero_offset())
            } else {
                i64::from(w.to_monday_zero_offset())
            }
        };
        let week_of = |d: Date| d.checked_sub(from_week_start(d.weekday()).days()).ok();
        let on_days: BTreeSet<i64> = if rule.by_day.is_empty() {
            [from_week_start(first.weekday())].into()
        } else {
            rule.by_day
                .iter()
                .map(|(_, w)| from_week_start(*w))
                .collect()
        };
        let mut made = 0u32;
        for k in 0..i64::from(STEPS) {
            let at = k.saturating_mul(rule.interval);
            let period = match rule.freq {
                Freq::Daily => first.checked_add(at.days()),
                Freq::Weekly => match week_of(first) {
                    Some(week) => week.checked_add((at * 7).days()),
                    None => return,
                },
                Freq::Monthly => first.first_of_month().checked_add(at.months()),
                Freq::Yearly => first.first_of_month().checked_add(at.years()),
            };
            // What finds nothing stops here too, not at the end of all time.
            let Some(period) = period.ok().filter(|p| *p <= last) else {
                return;
            };
            let mut days: Vec<Date> = match rule.freq {
                Freq::Daily => {
                    if rule.by_day.is_empty()
                        || rule.by_day.iter().any(|(_, w)| *w == period.weekday())
                    {
                        vec![period]
                    } else {
                        Vec::new()
                    }
                }
                Freq::Weekly => on_days
                    .iter()
                    .filter_map(|o| period.checked_add(o.days()).ok())
                    .collect(),
                Freq::Monthly | Freq::Yearly => in_month(period, rule, first),
            };
            days.sort_unstable();
            days.dedup();
            for day in days {
                if day < first {
                    continue;
                }
                if let Some(count) = rule.count
                    && made >= count
                {
                    return;
                }
                if let Some(until) = &rule.until
                    && !self.within(day, until)
                {
                    return;
                }
                made += 1;
                if !go(day) {
                    return;
                }
            }
        }
    }

    /// Whether its repeat on `day` is not after `until`.
    fn within(&self, day: Date, until: &Start) -> bool {
        match (until, &self.start) {
            (Start::Day(u), _) => day <= *u,
            (u @ Start::Clock(..), Start::Clock(..)) => {
                let this = self.on(day);
                let zone = TimeZone::UTC;
                match (moment(&this, &zone), moment(u, &zone)) {
                    (Some(a), Some(b)) => a <= b,
                    _ => false,
                }
            }
            (Start::Clock(u, _), Start::Day(_)) => day <= u.date(),
        }
    }

    /// Its repeat on `day`.
    fn on(&self, day: Date) -> Start {
        match &self.start {
            Start::Day(_) => Start::Day(day),
            Start::Clock(at, z) => Start::Clock(day.to_datetime(at.time()), z.clone()),
        }
    }
}

/// The days of the month beginning `month` that `rule` picks: in the month
/// of `first`, if a rule by the year. A day the month lacks is passed over.
fn in_month(month: Date, rule: &Rule, first: Date) -> Vec<Date> {
    let last = month.days_in_month();
    let of = |day: i8| month.with().day(day).build().ok();
    if !rule.by_month_day.is_empty() {
        return rule
            .by_month_day
            .iter()
            .filter_map(|&d| {
                if d > 0 && d <= last {
                    of(d)
                } else if d < 0 && -d <= last {
                    of(last + d + 1)
                } else {
                    None
                }
            })
            .collect();
    }
    if !rule.by_day.is_empty() {
        let mut days = Vec::new();
        for &(n, weekday) in &rule.by_day {
            let all: Vec<Date> = (1..=last)
                .filter_map(of)
                .filter(|d| d.weekday() == weekday)
                .collect();
            match n {
                None => days.extend(all),
                Some(n) => days.extend(month.nth_weekday_of_month(n, weekday).ok()),
            }
        }
        return days;
    }
    of(first.day()).into_iter().collect()
}

/// What of `entries` is going on or begins from `from` to `to`, soonest first,
/// in `zone` where a time names none.
pub fn upcoming(entries: &[Entry], from: Timestamp, to: Timestamp, zone: &TimeZone) -> Vec<Coming> {
    let mut out = Vec::new();
    // Far enough past `to` in any zone for the rule's last period.
    let last = to
        .to_zoned(TimeZone::UTC)
        .date()
        .tomorrow()
        .unwrap_or(Date::MAX);
    let mut instead: HashMap<&str, Vec<&Start>> = HashMap::new();
    for entry in entries {
        if let Some(of) = &entry.instead_of {
            instead.entry(&entry.uid).or_default().push(of);
        }
    }
    for entry in entries {
        let moved = instead
            .get(entry.uid.as_str())
            .map_or(&[][..], Vec::as_slice);
        if entry.cancelled {
            continue;
        }
        let all_day = matches!(entry.start, Start::Day(_));
        entry.repeats(last, |day| {
            let Some(begins) = moment(&entry.on(day), zone) else {
                return true;
            };
            if begins >= to {
                return false;
            }
            let end = if all_day {
                day.checked_add(entry.length.days())
                    .ok()
                    .and_then(|d| d.to_zoned(zone.clone()).ok())
                    .map(|z| z.timestamp())
            } else {
                begins.checked_add(entry.length.seconds()).ok()
            };
            let Some(end) = end else {
                return false;
            };
            let coming = Coming {
                start: begins,
                end,
                all_day,
                title: entry.title.clone(),
            };
            // What is past is passed over before it is looked up.
            if !coming.on_at(from) {
                return true;
            }
            let same = |o: &Start| moment(o, zone) == Some(begins);
            if entry.instead_of.is_some()
                || !(entry.except.iter().any(same) || moved.iter().any(|o| same(o)))
            {
                out.push(coming);
            }
            true
        });
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.title.cmp(&b.title)));
    out
}

/// What `shown` is called on `today`: its name, near, and from a week away,
/// when its weekday alone would mislead, with its date.
fn day_said(shown: jiff::civil::Date, today: jiff::civil::Date) -> String {
    if shown == today {
        "Today".to_string()
    } else if today.tomorrow().is_ok_and(|t| t == shown) {
        "Tomorrow".to_string()
    } else {
        let form = if (shown - today).get_days() < 7 {
            "%a"
        } else {
            "%a %b %-d"
        };
        shown.strftime(form).to_string()
    }
}

/// What the strip says of what is `coming` still at `now`: when, in `zone`,
/// and what.
pub fn strip(coming: &[Coming], now: Timestamp, zone: &TimeZone) -> Vec<(String, String)> {
    let today = now.to_zoned(zone.clone()).date();
    coming
        .iter()
        .filter(|c| c.on_at(now))
        .map(|c| {
            let at: Zoned = c.start.to_zoned(zone.clone());
            if !c.all_day && c.start <= now {
                return ("Now".to_string(), c.title.clone());
            }
            let day = day_said(at.date().max(today), today);
            let when = if c.all_day {
                day
            } else {
                format!("{day} {}", at.strftime("%-I:%M %p"))
            };
            (when, c.title.clone())
        })
        .collect()
}

/// What the popup lists of what is `coming` still at `now`: the day, said
/// only as it changes; when in it, in `zone`; and what.
pub fn listing(
    coming: &[Coming],
    now: Timestamp,
    zone: &TimeZone,
) -> Vec<(String, String, String)> {
    let today = now.to_zoned(zone.clone()).date();
    let mut last = None;
    coming
        .iter()
        .filter(|c| c.on_at(now))
        .map(|c| {
            let at: Zoned = c.start.to_zoned(zone.clone());
            let shown = at.date().max(today);
            let day = if last == Some(shown) {
                String::new()
            } else {
                day_said(shown, today)
            };
            last = Some(shown);
            let when = if c.all_day {
                "all day".to_string()
            } else if c.start <= now {
                "now".to_string()
            } else {
                at.strftime("%-I:%M %p").to_string()
            };
            (day, when, c.title.clone())
        })
        .collect()
}

/// What is on still at `now` on each of `days` days from `first`, in `zone`,
/// each said by when (nothing, all day) and what: an all-day event on every day it covers, a
/// timed one where it begins, or the first shown if it began before.
pub fn on_each_day(
    coming: &[Coming],
    now: Timestamp,
    first: jiff::civil::Date,
    days: usize,
    zone: &TimeZone,
) -> Vec<Vec<(String, String)>> {
    let mut out = vec![Vec::new(); days];
    for c in coming.iter().filter(|c| c.on_at(now)) {
        let at = c.start.to_zoned(zone.clone());
        let begins = (at.date() - first).get_days().max(0);
        let (ends, when) = if c.all_day {
            let ends = (c.end.to_zoned(zone.clone()).date() - first).get_days();
            (ends.max(begins + 1), String::new())
        } else {
            let am = if at.hour() < 12 { "a" } else { "p" };
            (begins + 1, format!("{}{am}", at.strftime("%-I:%M")))
        };
        for on in out
            .iter_mut()
            .take(usize::try_from(ends).unwrap_or(0))
            .skip(usize::try_from(begins).unwrap_or(0))
        {
            on.push((when.clone(), c.title.clone()));
        }
    }
    out
}

/// How far ahead is read, for the strip and the popup: the five weeks of its
/// month, from the start of this one.
pub const AHEAD_DAYS: i64 = 35;

/// One calendar as it was read.
#[derive(Debug)]
pub struct Feed {
    pub entries: Vec<Entry>,
    pub unread: Unread,
    pub name: Option<String>,
}

/// What stands for Google's calendars when it gave none of them.
const GOOGLE: &str = "google";

/// A calendar as it came back, read or not.
#[derive(Debug)]
pub struct Reading {
    pub id: String,
    /// What it is called, if that is known whether it can be read or not.
    pub name: Option<String>,
    pub feed: Result<Feed, String>,
}

impl Reading {
    pub fn of(id: &str, feed: Result<Feed, String>) -> Reading {
        Reading {
            id: id.to_string(),
            name: None,
            feed,
        }
    }
}

/// A calendar set, as it was last read.
#[derive(Clone, Debug, PartialEq)]
pub struct CalendarRead {
    /// What its address is kept under, or, for Google's, `google:` and its ID;
    /// `google` itself if Google gave none of them.
    pub id: String,
    /// What it calls itself, if that is known.
    pub name: Option<String>,
    /// Why it could not be read.
    pub fault: Option<String>,
    /// What it has coming up; if it could not be read, as it last did.
    pub coming: Vec<Coming>,
}

impl CalendarRead {
    /// What to call it, the `place`th of those set, from one.
    pub fn called(&self, place: usize) -> String {
        match (&self.name, &self.fault) {
            (Some(name), _) => name.clone(),
            (None, Some(_)) => format!("calendar {place} (not read)"),
            (None, None) => format!("calendar {place}"),
        }
    }
}

/// What every calendar set has coming up: what was read of them, together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Read {
    pub coming: Vec<Coming>,
    pub unread: Unread,
    pub calendars: Vec<CalendarRead>,
}

/// What those of `calendars` read are called.
pub fn names(calendars: &[CalendarRead]) -> Vec<String> {
    calendars
        .iter()
        .enumerate()
        .filter(|(_, c)| c.fault.is_none())
        .map(|(i, c)| c.called(i + 1))
        .collect()
}

/// `coming` soonest first, an event on two calendars once.
fn in_order(coming: &mut Vec<Coming>) {
    coming.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.end.cmp(&b.end))
    });
    coming.dedup_by(|a, b| a.start == b.start && a.end == b.end && a.title == b.title);
}

/// Those of `calendars` that could not be read, and why.
pub fn faults(calendars: &[CalendarRead]) -> Vec<String> {
    calendars
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let why = c.fault.as_ref()?;
            Some(match &c.name {
                Some(name) => format!("{name}: {why}"),
                None => format!("calendar {}: {why}", i + 1),
            })
        })
        .collect()
}

/// `feeds` as one, from `now` to `to`, in `zone`: soonest first, an event on two
/// of them once; one that could not be read said, the others shown.
pub fn together(feeds: Vec<Reading>, now: Timestamp, to: Timestamp, zone: &TimeZone) -> Read {
    let mut read = Read::default();
    for Reading { id, name, feed } in feeds {
        let (name, fault, coming) = match feed {
            Ok(feed) => {
                read.unread.rules += feed.unread.rules;
                read.unread.events += feed.unread.events;
                let coming = upcoming(&feed.entries, now, to, zone);
                (feed.name.or(name), None, coming)
            }
            Err(why) => (name, Some(why), Vec::new()),
        };
        read.coming.extend_from_slice(&coming);
        read.calendars.push(CalendarRead {
            id,
            name,
            fault,
            coming,
        });
    }
    in_order(&mut read.coming);
    read
}

/// What is coming up, as `read` has it and, for each calendar it could not read,
/// as `last` did: a gap would be taken for free time. What each calendar that
/// was read has takes the place of what it had, and one no longer set is gone.
pub fn carried_over(last: &[CalendarRead], read: &mut Read) -> Vec<Coming> {
    for calendar in read.calendars.iter_mut().filter(|c| c.fault.is_some()) {
        // Google not read at all stands for each of its calendars.
        let was = |c: &&CalendarRead| {
            c.id == calendar.id || (calendar.id == GOOGLE && c.id.starts_with("google:"))
        };
        calendar.coming = last
            .iter()
            .filter(was)
            .flat_map(|c| c.coming.clone())
            .collect();
    }
    let mut all: Vec<Coming> = read
        .calendars
        .iter()
        .flat_map(|c| c.coming.iter().cloned())
        .collect();
    in_order(&mut all);
    all
}

/// Open the vault, if `keeper` is it and its key is at hand.
fn reach(keeper: crate::secret::Keeper) -> anyhow::Result<()> {
    if keeper == crate::secret::Keeper::Vault
        && !crate::vault::is_open()
        && !crate::vault::unlock_by_key().is_some_and(|r| r.is_ok())
    {
        bail!("the calendar's address is in the vault, which is locked");
    }
    Ok(())
}

/// What is coming up for `days` from now in the calendars the accounts' file
/// at `config` has set, in the user's zone; none if it has none set. Their
/// addresses are asked for in turn, as the system's store may ask the user; they
/// and Google are read at once, so none that does not answer holds up the others
/// more than it takes to give up on it.
pub fn read(config: &std::path::Path, days: i64) -> anyhow::Result<Option<Read>> {
    let Some(keeper) = crate::config::calendar(config)? else {
        return Ok(None);
    };
    reach(keeper)?;
    let kept: Vec<_> = crate::config::calendar_ids(config)?
        .into_iter()
        .map(|id| {
            let at = crate::secret::find(keeper, HOST, &id).and_then(|kept| address(&kept));
            (id, at)
        })
        .collect();
    let now = Timestamp::now();
    let to = now.checked_add(jiff::SignedDuration::from_hours(days.saturating_mul(24)))?;
    let client = crate::config::google_client(config)?;
    let feeds = std::thread::scope(|scope| {
        let reading: Vec<_> = kept
            .into_iter()
            .map(|(id, at)| (id, scope.spawn(move || fetch(&at?))))
            .collect();
        let google =
            client.map(|client| scope.spawn(move || read_google(keeper, &client, now, to)));
        let mut feeds: Vec<Reading> = reading
            .into_iter()
            .map(|(id, read)| {
                let feed = read
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("the calendar could not be read")));
                Reading::of(&id, feed.map_err(|e| format!("{e:#}")))
            })
            .collect();
        if let Some(google) = google {
            match google.join() {
                Ok(Ok(calendars)) => feeds.extend(calendars),
                Ok(Err(why)) => feeds.push(google_unread(format!("{why:#}"))),
                Err(_) => feeds.push(google_unread("it could not be read".to_string())),
            }
        }
        feeds
    });
    Ok(Some(together(feeds, now, to, &TimeZone::system())))
}

/// Google's calendars not read at all, and why.
fn google_unread(why: String) -> Reading {
    Reading {
        id: GOOGLE.to_string(),
        name: Some("Google".to_string()),
        feed: Err(why),
    }
}

/// What the user sees in Google Calendar, by the sign-in kept in `keeper`.
fn read_google(
    keeper: crate::secret::Keeper,
    client: &str,
    now: Timestamp,
    to: Timestamp,
) -> anyhow::Result<Vec<Reading>> {
    let app = app_kept(client, |what| crate::secret::find(keeper, HOST, what))?;
    let refresh = crate::secret::find(keeper, HOST, crate::google::REFRESH).map_err(|_| {
        anyhow::anyhow!(
            "its sign-in could not be read from where it is kept: lbx calendar google \
             signs in again"
        )
    })?;
    crate::google::read(&crate::google::Hosts::default(), &app, &refresh, now, to)
}

/// The app of the last sign-in, by its ID and by `find` for its secret, which
/// comes only from there or from Google's file.
pub fn app_kept(
    client: &str,
    find: impl FnOnce(&str) -> anyhow::Result<String>,
) -> anyhow::Result<crate::google::App> {
    let secret = find(crate::google::SECRET).context(
        "the app of the last sign-in could not be read from where it is kept: \
         lbx calendar google --app the-file signs in again",
    )?;
    Ok(crate::google::App {
        client: client.to_string(),
        secret,
    })
}

/// Keep a sign-in to Google with `app`, once it has read: one that does not read
/// takes the place of nothing. What it read of the day ahead.
pub fn keep_google(
    config: &std::path::Path,
    keeper: crate::secret::Keeper,
    app: &crate::google::App,
    refresh: &str,
) -> anyhow::Result<Read> {
    let now = Timestamp::now();
    let to = now.checked_add(jiff::SignedDuration::from_hours(24))?;
    let hosts = crate::google::Hosts::default();
    let feeds = crate::google::read(&hosts, app, refresh, now, to).context("Google")?;
    let read = together(feeds, now, to, &TimeZone::system());
    let faults = faults(&read.calendars);
    if names(&read.calendars).is_empty() && !faults.is_empty() {
        bail!(
            "signed in, but no calendar could be read: {}",
            faults.join("; ")
        );
    }
    keep_sign_in(
        config,
        keeper,
        app,
        refresh,
        |what| crate::secret::find(keeper, HOST, what).ok(),
        |what, value| crate::secret::store(keeper, HOST, what, value),
        |what| crate::secret::remove(keeper, HOST, what),
    )?;
    Ok(read)
}

/// Keep what a sign-in to Google needs, by `find`, `store` and `forget`, each of
/// what it is kept under; if any of it cannot be, all is put back as it was, as
/// one part of a new sign-in does not work with another of an old.
pub fn keep_sign_in(
    config: &std::path::Path,
    keeper: crate::secret::Keeper,
    app: &crate::google::App,
    refresh: &str,
    find: impl Fn(&str) -> Option<String>,
    store: impl Fn(&str, &str) -> anyhow::Result<()>,
    forget: impl Fn(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    use crate::google::{REFRESH, SECRET};
    let was = [(SECRET, find(SECRET)), (REFRESH, find(REFRESH))];
    let kept = store(SECRET, &app.secret)
        .and_then(|()| store(REFRESH, refresh))
        .and_then(|()| crate::config::set_google(config, keeper, &app.client));
    if kept.is_err() {
        for (what, value) in &was {
            // What it said first is what is said: this is only to tidy up.
            let _ = match value {
                Some(value) => store(what, value),
                None => forget(what),
            };
        }
    }
    kept
}

/// Where the calendars' addresses are kept: where they are, or, with none yet,
/// with the passwords.
pub fn keeper(config: &std::path::Path) -> anyhow::Result<crate::secret::Keeper> {
    if let Some(keeper) = crate::config::calendar(config)? {
        return Ok(keeper);
    }
    let keepers = crate::secret::keepers();
    crate::config::load(config)?
        .first()
        .map(|a| a.keep)
        .filter(|k| keepers.contains(k))
        .or(keepers.first().copied())
        .ok_or_else(|| anyhow::anyhow!("nowhere to keep its address"))
}

/// Keep `address` beside the others' or, `alone`, in their place, by `find`,
/// `store` and `forget`, each of what an address is kept under: nothing is set
/// until it is kept, and nothing forgotten till then.
pub fn keep_address(
    config: &std::path::Path,
    address: &str,
    keeper: crate::secret::Keeper,
    alone: bool,
    find: impl Fn(&str) -> anyhow::Result<String>,
    store: impl Fn(&str, &str) -> anyhow::Result<()>,
    forget: impl Fn(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let ids = crate::config::calendar_ids(config)?;
    if !alone
        && ids
            .iter()
            .any(|id| find(id).is_ok_and(|kept| kept == address))
    {
        bail!("that calendar is set already");
    }
    let id = crate::config::next_calendar_id(&ids);
    store(&id, address)?;
    crate::config::set_calendar(config, Some(keeper))?;
    if alone {
        crate::config::set_calendars(config, std::slice::from_ref(&id))?;
        return ids.iter().try_for_each(|id| forget(id));
    }
    let mut all = ids;
    all.push(id);
    crate::config::set_calendars(config, &all)
}

/// One more calendar, at `given`, or, `alone`, this one only: read before it
/// is kept. What was read, and where it is kept. What goes wrong never says
/// the address.
pub fn add(
    config: &std::path::Path,
    given: &str,
    alone: bool,
) -> anyhow::Result<(Feed, crate::secret::Keeper)> {
    let address = address(given)?;
    let feed = fetch(&address)?;
    let keeper = keeper(config)?;
    reach(keeper)?;
    keep_address(
        config,
        &address,
        keeper,
        alone,
        |id| crate::secret::find(keeper, HOST, id),
        |id, address| crate::secret::store(keeper, HOST, id, address),
        |id| crate::secret::remove(keeper, HOST, id),
    )?;
    Ok((feed, keeper))
}

/// Those of `kept` that `forget` could not forget: each is tried, whatever came
/// of the others.
fn not_forgotten(kept: &[String], forget: impl Fn(&str) -> anyhow::Result<()>) -> Vec<&String> {
    kept.iter().filter(|what| forget(what).is_err()).collect()
}

/// Forget the calendars `gone`, by `forget`, each of what an address is kept
/// under: the others stay as they are, and so does one that could not be
/// forgotten, set still so that it may be tried again.
pub fn keep_only(
    config: &std::path::Path,
    gone: &[String],
    forget: impl Fn(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let ids = crate::config::calendar_ids(config)?;
    let going: Vec<String> = ids.iter().filter(|id| gone.contains(id)).cloned().collect();
    let failed = not_forgotten(&going, forget);
    let left: Vec<String> = ids
        .iter()
        .filter(|id| !going.contains(id) || failed.contains(id))
        .cloned()
        .collect();
    if left.is_empty() && crate::config::google_client(config)?.is_none() {
        crate::config::set_calendar(config, None)?;
    } else {
        crate::config::set_calendars(config, &left)?;
    }
    if !failed.is_empty() {
        bail!(
            "{} could not be forgotten: still set, to try again",
            failed.len()
        );
    }
    Ok(())
}

/// Read no calendar, and forget all that is kept for them.
pub fn forget_all(config: &std::path::Path) -> anyhow::Result<()> {
    let Some(keeper) = crate::config::calendar(config)? else {
        return Ok(());
    };
    reach(keeper)?;
    forget_all_by(config, |what| crate::secret::remove(keeper, HOST, what))
}

/// As `forget_all`, by `forget`: all of it is tried, and only with all of it gone
/// are the calendars no longer set, so what is left may be tried again.
pub fn forget_all_by(
    config: &std::path::Path,
    forget: impl Fn(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let kept = crate::config::calendar_secrets(config)?;
    let failed = not_forgotten(&kept, forget);
    if !failed.is_empty() {
        bail!(
            "{} could not be forgotten: the calendars stay set, to try again",
            failed.len()
        );
    }
    crate::config::set_calendar(config, None)
}

/// Forget the calendars `gone`, as `keep_only` does, where they are kept.
pub fn remove(config: &std::path::Path, gone: &[String]) -> anyhow::Result<()> {
    let Some(keeper) = crate::config::calendar(config)? else {
        return Ok(());
    };
    reach(keeper)?;
    keep_only(config, gone, |id| crate::secret::remove(keeper, HOST, id))
}

/// `coming` for an agent: times with their offset in `zone`, as every message's
/// date is; days as days, the last one it is on.
pub fn for_agent(coming: &[Coming], zone: &TimeZone) -> Vec<serde_json::Value> {
    let time = |t: Timestamp| {
        t.to_zoned(zone.clone())
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string()
    };
    let day = |t: Timestamp| t.to_zoned(zone.clone()).date();
    coming
        .iter()
        .map(|c| {
            let (start, end) = if c.all_day {
                let last = day(c.end).yesterday().unwrap_or_else(|_| day(c.end));
                (day(c.start).to_string(), last.to_string())
            } else {
                (time(c.start), time(c.end))
            };
            serde_json::json!({ "start": start, "end": end, "all_day": c.all_day, "title": c.title })
        })
        .collect()
}

/// How the address is kept, as a password is.
pub const HOST: &str = "calendar";
pub const USER: &str = "ical";

/// A week of what the demo has coming up.
pub fn demo() -> Vec<Coming> {
    let now = Timestamp::now();
    let zone = TimeZone::system();
    let today = now.to_zoned(zone.clone()).date();
    let event = |days: i64, hour: i8, title: &str| {
        let start = today
            .checked_add(days.days())
            .ok()?
            .to_datetime(Time::new(hour, 0, 0, 0).ok()?)
            .to_zoned(zone.clone())
            .ok()?
            .timestamp();
        Some(Coming {
            start,
            end: start
                .checked_add(jiff::SignedDuration::from_hours(1))
                .ok()?,
            all_day: false,
            title: title.to_string(),
        })
    };
    [
        event(0, 23, "Call with a collaborator"),
        event(1, 14, "Lab meeting"),
        event(3, 10, "Thesis committee"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The address to ask, as given. It is as good as a password, so what is
/// said of one refused does not repeat it.
pub fn address(given: &str) -> anyhow::Result<String> {
    let given = given.trim();
    if let Some(rest) = given.strip_prefix("webcal://") {
        return Ok(format!("https://{rest}"));
    }
    if given.starts_with("https://") {
        return Ok(given.to_string());
    }
    bail!("a calendar's address begins with https://, as Google's secret address does");
}

/// `text` with each line that goes on over several as one.
fn unfold(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace("\n ", "")
        .replace("\n\t", "")
}

/// What the calendar in `text` calls itself, cleaned as an event's title is.
pub fn named(given: &str) -> Option<String> {
    // It comes before the events, which may be many.
    unfold(given.split("BEGIN:VEVENT").next().unwrap_or(given))
        .lines()
        .find_map(|l| {
            let (name, value) = name_and_value(l)?;
            let name = name.split(';').next()?;
            name.eq_ignore_ascii_case("X-WR-CALNAME")
                .then(|| text(value))
        })
        .filter(|n| !n.is_empty())
}

/// Read the calendar at `address`. What goes wrong never says the address.
pub fn fetch(address: &str) -> anyhow::Result<Feed> {
    let agent = crate::net::agent();
    let Ok(mut resp) = agent.get(address).call() else {
        bail!("the calendar could not be reached");
    };
    if !resp.status().is_success() {
        bail!("the calendar could not be read: {}", resp.status().as_u16());
    }
    let Ok(text) = resp.body_mut().with_config().limit(LIMIT).read_to_string() else {
        bail!("the calendar could not be read");
    };
    if !text.contains("BEGIN:VCALENDAR") {
        bail!("that address does not give a calendar");
    }
    let (entries, unread) = parse(&text);
    Ok(Feed {
        entries,
        unread,
        name: named(&text),
    })
}
