//! letterbox: mail in the terminal. IMAP accounts, read in a TUI laid out
//! as a mail app's, or from the shell, a folder or a message at a time. The
//! program is `letterbox`, and `lbx` for short, each of them `run`.

mod app;
mod compose;
mod config;
mod confirm;
mod demo;
mod edit;
mod filter;
mod forest;
mod imap;
mod index;
mod keymap;
mod mail;
mod map;
mod mcp;
mod model;
mod mosaic;
mod net;
mod oauth;
#[cfg(feature = "pages")]
mod pages;
mod papers;
mod paperstack;
mod pick;
mod secret;
mod setup;
mod smtp;
mod store;
mod theme;
mod trash;
mod ui;
mod vault;
mod worker;

use anyhow::{Context, bail};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;
use store::Store;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// The accounts: a TOML file of [[account]] tables
    #[arg(long, env = "LETTERBOX_CONFIG", default_value_os_t = config::default_path(), global = true)]
    config: PathBuf,
    /// Made-up mail to try letterbox with, instead of your accounts
    #[arg(long, global = true)]
    demo: bool,
    /// The TUI's colours: a light page, or the terminal's own dark one
    #[arg(
        long,
        env = "LETTERBOX_THEME",
        value_enum,
        default_value_t,
        global = true
    )]
    theme: theme::Name,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Args)]
struct Which {
    /// The account, by its name; else the first
    #[arg(long, short)]
    account: Option<String>,
    /// The folder, by its name on the server
    #[arg(long, short, default_value = "INBOX")]
    folder: String,
}

