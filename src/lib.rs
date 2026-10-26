//! letterbox: mail in the terminal. IMAP accounts, read in a TUI laid out
//! as a mail app's, or from the shell, a folder or a message at a time. The
//! program is `letterbox`, and `lbx` for short, each of them `run`.

mod app;
mod cli;
mod commands;
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

use self::cli::*;
use self::commands::*;

use anyhow::{Context, bail};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;
use store::Store;

#[derive(Clone, Copy, clap::ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
enum SignIn {
    Password,
    Microsoft,
}

fn in_vault(a: &config::Account) -> bool {
    a.keep == secret::Keeper::Vault && a.password_command.is_none()
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
