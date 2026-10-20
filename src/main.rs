//! letterbox: mail in the terminal. IMAP accounts, read in a TUI laid out
//! as a mail app's, or from the shell, a folder or a message at a time.

mod app;
mod config;
mod demo;
mod imap;
mod mail;
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
    /// The accounts set up, and where; with --check, sign in to each
    Accounts {
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

/// The accounts' stores, by name: the demo's, or those set up.
fn stores(args: &Args) -> anyhow::Result<Vec<(String, Box<dyn Store>)>> {
    if args.demo {
        return Ok(vec![("Demo".to_string(), Box::new(demo::Demo::new()))]);
    }
    Ok(config::load(&args.config)?
        .into_iter()
        .map(|a| {
            (
                a.name.clone(),
                Box::new(imap::Imap::new(a)) as Box<dyn Store>,
            )
        })
        .collect())
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
            let mut app = app::App::new(stores(&args)?, args.config.clone());
            app.theme = theme::Theme::of(args.theme);
            app::run(&mut app)
        }
        Some(Command::Accounts { check }) => {
            let accounts = config::load(&args.config)?;
            if accounts.is_empty() {
                println!(
                    "no accounts yet: add them to {}, as\n\n{}",
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