#[derive(Subcommand)]
enum Command {
    /// The accounts set up, and where; add and remove them
    Accounts {
        #[command(subcommand)]
        action: Option<AccountAction>,
        /// Sign in to each, and say what went wrong
        #[arg(long)]
        check: bool,
    },
    /// An account's folders, and how many messages each holds
    Folders {
        #[arg(long, short)]
        account: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// A folder's newest messages
    List {
        #[command(flatten)]
        which: Which,
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// A message, by its UID in the folder; it is not marked read
    Read {
        uid: u32,
        #[command(flatten)]
        which: Which,
        #[arg(long)]
        json: bool,
        /// Its bytes, as sent
        #[arg(long, conflicts_with = "json")]
        raw: bool,
    },
    /// The messages in a folder with all of these words, on the server
    Search {
        #[arg(required = true)]
        words: Vec<String>,
        #[command(flatten)]
        which: Which,
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Move every password letterbox keeps, and the map's key, to the
    /// system's store or to letterbox's vault; each is forgotten where it
    /// was only once it is kept in the other
    Keep {
        #[arg(value_enum)]
        to: secret::Keeper,
    },
    /// letterbox's vault: give it a new passphrase, or a key file instead
    Vault {
        #[command(subcommand)]
        action: VaultAction,
    },
    /// The map of every account's mail: lay it out again, or say its key
    Map {
        #[command(subcommand)]
        action: MapAction,
    },
    /// What letterbox has learnt of the mail you throw away: how well it
    /// would have done, or learnt from the Trash and your labels too
    Filter {
        #[command(subcommand)]
        action: FilterAction,
    },
    /// Serve the mail to an AI agent over MCP, on stdin and stdout: its
    /// program, as Cursor or Claude Code, runs `letterbox mcp`
    Mcp,
    /// The TUI's screen after the keys given, as text: `j <enter>`
    #[command(hide = true)]
    Screen {
        #[arg(long, default_value_t = 140)]
        width: u16,
        #[arg(long, default_value_t = 40)]
        height: u16,
        /// As an SVG, with its colours
        #[arg(long)]
        svg: bool,
        keys: Vec<String>,
    },
}

#[derive(Subcommand)]
enum AccountAction {
    /// Add an account, asking for what is not given; it signs in first, and
    /// its password is kept in the keychain, not the file
    Add {
        #[arg(long)]
        email: Option<String>,
        /// What it is called here; its address's domain, else
        #[arg(long)]
        name: Option<String>,
        /// The IMAP server; the provider's, or imap. and the domain, else
        #[arg(long)]
        host: Option<String>,
        #[arg(long, default_value_t = 993)]
        port: u16,
        /// The name signed in with, if not the address
        #[arg(long)]
        user: Option<String>,
        /// The server mail is sent through, and its port after a colon if
        /// not 587; Microsoft's for an account signed in there, the
        /// provider's, or smtp. and the domain, else
        #[arg(long)]
        outgoing: Option<String>,
        /// A command that prints the password, instead of keeping it
        #[arg(long, conflicts_with = "keep")]
        password_command: Option<String>,
        /// Where to keep the password, or Microsoft's token: the system's
        /// store, unless the vault is asked for
        #[arg(long, value_enum)]
        keep: Option<secret::Keeper>,
        /// Sign in with a password, or on Microsoft's page; Microsoft's for
        /// an address Microsoft says is its own
        #[arg(long, value_enum)]
        auth: Option<SignIn>,
    },
    /// Remove an account; its mail stays on its server
    Remove {
        name: String,
        /// Keep the password letterbox kept for it
        #[arg(long)]
        keep_password: bool,
    },
    /// Call an account something else; only letterbox's name for it changes
    Rename { name: String, new: String },
}

#[derive(Subcommand)]
enum VaultAction {
    /// A new passphrase for it, asked for as letterbox starts; or none, its
    /// key a file beside it only you may read, never asked for
    Passphrase,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Subcommand)]
enum FilterAction {
    /// How it would have done: each decision said before it was learnt,
    /// from those before it alone; what it caught, and what you kept that
    /// it would have thrown away
    Check {
        /// How much of each weight is kept a decision, to try another
        #[arg(long, hide = true)]
        decay: Option<f32>,
    },
    /// Learn from mail already in each account's Trash, thrown away
    /// unread, as thrown away, and filed under your labels, as kept, the
    /// newest of each; then check
    Learn {
        /// How many of each, at most, from each place
        #[arg(long, default_value_t = 500)]
        limit: usize,
    },
    /// Forget what was learnt from the Trash and your labels: only your
    /// own decisions are kept; then check
    Forget,
    /// Throw new mail away on its own, when 95% sure, or not: told of as
    /// letterbox opens, and by B, never learnt from
    Auto {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// What was thrown away on its own, and why, and what was learnt since
    Debrief {
        /// As far back as this many days
        #[arg(long, default_value_t = 7)]
        days: i64,
    },
}

#[derive(Subcommand)]
enum MapAction {
    /// Lay the map out again by every lens, from nothing, as its seed has
    /// it: the same mail, the same map
    Rebuild {
        /// Read every account's mail again first: what the servers said is
        /// let go, and what only letterbox knows is kept
        #[arg(long)]
        from_servers: bool,
    },
    /// How well the map would file what you filed: each message under a
    /// label, its own hidden, put back by the label nearest it
    Check,
    /// The map's key: where it is kept
    Key {
        /// Print its secret, as age and rage read a key file
        #[arg(long)]
        export: bool,
    },
}

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
enum SignIn {
    Password,
    Microsoft,
}

/// Ask on the terminal, with `default` taken for nothing typed.
fn ask(question: &str, default: &str) -> anyhow::Result<String> {
    use std::io::Write;
    if default.is_empty() {
        eprint!("{question}: ");
    } else {
        eprint!("{question} [{default}]: ");
    }
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let line = line.trim();
    Ok(if line.is_empty() { default } else { line }.to_string())
}

/// Ask for a password on the terminal, not shown as it is typed; read
/// from the input as a line where it is not a terminal.
fn ask_password(question: &str) -> anyhow::Result<String> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::crossterm::terminal;
    use std::io::{IsTerminal, Write};
    eprint!("{question}: ");
    std::io::stderr().flush()?;
    if !std::io::stdin().is_terminal() {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        eprintln!();
        return Ok(line.trim_end_matches(['\r', '\n']).to_string());
    }
    terminal::enable_raw_mode()?;
    let mut password = String::new();
    let done = loop {
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Enter => break Ok(()),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    break Err(anyhow::anyhow!("stopped"));
                }
                KeyCode::Esc => break Err(anyhow::anyhow!("stopped")),
                KeyCode::Backspace => {
                    password.pop();
                }
                KeyCode::Char(c) => password.push(c),
                _ => {}
            },
            Ok(_) => {}
            Err(e) => break Err(e.into()),
        }
    };
    terminal::disable_raw_mode()?;
    eprintln!();
    done.map(|()| password)
}

