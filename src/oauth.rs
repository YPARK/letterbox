//! Signing in to Microsoft 365 and Outlook.com as Microsoft has mail apps
//! do, on Microsoft's own page: OAuth 2.0, with PKCE. The password, and a
//! second factor, are given to Microsoft alone; letterbox is given a token
//! for IMAP and for sending that it renews, and that the account's owner, or its
//! organisation, may take back.
//!
//! Two ways are tried at once, whichever is done first: Microsoft's page in
//! the browser, which comes back to a server letterbox runs on this machine
//! for the minute it takes; and a code entered at Microsoft's device page,
//! on any device, as from a machine reached over ssh. letterbox signs in as
//! Thunderbird does, by Mozilla's public client ID, which organisations
//! mostly allow already; an account may give another.

use crate::config::Account;
use crate::secret;
use anyhow::{Context, bail};
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Thunderbird's client ID, which Mozilla publishes for mail apps.
pub const CLIENT_ID: &str = "9e5f94bc-e8a4-4e73-b8be-63364c29d753";
/// Any organisation's account, or a personal one.
pub const TENANT: &str = "common";
pub const IMAP_HOST: &str = "outlook.office365.com";
const SCOPE: &str = "https://outlook.office.com/IMAP.AccessAsUser.All \
     https://outlook.office.com/SMTP.Send offline_access";
/// How long signing in is waited for: in tests, not long.
const WAIT: Duration = Duration::from_secs(if cfg!(test) { 20 } else { 15 * 60 });
/// A token is renewed this long before it runs out.
const EARLY: Duration = Duration::from_secs(120);

/// `$LETTERBOX_MS_LOGIN`, for tests, else Microsoft's.
fn login() -> String {
    std::env::var("LETTERBOX_MS_LOGIN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://login.microsoftonline.com".to_string())
}

fn endpoint(tenant: &str, what: &str) -> String {
    format!("{}/{tenant}/oauth2/v2.0/{what}", login())
}

use crate::net::agent;

fn post(url: &str, form: &[(&str, &str)]) -> anyhow::Result<Value> {
    let mut resp = agent()
        .post(url)
        .send_form(form.iter().copied())
        .with_context(|| format!("cannot reach {url}"))?;
    let text = resp.body_mut().read_to_string()?;
    serde_json::from_str(&text).with_context(|| format!("{url} gave no JSON"))
}

/// The tokens of an account signed in.
#[derive(Clone)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub until: Instant,
}

/// Tokens, as they may be shown: not the tokens.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Tokens { .. }")
    }
}

fn tokens(v: &Value, refresh: Option<&str>) -> anyhow::Result<Tokens> {
    if let Some(e) = failure(v) {
        bail!("{e}");
    }
    let access = v["access_token"]
        .as_str()
        .context("Microsoft gave no access token")?;
    let refresh = v["refresh_token"]
        .as_str()
        .or(refresh)
        .context("Microsoft gave no refresh token")?;
    let secs = v["expires_in"].as_u64().unwrap_or(3600);
    Ok(Tokens {
        access: access.to_string(),
        refresh: refresh.to_string(),
        until: Instant::now() + Duration::from_secs(secs),
    })
}

/// What went wrong, as Microsoft says it: its first line, which names the
/// error, without the trace and time it adds.
fn failure(v: &Value) -> Option<String> {
    let code = v["error"].as_str()?;
    let said = v["error_description"]
        .as_str()
        .and_then(|d| d.lines().next())
        .unwrap_or(code);
    Some(crate::mail::clean(said))
}

