//! Google's calendars, by a sign-in with an app registered with Google: one a
//! build comes with, if any, or one the user registered, which needs no one's
//! leave but theirs. It reads, and asks to do nothing more.

use crate::agenda::{Entry, Feed, Reading, Unread};
use crate::oauth::{Loopback, WAIT, encode, post};
use anyhow::{Context, bail};
use jiff::{Timestamp, civil::Date};
use serde_json::Value;
use std::time::{Duration, Instant};

const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";
/// Of the user's calendars, and of a calendar's events, only what is used.
const LIST_FIELDS: &str = "nextPageToken,items(id,summary,summaryOverride,selected,hidden,deleted)";
const EVENT_FIELDS: &str =
    "nextPageToken,items(eventType,status,summary,start,end,attendees(self,responseStatus))";
/// The most pages of events asked for of one calendar.
const PAGES: usize = 20;

/// How the client's secret and the sign-in are kept, beside the addresses.
pub const SECRET: &str = "google-secret";
pub const REFRESH: &str = "google-refresh";

/// Where Google is.
pub struct Hosts {
    pub auth: String,
    pub token: String,
    pub api: String,
}

impl Default for Hosts {
    fn default() -> Self {
        Hosts {
            auth: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token: "https://oauth2.googleapis.com/token".to_string(),
            api: "https://www.googleapis.com/calendar/v3".to_string(),
        }
    }
}

/// An app registered with Google, as the file Google gives for it says. It may
/// not be printed, as it holds a secret.
pub struct App {
    pub client: String,
    pub secret: String,
}

impl App {
    /// The app in the file Google gives, `text`.
    pub fn from_json(text: &str) -> anyhow::Result<App> {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            bail!("that is not the file Google gives for a client");
        };
        let Some(app) = v.get("installed") else {
            bail!("that file is not for a client of the type Desktop app");
        };
        let field = |name: &str| app[name].as_str().unwrap_or_default().trim().to_string();
        let (client, secret) = (field("client_id"), field("client_secret"));
        if !client.ends_with(CLIENT_END) {
            bail!("the client's ID in that file does not end in {CLIENT_END}");
        }
        if secret.is_empty() {
            bail!(
                "that file has no secret, as Google gives it only as a client is made: make \
                 another, and download its file there"
            );
        }
        Ok(App { client, secret })
    }

    /// The app letterbox comes with, if this build has one.
    pub fn built_in() -> Option<App> {
        App::from_json(include_str!("google_app.json")).ok()
    }
}

/// What a client's ID ends in.
const CLIENT_END: &str = ".apps.googleusercontent.com";

/// What went wrong, as Google says it, in what it gave.
fn failure(v: &Value) -> Option<String> {
    crate::oauth::failure(v).or_else(|| v["error"]["message"].as_str().map(crate::mail::clean))
}

