//! A calendar read from its address, as an iCalendar file (RFC 5545), and
//! what is coming up in it, each repeat worked out: to look at, never written
//! to. What a rule says that is not understood is counted, not guessed at, as
//! an event at the wrong time is worse than one not shown.

use anyhow::bail;
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
    fn on_at(&self, t: Timestamp) -> bool {
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
    let unfolded = text.replace("\r\n", "\n");
    let unfolded = unfolded.replace("\n ", "").replace("\n\t", "");
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
            let shown = at.date().max(today);
            let day = if shown == today {
                "Today".to_string()
            } else if today.tomorrow().is_ok_and(|t| t == shown) {
                "Tomorrow".to_string()
            } else {
                let away = (shown - today).get_days();
                let form = if away < 7 { "%a" } else { "%a %b %-d" };
                shown.strftime(form).to_string()
            };
            let when = if c.all_day {
                day
            } else {
                format!("{day} {}", at.strftime("%-I:%M %p"))
            };
            (when, c.title.clone())
        })
        .collect()
}

/// How far ahead is read for the strip: a week, and a day it may be
/// looked at through before it is read again.
pub const AHEAD_DAYS: i64 = 8;

/// What is coming up for `days` from now in the calendar the accounts' file at
/// `config` has set, in the user's zone, with how much of it could not be
/// read; none if it has none set.
pub fn read(config: &std::path::Path, days: i64) -> anyhow::Result<Option<(Vec<Coming>, Unread)>> {
    let Some(keeper) = crate::config::calendar(config)? else {
        return Ok(None);
    };
    if keeper == crate::secret::Keeper::Vault
        && !crate::vault::is_open()
        && !crate::vault::unlock_by_key().is_some_and(|r| r.is_ok())
    {
        bail!("the calendar's address is in the vault, which is locked");
    }
    let address = address(&crate::secret::find(keeper, HOST, USER)?)?;
    let (entries, unread) = fetch(&address)?;
    let now = Timestamp::now();
    let to = now.checked_add(jiff::SignedDuration::from_hours(days.saturating_mul(24)))?;
    Ok(Some((
        upcoming(&entries, now, to, &TimeZone::system()),
        unread,
    )))
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

/// Read the calendar at `address`. What goes wrong never says the address.
pub fn fetch(address: &str) -> anyhow::Result<(Vec<Entry>, Unread)> {
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
    Ok(parse(&text))
}