fn random(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("the system's randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `s` as a URL's query has it.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                Some(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The domains of Microsoft's own mail, and of the providers that are not
/// Microsoft's, told without asking.
const MICROSOFT: &[&str] = &["outlook.com", "hotmail.com", "live.com", "msn.com"];
const NOT_MICROSOFT: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "yahoo.com",
    "ymail.com",
    "fastmail.com",
    "fastmail.fm",
    "proton.me",
    "protonmail.com",
];

/// Whether `email` signs in with Microsoft: Outlook.com's, or an
/// organisation's in Microsoft 365, as Microsoft says of its domain.
pub fn is_microsoft(email: &str) -> bool {
    let Some((_, domain)) = email.trim().rsplit_once('@') else {
        return false;
    };
    let domain = domain.to_lowercase();
    if MICROSOFT.contains(&domain.as_str()) {
        return true;
    }
    if NOT_MICROSOFT.contains(&domain.as_str()) || !domain.contains('.') {
        return false;
    }
    let url = format!(
        "{}/getuserrealm.srf?login={}&json=1",
        login(),
        encode(email.trim())
    );
    let Ok(mut resp) = agent().get(&url).call() else {
        return false;
    };
    resp.body_mut()
        .read_to_string()
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|v| matches!(v["NameSpaceType"].as_str(), Some("Managed" | "Federated")))
}

/// How to sign in, while letterbox waits: the page opened in the browser,
/// and the device page with its code, if Microsoft gave one.
#[derive(Clone, Debug)]
pub struct Waiting {
    pub page: String,
    pub device: Option<(String, String)>,
}

/// Open `url` in the browser.
pub fn browse(url: &str) {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Sign in as `email`, with the client `client` of the tenant `tenant`:
/// Microsoft's page opened by `open`, and its device page, both offered by
/// `show`, until one of them is done, `stop` is set, or a quarter of an
/// hour passes.
pub fn sign_in(
    email: &str,
    tenant: &str,
    client: &str,
    stop: &AtomicBool,
    open: fn(&str),
    show: impl FnOnce(&Waiting),
) -> anyhow::Result<Tokens> {
    let listener = TcpListener::bind("127.0.0.1:0").context("cannot listen for the browser")?;
    listener.set_nonblocking(true)?;
    let redirect = format!("http://localhost:{}", listener.local_addr()?.port());
    let verifier = random(48);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let state = random(16);
    let page = format!(
        "{}?client_id={}&response_type=code&response_mode=query&redirect_uri={}&scope={}\
         &code_challenge={challenge}&code_challenge_method=S256&state={state}&login_hint={}",
        endpoint(tenant, "authorize"),
        encode(client),
        encode(&redirect),
        encode(SCOPE),
        encode(email)
    );
    let mut device = post(
        &endpoint(tenant, "devicecode"),
        &[("client_id", client), ("scope", SCOPE)],
    )
    .ok()
    .filter(|v| failure(v).is_none());
    show(&Waiting {
        page: page.clone(),
        device: device.as_ref().and_then(|v| {
            Some((
                v["verification_uri"].as_str()?.to_string(),
                v["user_code"].as_str()?.to_string(),
            ))
        }),
    });
    // Not waited for: the browser may come back before it is done opening.
    let opening = page.clone();
    std::thread::spawn(move || open(&opening));
    let mut every = Duration::from_secs(
        device
            .as_ref()
            .and_then(|v| v["interval"].as_u64())
            .unwrap_or(5),
    );
    let mut next = Instant::now() + every;
    let until = Instant::now() + WAIT;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            bail!("signing in was stopped");
        }
        if let Ok((tcp, _)) = listener.accept()
            && let Some(code) = came_back(tcp, &state)?
        {
            let v = post(
                &endpoint(tenant, "token"),
                &[
                    ("client_id", client),
                    ("grant_type", "authorization_code"),
                    ("code", &code),
                    ("redirect_uri", &redirect),
                    ("code_verifier", &verifier),
                    ("scope", SCOPE),
                ],
            )?;
            return tokens(&v, None);
        }
        if let Some(d) = &device
            && Instant::now() >= next
        {
            let code = d["device_code"].as_str().unwrap_or_default().to_string();
            let v = post(
                &endpoint(tenant, "token"),
                &[
                    ("client_id", client),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", &code),
                ],
            )?;
            match v["error"].as_str() {
                None => return tokens(&v, None),
                Some("authorization_pending") => {}
                Some("slow_down") => every += Duration::from_secs(5),
                // Refused, or run out: the browser's way may still be taken.
                Some("authorization_declined") => bail!("signing in was declined"),
                Some(_) => device = None,
            }
            next = Instant::now() + every;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("no one signed in within a quarter of an hour")
}

/// Take the browser's request back from Microsoft: the code it brings, if
/// it is for this sign-in; the page shown in the browser says what came of
/// it.
fn came_back(tcp: std::net::TcpStream, state: &str) -> anyhow::Result<Option<String>> {
    tcp.set_nonblocking(false)?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = String::new();
    BufReader::new(&tcp).read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), decode(v)))
        .collect();
    let reply = |status: &str, said: &str| {
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>letterbox</title>\
             <body style='font: 16px system-ui; margin: 4em; color: #262b27'>\
             <h2>letterbox</h2><p>{said}</p>"
        );
        let _ = write!(
            &tcp,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    };
    if path != "/" {
        reply("404 Not Found", "Nothing here.");
        return Ok(None);
    }
    if params.get("state").map(String::as_str) != Some(state) {
        reply(
            "400 Bad Request",
            "This is not the sign-in letterbox is waiting for.",
        );
        return Ok(None);
    }
    if let Some(code) = params.get("code") {
        reply(
            "200 OK",
            "Signed in. You may close this tab, and go back to letterbox.",
        );
        return Ok(Some(code.clone()));
    }
    let said = crate::mail::clean(
        params
            .get("error_description")
            .or(params.get("error"))
            .map_or("Microsoft did not sign you in.", String::as_str),
    );
    reply("200 OK", &escape(&said));
    bail!("{said}")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renew the tokens with `refresh`.