fn add_account(args: &Args, action: &AccountAction) -> anyhow::Result<()> {
    let AccountAction::Add {
        email,
        name,
        host,
        port,
        user,
        outgoing,
        password_command,
        keep,
        auth,
    } = action
    else {
        unreachable!("only adding")
    };
    let email = match email {
        Some(e) => e.clone(),
        None => ask("Email", "")?,
    };
    if !email.contains('@') {
        bail!("{email} is not an address");
    }
    let microsoft = match auth {
        Some(a) => *a == SignIn::Microsoft,
        None => password_command.is_none() && oauth::is_microsoft(&email),
    };
    let (guessed, note) = if microsoft {
        (oauth::IMAP_HOST.to_string(), Some(config::MICROSOFT_NOTE))
    } else {
        config::guess(&email)
    };
    let name = match name {
        Some(n) => n.clone(),
        None => ask("Name", &setup::name_for(&email))?,
    };
    let host = match host {
        Some(h) => h.clone(),
        None => ask("IMAP server", &guessed)?,
    };
    let auth = if microsoft {
        config::Auth::Microsoft
    } else {
        config::Auth::Password
    };
    let outgoing = match outgoing {
        Some(o) => o.clone(),
        None => ask(
            "Outgoing server",
            &config::guess_outgoing(&email, &host, auth),
        )?,
    };
    let Some((smtp_host, smtp_port)) = setup::outgoing(&outgoing) else {
        bail!("{outgoing}: a port after the colon is a number, as smtp.example.com:465");
    };
    let mut account = setup::account(
        &name,
        &email,
        &host,
        *port,
        user.as_deref().unwrap_or(&email),
    );
    account.smtp_host = smtp_host;
    account.smtp_port = smtp_port;
    let keepers = secret::keepers();
    let keeper = match keep {
        Some(k) if keepers.contains(k) => *k,
        Some(k) => bail!("there is no {} here", k.name()),
        None => keepers[0],
    };
    if keeper == secret::Keeper::Vault && password_command.is_none() {
        open_vault(true)?;
    }
    if let Some(c) = password_command {
        account.password_command = Some(c.clone());
    } else if microsoft {
        account.auth = config::Auth::Microsoft;
        eprintln!("{}", config::MICROSOFT_NOTE);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let tokens = oauth::sign_in(
            &email,
            account.tenant(),
            account.client(),
            &stop,
            oauth::browse,
            |w| {
                eprintln!(
                    "\nSign in on Microsoft's page, opened in your browser:\n  {}",
                    w.page
                );
                if let Some((page, code)) = &w.device {
                    eprintln!("or, on any device, open {page} and enter {code}");
                }
                eprintln!("\nwaiting…");
            },
        )?;
        account.password = Some(tokens.refresh.clone());
        oauth::remember(&account, tokens);
    } else {
        if let Some(note) = note {
            eprintln!("{note}");
        }
        account.password = Some(ask_password("Password")?);
    }
    eprintln!("signing in to {}…", account.host);
    let (_, folders) = setup::add(&args.config, account, keeper, secret::store)?;
    println!(
        "added {name}: {} folders, in {}",
        folders.len(),
        args.config.display()
    );
    Ok(())
}

/// The accounts' stores, by name: the demo's, or those set up, the vault
/// opened first if any keeps its password there.
fn stores(args: &Args) -> anyhow::Result<Vec<(String, Box<dyn Store>)>> {
    if args.demo {
        return Ok(vec![("Demo".to_string(), Box::new(demo::Demo::new()))]);
    }
    let accounts = config::load(&args.config)?;
    if accounts.iter().any(in_vault) {
        open_vault(false)?;
    }
    Ok(accounts
        .into_iter()
        .map(|a| {
            (
                a.name.clone(),
                Box::new(imap::Imap::new(a)) as Box<dyn Store>,
            )
        })
        .collect())
}

/// How well the map would file what was filed, as `letterbox map check`
/// says it: by each way of reading the mail.
fn map_check(index: &index::Index) -> String {
    let given: Vec<(&index::Record, Vec<String>, bool)> = index
        .records
        .values()
        .map(|r| (r, index.labels(&r.key), index.system(&r.key).sent))
        .collect();
    let corpus = model::Corpus::read(&given, index::Lens::Both);
    let said = |c: model::LabelCheck| {
        let pct = if c.total == 0 {
            0.0
        } else {
            100.0 * c.hits as f32 / c.total as f32
        };
        let right = if c.said == 0 {
            0.0
        } else {
            100.0 * c.hits as f32 / c.said as f32
        };
        format!(
            "{} of {} ({pct:.0}%), said for {} and right for {right:.0}% of those",
            c.hits, c.total, c.said
        )
    };
    let mut out = format!(
        "{} messages; each filed one put back under a label of its own, its own hidden:\n",
        given.len()
    );
    let within = |deg: f32| corpus.label_check_nearest(&given, corpus.base(), model::VOTERS, deg);
    for (how, check) in [
        (
            format!(
                "the nearest filed mail voting, within {}° (as it is)",
                model::VOTED_WITHIN
            ),
            within(model::VOTED_WITHIN),
        ),
        ("  within 30°".to_string(), within(30.0)),
        ("  within 60°".to_string(), within(60.0)),
        ("  at any distance".to_string(), within(90.0)),
        (
            "the label whose mean is nearest".to_string(),
            corpus.label_check(&given, corpus.base()),
        ),
        (
            "always the commonest label".to_string(),
            model::commonest_label_check(&given),
        ),
    ] {
        out.push_str(&format!("  {how}: {}\n", said(check)));
    }
    out
}

