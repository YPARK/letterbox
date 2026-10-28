//! What the commands do besides the TUI: accounts added and signed in,
//! the vault opened, the map and the filter checked, mail listed.
use super::*;

/// Ask on the terminal, with `default` taken for nothing typed.
pub(super) fn ask(question: &str, default: &str) -> anyhow::Result<String> {
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
pub(super) fn ask_password(question: &str) -> anyhow::Result<String> {
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

pub(super) fn add_account(args: &Args, action: &AccountAction) -> anyhow::Result<()> {
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
pub(super) fn stores(args: &Args) -> anyhow::Result<Vec<(String, Box<dyn Store>)>> {
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
pub(super) fn map_check(index: &index::Index) -> String {
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
pub(super) fn open_index(args: &Args) -> anyhow::Result<index::Index> {
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
pub(super) fn learn(
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
    let ledger = trash::Ledger::load();
    let mut thrown = Vec::new();
    for (name, store) in stores.iter_mut() {
        let ours = ledger.message_ids(store.address());
        if let Err(e) = index::sync::refresh(store.as_mut(), &shared, &keys, now) {
            eprintln!("{name}: what you filed cannot be read again: {e:#}");
        }
        let known = shared.lock().unwrap_or_else(|e| e.into_inner()).known();
        match trash_seeds(store.as_mut(), &known, limit, now, &ours) {
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
/// dealt with, not unwanted: it is left out; and so is what letterbox
/// itself moved there, by its Message-ID in `ours`, taught as it was
/// trashed alone, or not at all, trashed with others.
pub(super) fn trash_seeds(
    store: &mut dyn Store,
    known: &filter::Known,
    limit: usize,
    now: i64,
    ours: &std::collections::HashSet<String>,
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
            if !r.message_id.is_empty() && ours.contains(&r.message_id) {
                continue;
            }
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
pub(super) fn set_auto(path: &std::path::Path, on: bool, now: i64) -> anyhow::Result<()> {
    let mut a = filter::Auto::read(&std::fs::read_to_string(path).unwrap_or_default(), now);
    if on && !a.on {
        a.since = now;
    }
    a.on = on;
    std::fs::write(path, a.to_string()).with_context(|| format!("writing {}", path.display()))
}

/// What was thrown away on its own after `since`, and why, and what was
/// learnt since, as `letterbox filter debrief` says it.
pub(super) fn debrief_text(index: &index::Index, since: i64) -> String {
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
pub(super) fn check(index: &index::Index, decay: f32) -> String {
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

/// The stores for the TUI: as `stores`, but the vault left locked if its
/// passphrase is not given, its accounts saying so; and passwords printed by
/// a command found before the TUI takes the terminal, as such a command
/// may ask for a passphrase on it, as `pass` does by gpg's pinentry.
pub(super) fn stores_for_tui(args: &Args) -> anyhow::Result<Vec<(String, Box<dyn Store>)>> {
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
pub(super) fn open_vault(create: bool) -> anyhow::Result<()> {
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
pub(super) fn store(args: &Args, name: Option<&str>) -> anyhow::Result<Box<dyn Store>> {
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

pub(super) fn print_list(list: &[mail::Summary], json: bool) -> anyhow::Result<()> {
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

pub(super) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

/// A time as `--since` and `--until` take it: seconds since 1970, of ten
/// digits or more, as any time since 2001 is; an instant with its offset;
/// or a date and time in local time. Digits fewer are refused, as a date
/// typed without dashes would be read as a time in 1970.
pub(super) fn when(text: &str) -> anyhow::Result<i64> {
    let text = text.trim();
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        if text.len() < 10 {
            bail!("not a time: {text}; as 2026-10-27 13:50, or seconds since 1970");
        }
        return Ok(text.parse::<i64>()?);
    }
    if let Ok(t) = text.parse::<jiff::Timestamp>() {
        return Ok(t.as_second());
    }
    let civil: jiff::civil::DateTime = text
        .replacen(' ', "T", 1)
        .parse()
        .with_context(|| format!("not a time: {text}, as 2026-10-27 13:50"))?;
    Ok(civil
        .to_zoned(jiff::tz::TimeZone::system())?
        .timestamp()
        .as_second())
}

/// Put back what letterbox moved to Trash from `since` to `until`, as the
/// ledger says, each to where it was; said first, and asked, unless `yes`.
pub(super) fn restore_trash(
    args: &Args,
    since: i64,
    until: Option<i64>,
    account: Option<&str>,
    origin: &[String],
    dry_run: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let until = until.unwrap_or_else(|| jiff::Timestamp::now().as_second());
    // Held while it runs: letterbox open writes the ledger too.
    let _held = if args.demo {
        None
    } else {
        index::lock(&index::key::dir())?
    };
    let mut ledger = trash::Ledger::load();
    if let Some(fault) = ledger.fault() {
        bail!("{fault}");
    }
    let accounts = if args.demo {
        Vec::new()
    } else {
        config::load(&args.config)?
    };
    let email = match account {
        None => None,
        Some(n) => Some(
            accounts
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(n))
                .map(|a| a.email.clone())
                .ok_or_else(|| anyhow::anyhow!("no account named {n}"))?,
        ),
    };
    let entries: Vec<trash::Entry> = ledger
        .between(since, until, email.as_deref())
        .into_iter()
        .filter(|e| origin.is_empty() || origin.contains(&e.origin))
        .collect();
    if entries.is_empty() {
        println!("nothing letterbox moved to Trash then");
        return Ok(());
    }
    let mut counts: std::collections::BTreeMap<(&str, &str, &str), usize> = Default::default();
    for e in &entries {
        *counts
            .entry((&e.account, &e.origin, &e.folder))
            .or_default() += 1;
    }
    for ((account, origin, folder), n) in &counts {
        println!("{n:>6}  {account}  {folder} → {origin}");
    }
    println!("{:>6}  in all", entries.len());
    if dry_run {
        return Ok(());
    }
    if !yes && !ask("put them back? y or n", "n")?.eq_ignore_ascii_case("y") {
        println!("nothing moved");
        return Ok(());
    }
    let emails: Vec<String> = accounts.iter().map(|a| a.email.clone()).collect();
    let mut failed = 0;
    for (name, mut store) in stores(args)? {
        let email = store.address().to_string();
        let mine: Vec<trash::Entry> = entries
            .iter()
            .filter(|e| e.account == email)
            .cloned()
            .collect();
        if mine.is_empty() {
            continue;
        }
        let done = trash::restore(&mut ledger, &mine, store.as_mut());
        let bad: Vec<&trash::Restored> = done.iter().filter(|r| r.result.is_err()).collect();
        println!(
            "{name}: {} put back, {} not",
            done.len() - bad.len(),
            bad.len()
        );
        for r in &bad {
            let e = &r.entry;
            println!(
                "  {} {} → {}: {}",
                e.folder,
                e.uid,
                e.origin,
                r.result.as_ref().err().map(String::as_str).unwrap_or("")
            );
        }
        failed += bad.len();
    }
    let missing = entries
        .iter()
        .filter(|e| !args.demo && !emails.contains(&e.account))
        .count();
    if missing > 0 {
        println!("{missing} of no account set up here, left as they are");
    }
    if failed > 0 {
        bail!("{failed} not put back, each said above: still in the ledger, to try again");
    }
    Ok(())
}
