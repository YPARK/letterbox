//! `letterbox slack`: add a workspace, say who you are in each, list its
//! conversations, read one, remove it. Nothing here posts anything.

use super::reading::{Message, find, named};
use super::{Kind, Session, Slack, Workspace, remember, user_token, workspaces};
use crate::cli::SlackAction;
use crate::commands::ask_password;
use crate::secret;
use anyhow::{Context, bail};
use std::path::Path;

pub fn run(config: &Path, action: Option<&SlackAction>) -> anyhow::Result<()> {
    match action {
        None => list(config),
        Some(SlackAction::Add) => add(config),
        Some(SlackAction::Channels { workspace, json }) => {
            channels(config, workspace.as_deref(), *json)
        }
        Some(SlackAction::Read {
            conversation,
            thread,
            count,
            workspace,
        }) => read(
            config,
            workspace.as_deref(),
            conversation,
            thread.as_deref(),
            *count,
        ),
        Some(SlackAction::Live { workspace }) => live(config, workspace.as_deref()),
        Some(SlackAction::Watch { workspace }) => watch(config, workspace.as_deref()),
        Some(SlackAction::Remove { workspace }) => remove(config, workspace),
    }
}

fn add(config: &Path) -> anyhow::Result<()> {
    let typed = ask_password("Your app's User OAuth Token (xoxp-…, not shown as you paste it)")?;
    let token = user_token(&typed)?;
    let who = Slack::new(Session {
        domain: String::new(),
        token: token.clone(),
        app: None,
    })
    .whoami()?;
    if who.domain.is_empty() {
        bail!("Slack did not say which workspace this token is for");
    }
    let keep = super::keeper_for_new(crate::vault::exists(), &secret::keepers());
    let session = Session {
        domain: who.domain.clone(),
        token,
        app: None,
    };
    session.keep(keep)?;
    remember(
        config,
        &Workspace {
            domain: who.domain.clone(),
            keep,
        },
    )?;
    println!(
        "Signed in to {} as {}; the token is kept {}.",
        who.team,
        who.user,
        keep.about()
    );
    Ok(())
}

fn list(config: &Path) -> anyhow::Result<()> {
    let all = workspaces(config)?;
    if all.is_empty() {
        println!("No Slack workspace yet: letterbox slack add");
    }
    for workspace in all {
        let workspace = settled(config, workspace);
        match Session::kept(&workspace).and_then(|s| Slack::new(s).whoami()) {
            Ok(who) => println!("{}: {} as {}", workspace.domain, who.team, who.user),
            Err(e) => println!("{}: {e:#}", workspace.domain),
        }
    }
    Ok(())
}

/// `workspace`, its tokens moved out of the keychain into the vault, if they
/// are to be, and said so.
fn settled(config: &Path, workspace: Workspace) -> Workspace {
    match super::settle(config, &workspace) {
        Ok((now, true)) => {
            eprintln!(
                "{}: Slack's tokens moved out of the macOS keychain into letterbox's vault",
                now.domain
            );
            now
        }
        Ok((now, false)) => now,
        Err(e) => {
            eprintln!("{}: left in the keychain: {e:#}", workspace.domain);
            workspace
        }
    }
}

fn choose(config: &Path, named: Option<&str>) -> anyhow::Result<Workspace> {
    choose_kept(config, named).map(|w| settled(config, w))
}

fn choose_kept(config: &Path, named: Option<&str>) -> anyhow::Result<Workspace> {
    let mut all = workspaces(config)?;
    match named {
        Some(name) => all
            .into_iter()
            .find(|w| w.domain == name)
            .with_context(|| format!("no Slack workspace {name}: see letterbox slack")),
        None if all.len() == 1 => Ok(all.remove(0)),
        None if all.is_empty() => bail!("no Slack workspace yet: letterbox slack add"),
        None => bail!("which workspace? --workspace, as letterbox slack lists them"),
    }
}