/// The map's index, its key found where it is kept: the vault opened
/// first if an account keeps its password there.
fn open_index(args: &Args) -> anyhow::Result<index::Index> {
    if args.demo {
        return Ok(index::Index::memory());
    }
    if config::load(&args.config)?.iter().any(in_vault) {
        open_vault(false)?;
    }
    index::Index::open(
        &index::key::dir(),
        secret::keepers,
        secret::find,
        secret::store,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Learn from what each of `stores` has in its Trash, never read, and from
/// what you filed under your labels, read again first as the index now
/// reads a message; how many were thrown away, kept, and not learnt before.
fn learn(
    index: &mut index::Index,
    stores: &mut [(String, Box<dyn Store>)],
    limit: usize,
    now: i64,
) -> anyhow::Result<(usize, usize, usize)> {
    let keys: Vec<index::Key> = index
        .kept_seeds(limit)
        .into_iter()
        .map(|(_, t)| t.key)
        .collect();
    let shared = std::sync::Mutex::new(std::mem::replace(index, index::Index::memory()));
    let mut thrown = Vec::new();
    for (name, store) in stores.iter_mut() {
        if let Err(e) = index::sync::refresh(store.as_mut(), &shared, &keys, now) {
            eprintln!("{name}: what you filed cannot be read again: {e:#}");
        }
        let known = shared.lock().unwrap_or_else(|e| e.into_inner()).known();
        match trash_seeds(store.as_mut(), &known, limit, now) {
            Ok(found) => thrown.extend(found),
            Err(e) => eprintln!("{name}: its Trash cannot be read: {e:#}"),
        }
    }
    *index = shared.into_inner().unwrap_or_else(|e| e.into_inner());
    let mut seeds = index.kept_seeds(limit);
    let (kept, n_thrown) = (seeds.len(), thrown.len());
    seeds.extend(thrown);
    // Oldest first, as if decided as they came.
    seeds.sort_by_key(|(at, t)| (*at, t.key.clone()));
    let learnt = index.seed(seeds.into_iter().map(|(_, t)| t).collect())?;
    Ok((n_thrown, kept, learnt))
}

/// The newest `limit` messages of the account's Trash never read, as thrown
/// away, each with when it was sent. Mail read and then thrown away was
/// dealt with, not unwanted: it is left out.
fn trash_seeds(
    store: &mut dyn Store,
    known: &filter::Known,
    limit: usize,
    now: i64,
) -> anyhow::Result<Vec<(i64, filter::Taught)>> {
    let trash = store
        .folders()?
        .into_iter()
        .find(|f| f.role == mail::Role::Trash)
        .map(|f| f.path)
        .or_else(|| store.trash().map(str::to_string));
    let Some(trash) = trash else {
        return Ok(Vec::new());
    };
    let ident = store.ident();
    let uids = store.uids(&trash)?;
    let mut seeds = Vec::new();
    // Newest first, as far back as it takes to find `limit` never read.
    for chunk in uids.rchunks(100) {
        if seeds.len() >= limit {
            break;
        }
        for got in store.fetch_index(&trash, chunk)? {
            if got.flags.seen {
                continue;
            }
            let r = index::Record::of(&ident, &trash, 0, &got, now);
            seeds.push((
                r.date.unwrap_or(now),
                filter::Taught {
                    key: r.key.clone(),
                    thrown: true,
                    tokens: filter::Filter::tokens(&r, known),
                    message_id: r.message_id.clone(),
                    seeded: true,
                    ..Default::default()
                },
            ));
        }
    }
    seeds.truncate(limit);
    Ok(seeds)
}

/// Turn throwing new mail away on its own on, for mail come from `now`, or
/// off, as the file at `path` keeps it.
fn set_auto(path: &std::path::Path, on: bool, now: i64) -> anyhow::Result<()> {
    let mut a = filter::Auto::read(&std::fs::read_to_string(path).unwrap_or_default(), now);
    if on && !a.on {
        a.since = now;
    }
    a.on = on;
    std::fs::write(path, a.to_string()).with_context(|| format!("writing {}", path.display()))
}

/// What was thrown away on its own after `since`, and why, and what was
/// learnt since, as `letterbox filter debrief` says it.
fn debrief_text(index: &index::Index, since: i64) -> String {
    use std::fmt::Write;
    let d = index.debrief(since);
    let mut out = String::new();
    let _ = writeln!(out, "thrown away on its own: {}", d.thrown.len());
    let now = jiff::Zoned::now();
    for t in &d.thrown {
        let back = if t.back { "  · put back" } else { "" };
        let _ = writeln!(
            out,
            "  {:>8}  {:>3.0}%  {}  {}{back}",
            ui::short_date(t.at, &now),
            t.odds * 100.0,
            mail::clean(&t.from),
            mail::clean(&t.subject)
        );
        let _ = writeln!(out, "                   {}", t.why.join(" · "));
    }
    let _ = writeln!(
        out,
        "\n{} of your decisions learnt since: {} kept, {} thrown away",
        d.decided, d.kept, d.thrown_by_you
    );
    out
}

/// How what was learnt would have done, as `letterbox filter check` says it.
fn check(index: &index::Index, decay: f32) -> String {
    use std::fmt::Write;
    let taught = &index.filter.taught;
    let said = filter::Filter::check_with(taught, decay);
    let mut out = String::new();
    let seeded = taught.iter().filter(|t| t.seeded).count();
    let _ = writeln!(
        out,
        "{} decisions: {} thrown away, {} kept; {} of them yours, {} read from the Trash and your labels",
        taught.len(),
        taught.iter().filter(|t| t.thrown).count(),
        taught.iter().filter(|t| !t.thrown).count(),
        taught.len() - seeded,
        seeded,
    );
    if said.is_empty() {
        let _ = writeln!(
            out,
            "nothing to grade yet: it says nothing until it has learnt {} of each",
            filter::READY
        );
        return out;
    }
    let _ = writeln!(
        out,
        "{} graded, each before it was learnt, by what was learnt before it\n",
        said.len()
    );
    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f32 / b as f32
        }
    };
    for level in [0.7, 0.8, 0.9, 0.95] {
        let g = filter::grade(&said, level);
        let mark = if level == filter::SURE {
            "  (as it is)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "sure to {:.0}%: caught {} of {} thrown away ({:.0}%); would have thrown away {} of {} kept ({:.1}%){mark}",
            level * 100.0,
            g.caught,
            g.caught + g.missed,
            pct(g.caught, g.caught + g.missed),
            g.wrong,
            g.wrong + g.right,
            pct(g.wrong, g.wrong + g.right),
        );
    }
    let mut wrong: Vec<&filter::Said> = said
        .iter()
        .filter(|s| !s.thrown && s.odds >= filter::SURE)
        .collect();
    wrong.sort_by(|a, b| b.odds.total_cmp(&a.odds));
    if !wrong.is_empty() {
        let _ = writeln!(out, "\nkept, but it would have thrown away:");
        for s in wrong.iter().take(30) {
            let subject = index
                .records
                .get(&s.key)
                .map(|r| r.subject.clone())
                .unwrap_or_else(|| "(no longer here)".to_string());
            let _ = writeln!(
                out,
                "  {:>3.0}%  {}  {}{}",
                s.odds * 100.0,
                s.from,
                mail::clean(&subject),
                if s.seeded { "  · from a label" } else { "" }
            );
        }
        if wrong.len() > 30 {
            let _ = writeln!(out, "  and {} more", wrong.len() - 30);
        }
    }
    out
}

