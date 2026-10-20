//! letterbox: mail in the terminal. IMAP accounts, read in a TUI laid out
//! as a mail app's, or from the shell, a folder or a message at a time.

mod app;
mod config;
mod demo;
mod imap;
mod mail;
mod secret;
mod setup;
mod store;
mod theme;
mod ui;
mod worker;

use anyhow::bail;
use clap::{Parser, Subcommand};
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
        /// A command that prints the password, instead of keeping it
        #[arg(long, conflicts_with = "keep")]
        password_command: Option<String>,
        /// Where to keep the password: the keychain on macOS, the keyring
        /// elsewhere, unless gpg is asked for
        #[arg(long, value_enum)]
        keep: Option<secret::Keeper>,
    },
    /// Remove an account; its mail stays on its server
    Remove {
        name: String,
        /// Keep the password letterbox kept for it
        #[arg(long)]
        keep_password: bool,
    },
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
        password_command,
        keep,
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
    let (guessed, note) = config::guess(&email);
    let name = match name {
        Some(n) => n.clone(),
        None => ask("Name", &setup::name_for(&email))?,
    };
    let host = match host {
        Some(h) => h.clone(),
        None => ask("IMAP server", &guessed)?,
    };
    let mut account = setup::account(
        &name,
        &email,
        &host,
        *port,
        user.as_deref().unwrap_or(&email),
    );
    if let Some(c) = password_command {
        account.password_command = Some(c.clone());
    } else {
        if let Some(note) = note {
            eprintln!("{note}");
        }
        account.password = Some(ask_password("Password")?);
    }
    let keepers = secret::keepers();
    let keeper = match keep {
        Some(k) if keepers.contains(k) => *k,
        Some(k) => bail!("cannot keep a password in the {} here", k.name()),
        None => *keepers.first().ok_or_else(|| {
            anyhow::anyhow!("there is nowhere here to keep a password: give --password-command")
        })?,
    };
    eprintln!("signing in to {}…", account.host);
    let (_, folders) = setup::add(&args.config, account, keeper, secret::store)?;
    println!(
        "added {name}: {} folders, in {}",
        folders.len(),
        args.config.display()
    );
    Ok(())
}

/// The accounts' stores, by name: the demo's, or those set up.
fn stores(args: &Args) -> anyhow::Result<Vec<(String, Box<dyn Store>)>> {
    stores_of(args, |_| {})
}

/// The stores, each account readied by `ready` first.
fn stores_of(
    args: &Args,
    ready: impl Fn(&mut config::Account),
) -> anyhow::Result<Vec<(String, Box<dyn Store>)>> {
    if args.demo {
        return Ok(vec![("Demo".to_string(), Box::new(demo::Demo::new()))]);
    }
    Ok(config::load(&args.config)?
        .into_iter()
        .map(|mut a| {
            ready(&mut a);
            (
                a.name.clone(),
                Box::new(imap::Imap::new(a)) as Box<dyn Store>,
            )
        })
        .collect())
}

/// Find, before the TUI takes the terminal, the passwords whose command
/// may ask for a passphrase on it, as gpg's pinentry does; what cannot be
/// found is said when the account signs in, not asked for again.
fn unlock(a: &mut config::Account) {
    let Some(command) = &a.password_command else {
        return;
    };
    if !secret::asks(command) {
        return;
    }
    match a.password() {
        Ok(p) => a.password = Some(p),
        Err(e) => a.missing = Some(format!("{e:#}")),
    }
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

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    match &args.command {
        None => {
            let mut app = app::App::new(stores_of(&args, unlock)?, args.config.clone());
            app.theme = theme::Theme::of(args.theme);
            if args.demo {
                app.fix_accounts();
            }
            app::run(&mut app)
        }
        Some(Command::Accounts {
            action: Some(action @ AccountAction::Add { .. }),
            ..
        }) => add_account(&args, action),
        Some(Command::Accounts {
            action:
                Some(AccountAction::Remove {
                    name,
                    keep_password,
                }),
            ..
        }) => {
            let a = setup::remove(&args.config, name, *keep_password)?;
            println!("removed {}; its mail stays on {}", a.name, a.host);
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
                        Err(e) => print!(" — {name} cannot connect: {e:#}"),
                    }
                }
                println!();
            }
            Ok(())
        }
        Some(Command::Folders { account, json }) => {
            let mut store = store(&args, account.as_deref())?;
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
                    f.path
                );
            }
            Ok(())
        }
        Some(Command::List { which, limit, json }) => {
            let mut store = store(&args, which.account.as_deref())?;
            let page = store.page(&which.folder, 0, *limit)?;
            print_list(&page.summaries, *json)
        }
        Some(Command::Search {
            words,
            which,
            limit,
            json,
        }) => {
            let mut store = store(&args, which.account.as_deref())?;
            let found = store.search(&which.folder, &words.join(" "), *limit)?;
            print_list(&found, *json)
        }
        Some(Command::Read {
            uid,
            which,
            json,
            raw,
        }) => {
            let mut store = store(&args, which.account.as_deref())?;
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
        Some(Command::Screen {
            width,
            height,
            svg,
            keys,
        }) => {
            let mut app = app::App::new(stores(&args)?, args.config.clone());
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