/// Sign in, Google's page given to `open`, until someone has or a quarter of
/// an hour passes: what lets letterbox read from then on.
pub fn sign_in(
    hosts: &Hosts,
    app: &App,
    open: impl FnOnce(&str) + Send + 'static,
) -> anyhow::Result<String> {
    let back = Loopback::open("127.0.0.1")?;
    // Offline, and asked each time: what lets it read later comes only so.
    let page = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&access_type=offline\
         &prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
        hosts.auth,
        encode(&app.client),
        encode(&back.redirect),
        encode(SCOPE),
        back.challenge,
        back.state,
    );
    std::thread::spawn(move || open(&page));
    let until = Instant::now() + WAIT;
    while Instant::now() < until {
        if let Some(code) = back.poll()? {
            let v = post(
                &hosts.token,
                &[
                    ("client_id", &app.client),
                    ("client_secret", &app.secret),
                    ("grant_type", "authorization_code"),
                    ("code", &code),
                    ("redirect_uri", &back.redirect),
                    ("code_verifier", &back.verifier),
                ],
            )?;
            if let Some(why) = failure(&v) {
                bail!("{why}");
            }
            return v["refresh_token"]
                .as_str()
                .map(str::to_string)
                .context("Google gave nothing to read with later");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("no one signed in within a quarter of an hour")
}

/// What lets letterbox read now, from what lets it read later.
fn access(hosts: &Hosts, app: &App, refresh: &str) -> anyhow::Result<String> {
    let v = post(
        &hosts.token,
        &[
            ("client_id", &app.client),
            ("client_secret", &app.secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
        ],
    )?;
    if let Some(why) = failure(&v) {
        // Only this means the sign-in has died: anything else may pass.
        if v["error"].as_str() == Some("invalid_grant") {
            bail!("refused: {why}: lbx calendar google signs in again");
        }
        bail!("refused: {why}");
    }
    v["access_token"]
        .as_str()
        .map(str::to_string)
        .context("gave nothing to read with")
}

fn get(agent: &ureq::Agent, url: &str, token: &str) -> anyhow::Result<Value> {
    let Ok(mut resp) = agent
        .get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
    else {
        bail!("could not be reached");
    };
    let text = resp.body_mut().read_to_string()?;
    let v: Value = serde_json::from_str(&text).context("gave no JSON")?;
    if let Some(why) = failure(&v) {
        bail!("{why}");
    }
    Ok(v)
}

/// Every page of `url`, which has a query already, as far as `PAGES`.
fn pages(agent: &ureq::Agent, url: &str, token: &str) -> anyhow::Result<Vec<Value>> {
    let mut all = Vec::new();
    let mut at = url.to_string();
    for _ in 0..PAGES {
        let v = get(agent, &at, token)?;
        let next = v["nextPageToken"].as_str().map(encode);
        all.push(v);
        let Some(next) = next else { break };
        at = format!("{url}&pageToken={next}");
    }
    Ok(all)
}

/// The calendars in a page of the user's list that they have chosen to see in
/// Google Calendar: what each is known by, and is called.
pub fn shown(page: &Value) -> Vec<(String, String)> {
    let items = page["items"].as_array().map_or(&[][..], Vec::as_slice);
    items
        .iter()
        .filter(|c| c["selected"].as_bool() == Some(true))
        .filter(|c| c["hidden"].as_bool() != Some(true) && c["deleted"].as_bool() != Some(true))
        .filter_map(|c| {
            let name = c["summaryOverride"].as_str().or(c["summary"].as_str())?;
            Some((c["id"].as_str()?.to_string(), crate::mail::clean(name)))
        })
        .collect()
}

/// The event `e` as letterbox has one; none if it cannot be read.
fn entry(e: &Value) -> Option<Entry> {
    let title = crate::mail::clean(e["summary"].as_str().unwrap_or("Busy"));
    if let (Some(start), Some(end)) = (
        e["start"]["dateTime"].as_str(),
        e["end"]["dateTime"].as_str(),
    ) {
        let (start, end): (Timestamp, Timestamp) = (start.parse().ok()?, end.parse().ok()?);
        return Some(Entry::timed(title, start, end));
    }
    let first: Date = e["start"]["date"].as_str()?.parse().ok()?;
    let after: Date = e["end"]["date"].as_str()?.parse().ok()?;
    Some(Entry::days(title, first, after))
}

/// The events in a page of a calendar, but those struck out, those the user said
/// no to, and where they say they work. Those it cannot read are counted.
pub fn events(page: &Value) -> (Vec<Entry>, usize) {
    let items = page["items"].as_array().map_or(&[][..], Vec::as_slice);
    let mut out = Vec::new();
    let mut unread = 0;
    for e in items {
        let declined = e["attendees"].as_array().is_some_and(|all| {
            all.iter().any(|a| {
                a["self"].as_bool() == Some(true)
                    && a["responseStatus"].as_str() == Some("declined")
            })
        });
        if declined
            || e["status"].as_str() == Some("cancelled")
            || e["eventType"].as_str() == Some("workingLocation")
        {
            continue;
        }
        match entry(e) {
            Some(e) => out.push(e),
            None => unread += 1,
        }
    }
    (out, unread)
}

/// One calendar, from every page at `url`.
fn read_one(agent: &ureq::Agent, url: &str, token: &str) -> anyhow::Result<Feed> {
    let mut entries = Vec::new();
    let mut unread = Unread::default();
    for page in pages(agent, url, token)? {
        let (found, out) = events(&page);
        entries.extend(found);
        unread.events += out;
    }
    Ok(Feed {
        entries,
        unread,
        name: None,
    })
}

/// Every calendar the user has chosen to see, each read at once, from `from` to
/// `to`.
pub fn read(
    hosts: &Hosts,
    app: &App,
    refresh: &str,
    from: Timestamp,
    to: Timestamp,
) -> anyhow::Result<Vec<Reading>> {
    let token = access(hosts, app, refresh)?;
    let agent = crate::net::agent();
    let list = format!(
        "{}/users/me/calendarList?maxResults=250&fields={}",
        hosts.api,
        encode(LIST_FIELDS)
    );
    let calendars: Vec<_> = pages(&agent, &list, &token)?
        .iter()
        .flat_map(shown)
        .collect();
    let url_of = |id: &str| {
        format!(
            "{}/calendars/{}/events?singleEvents=true&orderBy=startTime&maxResults=250\
             &timeMin={}&timeMax={}&fields={}",
            hosts.api,
            encode(id),
            encode(&from.to_string()),
            encode(&to.to_string()),
            encode(EVENT_FIELDS),
        )
    };
    Ok(std::thread::scope(|scope| {
        let reading: Vec<_> = calendars
            .iter()
            .map(|(id, _)| {
                let (url, agent, token) = (url_of(id), &agent, &token);
                scope.spawn(move || read_one(agent, &url, token))
            })
            .collect();
        calendars
            .iter()
            .zip(reading)
            .map(|((id, name), read)| Reading {
                id: format!("google:{id}"),
                name: Some(name.clone()),
                feed: read
                    .join()
                    .unwrap_or_else(|_| bail!("could not be read"))
                    .map_err(|e| format!("{e:#}")),
            })
            .collect()
    }))
}