fn in_vault(a: &config::Account) -> bool {
    a.keep == secret::Keeper::Vault && a.password_command.is_none()
}

/// The stores for the TUI: as `stores`, but the vault left locked if its
/// passphrase is not given, its accounts saying so; and passwords printed by
/// a command found before the TUI takes the terminal, as such a command
/// may ask for a passphrase on it, as `pass` does by gpg's pinentry.
fn stores_for_tui(args: &Args) -> anyhow::Result<Vec<(String, Box<dyn Store>)>> {
    if args.demo {
        return stores(args);
    }
    let mut accounts = config::load(&args.config)?;
    if accounts.iter().any(in_vault)
        && let Err(e) = open_vault(false)
    {
        eprintln!("letterbox: {e:#}");
    }
    for a in accounts.iter_mut().filter(|a| a.password_command.is_some()) {
        match a.password() {
            Ok(p) => a.password = Some(p),
            Err(e) => a.missing = Some(format!("{e:#}")),
        }
    }
    Ok(accounts
        .into_iter()
        .map(|a| {
            (
                a.name.clone(),
                Box::new(imap::Imap::new(a)) as Box<dyn Store>,
            )
        })
        .collect())
}

/// Open the vault, asking for its passphrase: three times, at most; or, if
/// there is none yet and `create`, make one, its passphrase asked for twice.
fn open_vault(create: bool) -> anyhow::Result<()> {
    if vault::is_open() {
        return Ok(());
    }
    // By its key file; one that no longer opens it, as after its
    // passphrase was changed, asks for the passphrase instead.
    if let Some(Ok(())) = vault::unlock_by_key() {
        return Ok(());
    }
    if !vault::exists() {
        if !create {
            return Ok(());
        }
        eprintln!(
            "A new vault, {}: a passphrase is asked for each time letterbox starts; \
             with none, its key is kept in a file beside it, which only you may read, \
             and nothing is ever asked.",
            vault::path().display()
        );
        let first = ask_password("New vault passphrase, or nothing for a key file")?;
        if first.is_empty() {
            vault::make_key()?;
            eprintln!("its key: {}", vault::key_path().display());
            return Ok(());
        }
        if ask_password("Again")? != first {
            bail!("the two passphrases differ");
        }
        return vault::unlock(&first);
    }
    for _ in 0..3 {
        match vault::unlock(&ask_password("Vault passphrase")?) {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("{e:#}"),
        }
    }
    bail!("the vault stays locked")
}