pub fn renew(tenant: &str, client: &str, refresh: &str) -> anyhow::Result<Tokens> {
    let v = post(
        &endpoint(tenant, "token"),
        &[
            ("client_id", client),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("scope", SCOPE),
        ],
    )?;
    tokens(&v, Some(refresh))
        .context("cannot renew the sign-in: remove the account and add it again")
}

/// The tokens of the accounts signed in this run, by `user@host`.
static TOKENS: Mutex<Option<HashMap<String, Tokens>>> = Mutex::new(None);

fn key(a: &Account) -> String {
    format!("{}@{}", a.user(), a.host)
}

/// Keep `tokens` for `account`, for this run.
pub fn remember(account: &Account, tokens: Tokens) {
    let mut guard = TOKENS.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_default().insert(key(account), tokens);
}

/// An access token for `account`, renewed if it is about to run out; a
/// refresh token Microsoft gives in place of the old one is kept where the
/// old one was.
pub fn access(account: &Account) -> anyhow::Result<String> {
    let k = key(account);
    let cached = TOKENS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|t| t.get(&k).cloned());
    if let Some(t) = &cached
        && t.until > Instant::now() + EARLY
    {
        return Ok(t.access.clone());
    }
    let refresh = match (&cached, &account.password) {
        (Some(t), _) => t.refresh.clone(),
        (None, Some(r)) => r.clone(),
        (None, None) => secret::find(account.keep, &account.host, account.user())?,
    };
    let (tenant, client) = (account.tenant(), account.client());
    let fresh = renew(tenant, client, &refresh)?;
    if fresh.refresh != refresh && account.password_command.is_none() {
        let _ = secret::store(account.keep, &account.host, account.user(), &fresh.refresh);
    }
    let access = fresh.access.clone();
    remember(account, fresh);
    Ok(access)
}

/// XOAUTH2, as IMAP servers take a token: the user and the token, in the
/// form RFC 7628 and Microsoft give.
pub struct XOAuth2 {
    pub user: String,
    pub token: String,
}

impl XOAuth2 {
    /// The initial client response, before it is encoded for the server.
    pub fn secret(&self) -> String {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

impl imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&self, _: &[u8]) -> String {
        self.secret()
    }
}