fn channels(config: &Path, asked: Option<&str>, json: bool) -> anyhow::Result<()> {
    let workspace = choose(config, asked)?;
    let slack = Slack::new(Session::kept(&workspace)?);
    let channels = named(slack.channels()?, &slack.people()?);
    if json {
        println!("{}", serde_json::to_string_pretty(&channels)?);
        return Ok(());
    }
    for channel in channels {
        let mark = match channel.kind {
            Kind::Public => "#",
            Kind::Private => "!",
            Kind::Direct => "@",
            Kind::Group => "&",
        };
        println!("{mark}{}\t{}", channel.name, channel.id);
    }
    Ok(())
}

fn read(
    config: &Path,
    named_workspace: Option<&str>,
    conversation: &str,
    thread: Option<&str>,
    count: usize,
) -> anyhow::Result<()> {
    let workspace = choose(config, named_workspace)?;
    let slack = Slack::new(Session::kept(&workspace)?);
    let people = slack.people()?;
    let channels = named(slack.channels()?, &people);
    let channel = find(&channels, conversation)?;
    let messages = match thread {
        Some(ts) => slack.thread(&channel.id, ts, &people)?,
        None => slack.messages(&channel.id, count, &people)?,
    };
    for message in messages {
        print_message(&message);
    }
    Ok(())
}

/// A message's time, here, from Slack's `1730000000.000100`.
fn when(ts: &str) -> String {
    ts.split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .and_then(|s| jiff::Timestamp::from_second(s).ok())
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn print_message(message: &Message) {
    println!("{} {}: {}", when(&message.ts), message.who, message.text);
    if message.replies > 0 {
        println!("  {} replies: --thread {}", message.replies, message.ts);
    }
}

fn live(config: &Path, asked: Option<&str>) -> anyhow::Result<()> {
    let workspace = choose(config, asked)?;
    let mut session = Session::kept(&workspace)?;
    let typed = ask_password(
        "Your app's app-level token (xapp-…, under Basic Information; not shown as you paste it)",
    )?;
    let token = typed.trim();
    if !token.starts_with("xapp-") {
        bail!(
            "that is not an app-level token: it starts xapp-, under your app's Basic Information"
        );
    }
    super::live::Socket::new(token).link()?;
    session.app = Some(token.to_string());
    session.keep(workspace.keep)?;
    println!(
        "{} will be heard as things happen, in letterbox from when it next starts. \
         letterbox slack watch shows what comes, but not what is said.",
        workspace.domain
    );
    Ok(())
}

fn watch(config: &Path, asked: Option<&str>) -> anyhow::Result<()> {
    use super::live::{Event, Socket};
    let workspace = choose(config, asked)?;
    let session = Session::kept(&workspace)?;
    let Some(app) = session.app.clone() else {
        bail!("no app-level token yet: letterbox slack live");
    };
    let slack = Slack::new(session);
    let me = slack.whoami()?.user_id;
    let names: std::collections::HashMap<String, String> = named(slack.mine()?, &slack.people()?)
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let now = || jiff::Zoned::now().strftime("%H:%M:%S").to_string();
    println!("listening to {}; C-c ends it", workspace.domain);
    Socket::new(&app).listen(&mut |event| {
        match event {
            Event::Connected => println!("{} connected", now()),
            Event::Lost(why) => println!("{} lost, trying again: {why}", now()),
            Event::Message(m) => {
                let name = names
                    .get(&m.channel)
                    .map_or(m.channel.as_str(), String::as_str);
                let what = match (m.new, &m.thread) {
                    (true, None) => "a message",
                    (true, Some(_)) => "a reply in a thread",
                    (false, _) => "a message changed or gone",
                };
                let whose = if m.user == me { ", yours" } else { "" };
                println!("{} {name}: {what}{whose}", now());
            }
        }
        true
    });
    Ok(())
}

fn remove(config: &Path, name: &str) -> anyhow::Result<()> {
    let workspace = choose(config, Some(name))?;
    Session::forget(&workspace)?;
    super::forget_workspace(config, name)?;
    println!(
        "Forgot {name} here. Slack still holds the token good: revoke it in your app's \
         OAuth & Permissions."
    );
    Ok(())
}