/// The store of the account named, or of the first.
fn store(args: &Args, name: Option<&str>) -> anyhow::Result<Box<dyn Store>> {
    let mut all = stores(args)?;
    if all.is_empty() {
        bail!(
            "no accounts in {}: `letterbox accounts` shows how to add one",
            args.config.display()
        );
    }
    let at = match name {
        None => 0,
        Some(n) => all
            .iter()
            .position(|(a, _)| a.eq_ignore_ascii_case(n))
            .ok_or_else(|| anyhow::anyhow!("no account named {n}"))?,
    };
    Ok(all.swap_remove(at).1)
}

fn print_list(list: &[mail::Summary], json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(list)?);
        return Ok(());
    }
    let now = jiff::Zoned::now();
    for m in list {
        let mark = if m.flags.seen { ' ' } else { '●' };
        let flag = if m.flags.flagged { '⚑' } else { ' ' };
        let date = m.date.map(|d| ui::short_date(d, &now)).unwrap_or_default();
        println!(
            "{:>7} {mark}{flag} {date:<10}  {:<24}  {}",
            m.uid,
            truncate(&m.from, 24),
            m.subject
        );
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

/// The program, by whichever of its names it was run: `letterbox`, or
/// `lbx` for short.
pub fn run() -> anyhow::Result<()> {
    let name = std::env::args_os()
        .next()
        .and_then(|a| {
            std::path::Path::new(&a)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "letterbox".to_string());
    let matches = Args::command().bin_name(name).get_matches();
    let args = Args::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    // What went wrong is printed as it is: what a server says in it, with
    // nothing a terminal would take as a command.
    command(&args).map_err(|e| anyhow::anyhow!("{}", mail::clean_lines(&format!("{e:?}"))))
}

fn command(args: &Args) -> anyhow::Result<()> {
    match &args.command {
        None => {
            let stores = stores_for_tui(args)?;
            let index = if args.demo {
                Ok(Some(index::Index::memory()))
            } else {
                index::Index::open_made(
                    &index::key::dir(),
                    secret::keepers,
                    secret::find,
                    secret::store,
                )
            };
            let mut app = app::App::with_index(stores, args.config.clone(), index);
            app.theme = theme::Theme::of(args.theme);
            if args.demo {
                app.fix_accounts();
            } else {
                app.look_file = Some(args.config.with_file_name("look"));
                app.recall_look();
                // Thrown away on its own, unless turned off: told of first.
                let path = args.config.with_file_name("filter");
                let now = jiff::Timestamp::now().as_second();
                app.auto =
                    filter::Auto::read(&std::fs::read_to_string(&path).unwrap_or_default(), now);
                app.filter_file = Some(path);
                // Written at once, so that the first time it is for mail
                // come from now on.
                app.remember_auto();
                app.debrief_on_start();
            }
            app::run(&mut app)
        }
        Some(Command::Accounts {
            action: Some(action @ AccountAction::Add { .. }),
            ..
        }) => add_account(args, action),
        Some(Command::Accounts {
            action:
                Some(AccountAction::Remove {
                    name,
                    keep_password,
                }),
            ..
        }) => {
            let account = config::load(&args.config)?
                .into_iter()
                .find(|a| a.name.eq_ignore_ascii_case(name));
            if !keep_password && account.as_ref().is_some_and(in_vault) {
                open_vault(false)?;
            }
            let (a, forgotten) = setup::remove(&args.config, name, *keep_password)?;
            println!("removed {}; its mail stays on {}", a.name, a.host);
            if !forgotten && !keep_password {
                println!("its password is still in the vault, which is locked");
            }
            Ok(())
        }
        Some(Command::Accounts {
            action: Some(AccountAction::Rename { name, new }),
            ..
        }) => {
            let new = config::rename(&args.config, name, new)?;
            println!("renamed {name} to {new}");
            Ok(())
        }
        Some(Command::Accounts {
            action: None,
            check,
        }) => {
            let accounts = config::load(&args.config)?;
            if accounts.is_empty() {
                println!(
                    "no accounts yet: `letterbox accounts add` adds one, or press A in \
                     letterbox; or write them in {}, as\n\n{}",
                    args.config.display(),
                    config::EXAMPLE
                );
                return Ok(());
            }
            println!("{}", args.config.display());
            for a in accounts {
                print!("  {}: {} at {}:{}", a.name, a.user(), a.host, a.port());
                if *check {
                    let name = a.name.clone();
                    match imap::Imap::new(a).folders() {
                        Ok(f) => print!(" — signed in, {} folders", f.len()),
                        Err(e) => print!(" — {name} cannot connect: {}", worker::text(e)),
                    }
                }
                println!();
            }
            Ok(())
        }
        Some(Command::Folders { account, json }) => {
            let mut store = store(args, account.as_deref())?;
            let mut folders = store.folders()?;
            for f in folders.iter_mut().filter(|f| f.selectable) {
                if let Ok((total, unseen)) = store.counts(&f.path) {
                    f.total = Some(total);
                    f.unseen = Some(unseen);
                }
            }
            if *json {
                println!("{}", serde_json::to_string_pretty(&folders)?);
                return Ok(());
            }
            for f in &folders {
                let counts = match (f.total, f.unseen) {
                    (Some(t), Some(u)) if u > 0 => format!("{t}, {u} unread"),
                    (Some(t), _) => t.to_string(),
                    _ => String::new(),
                };
                println!(
                    "{}{} {:<30} {counts:>18}  {}",
                    "  ".repeat(f.depth),
                    f.role.icon(),
                    f.name,
                    mail::controls(&f.path)
                );
            }
            Ok(())
        }
        Some(Command::List { which, limit, json }) => {
            let mut store = store(args, which.account.as_deref())?;
            let page = store.page(&which.folder, 0, *limit)?;
            print_list(&page.summaries, *json)
        }
        Some(Command::Search {
            words,
            which,
            limit,
            json,
        }) => {
            let mut store = store(args, which.account.as_deref())?;
            let found = store.search(&which.folder, &words.join(" "), *limit)?;
            print_list(&found, *json)
        }
        Some(Command::Read {
            uid,
            which,
            json,
            raw,
        }) => {
            let mut store = store(args, which.account.as_deref())?;
            let bytes = store.raw(&which.folder, *uid)?;
            if *raw {
                use std::io::Write;
                std::io::stdout().write_all(&bytes)?;
                return Ok(());
            }
            let m = mail::message(*uid, bytes)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&m)?);
                return Ok(());
            }
            println!("Subject: {}", m.subject);
            println!("From:    {}", m.from);
            if !m.to.is_empty() {
                println!("To:      {}", m.to.join(", "));
            }
            if !m.cc.is_empty() {
                println!("Cc:      {}", m.cc.join(", "));
            }
            if let Some(d) = m.date {
                println!("Date:    {}", ui::long_date(d));
            }
            for a in &m.attachments {
                println!("File:    {} ({}, {} bytes)", a.name, a.mime, a.size);
            }
            println!("\n{}", m.text);
            Ok(())
        }
        Some(Command::Mcp) => mcp::Server::new(&args.config, args.demo)?.run(),
        Some(Command::Filter {
            action: FilterAction::Auto { state },
        }) => {
            let on = matches!(state, OnOff::On);
            set_auto(
                &args.config.with_file_name("filter"),
                on,
                jiff::Timestamp::now().as_second(),
            )?;
            println!(
                "{}",
                if on {
                    "new mail it is 95% sure of is thrown away on its own: told of as letterbox opens, and by B"
                } else {
                    "nothing is thrown away on its own"
                }
            );
            Ok(())
        }
        Some(Command::Filter {
            action: FilterAction::Debrief { days },
        }) => {
            let index = open_index(args)?;
            let since = jiff::Timestamp::now().as_second() - days * 24 * 60 * 60;
            print!("{}", debrief_text(&index, since));
            Ok(())
        }
        Some(Command::Filter { action }) => {
            let mut index = open_index(args)?;
            // Learnt or forgotten while letterbox is open, what either
            // writes would be lost.
            if matches!(action, FilterAction::Learn { .. } | FilterAction::Forget)
                && !args.demo
                && !index.writes()
            {
                bail!(
                    "letterbox is open, and writes what is learnt: close it, then run this again"
                );
            }
            if let FilterAction::Forget = action {
                let n = index.unseed()?;
                index.flush()?;
                println!("{n} read from the Trash and your labels, forgotten\n");
            }
            if let FilterAction::Learn { limit } = action {
                let now = jiff::Timestamp::now().as_second();
                let (thrown, kept, learnt) = learn(&mut index, &mut stores(args)?, *limit, now)?;
                index.flush()?;
                println!(
                    "{thrown} in the Trash and {kept} under your labels read; {learnt} not learnt before, learnt\n"
                );
            }
            let decay = match action {
                FilterAction::Check { decay } => decay.unwrap_or(filter::DECAY),
                _ => filter::DECAY,
            };
            print!("{}", check(&index, decay));
            Ok(())
        }
        Some(Command::Map {
            action: MapAction::Check,
        }) => {
            let index = if args.demo {
                open_index(args)?
            } else {
                if config::load(&args.config)?.iter().any(in_vault) && !vault::is_open() {
                    let _ = vault::unlock_by_key();
                }
                index::Index::read(&index::key::dir(), secret::find)?
                    .context("the map is not begun: open letterbox and press M to begin it")?
            };
            print!("{}", map_check(&index));
            Ok(())
        }
        Some(Command::Map {
            action: MapAction::Rebuild { from_servers },
        }) => {
            let mut index = open_index(args)?;
            if *from_servers {
                index.forget_cache()?;
            }
            if *from_servers || index.records.is_empty() {
                let shared = std::sync::Mutex::new(index);
                let stop = std::sync::atomic::AtomicBool::new(false);
                let now = jiff::Timestamp::now().as_second();
                for (name, store) in stores(args)? {
                    let mut say = |s: String| eprint!("\r{name}: {s}   ");
                    let mut twin = store.twin().unwrap_or(store);
                    index::sync::pass(twin.as_mut(), &shared, now, &stop, &mut say)?;
                    eprintln!();
                }
                index = shared.into_inner().unwrap_or_else(|e| e.into_inner());
            }
            println!("{}", map::rebuild(&mut index)?);
            Ok(())
        }
        Some(Command::Vault {
            action: VaultAction::Passphrase,
        }) => {
            if !vault::exists() {
                bail!("there is no vault yet: `letterbox keep vault` makes one");
            }
            open_vault(false)?;
            let new = ask_password("New vault passphrase, or nothing for a key file")?;
            if new.is_empty() {
                vault::rekey(None)?;
                println!(
                    "the vault's key is kept in {}, which only you may read: nothing is asked",
                    vault::key_path().display()
                );
            } else {
                if ask_password("Again")? != new {
                    bail!("the two passphrases differ: the vault is as it was");
                }
                vault::rekey(Some(&new))?;
                println!("the vault's passphrase is changed: it is asked for as letterbox starts");
            }
            Ok(())
        }
        Some(Command::Keep { to }) => {
            let accounts = config::load(&args.config)?;
            let dir = index::key::dir();
            let map_in_vault =
                index::key::card(&dir)?.is_some_and(|c| c.keeper == secret::Keeper::Vault);
            if *to == secret::Keeper::Keyring && !secret::keepers().contains(to) {
                bail!("there is no system store here to keep them in");
            }
            if *to == secret::Keeper::Vault || accounts.iter().any(in_vault) || map_in_vault {
                open_vault(*to == secret::Keeper::Vault)?;
            }
            let said = setup::move_secrets(
                &args.config,
                &dir,
                *to,
                secret::find,
                secret::store,
                secret::remove,
            )?;
            if said.is_empty() {
                println!("all of them are kept {} already", to.about());
            }
            for line in said {
                println!("{line}");
            }
            Ok(())
        }
        Some(Command::Map {
            action: MapAction::Key { export },
        }) => {
            let dir = index::key::dir();
            let Some(card) = index::key::card(&dir)? else {
                bail!("there is no map key yet: letterbox makes one as it starts");
            };
            if !*export {
                println!(
                    "{}\n  public key {}\n  its secret kept {}",
                    dir.join("map.toml").display(),
                    card.recipient,
                    card.keeper.about()
                );
                return Ok(());
            }
            if card.keeper == secret::Keeper::Vault {
                open_vault(false)?;
            }
            let identity = index::key::identity(&card, secret::find)?;
            print!("{}", index::key::export(&identity));
            Ok(())
        }
        Some(Command::Screen {
            width,
            height,
            svg,
            keys,
        }) => {
            let mut app = app::App::new(stores(args)?, args.config.clone());
            app.theme = theme::Theme::of(args.theme);
            if args.demo {
                app.fix_accounts();
            }
            app.settle(Duration::from_secs(30));
            for key in keys.iter().flat_map(|k| app::keys(k)) {
                // Drawn first, as a terminal would be, so that what a key
                // does knows the screen it was pressed on.
                let _ = app::screen(&mut app, *width, *height);
                app.key(key);
                app.settle(Duration::from_secs(30));
            }
            if *svg {
                print!("{}", app::screen_svg(&mut app, *width, *height));
            } else {
                println!("{}", app::screen(&mut app, *width, *height));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
