//! Your own AI agent, started as letterbox opens, and what it reports:
//! what came, what to ask it next, and what it would throw away or file,
//! each done only as you choose it here, and learnt as your own decision.
use super::*;
use crate::report::{Action, Report};

/// How often what an agent reported is looked for.
const REPORTS_EVERY: Duration = Duration::from_secs(3);
/// How long a report is shown before a key acts on it: it comes unasked,
/// and a key pressed as it came was meant for what was there before.
pub const GRACE: Duration = Duration::from_millis(800);

/// What connects Claude Code to letterbox, and has T start it: each short
/// enough to paste whole, and asked for the path as a Dock's app has no PATH.
pub const CONNECT_AGENT: [&str; 2] = [
    "claude mcp add --scope user letterbox -- \"$(command -v lbx)\" mcp",
    "lbx agent set",
];

/// The agent started, while it runs.
pub struct AgentRun {
    pub child: std::process::Child,
    /// Whether a report came while it ran.
    pub reported: bool,
    /// When it started: said to have run from then, once it has.
    pub started: i64,
}

/// Your agent, as set: the command, where what it says is written, and
/// where when it last ran is kept.
pub struct AgentSet {
    pub command: String,
    pub log: PathBuf,
    pub ran: PathBuf,
    /// When it last ran, in seconds since 1970; 0 if never.
    pub last_ran: i64,
    /// Whether it is to start on its own as letterbox opens: until each
    /// account's mail is first taken in, once, if new mail came since it
    /// last ran.
    pub due: bool,
    /// The accounts whose mail has been taken in since letterbox opened.
    pub mapped: std::collections::HashSet<usize>,
}

impl AgentSet {
    /// Say it last ran at `when`.
    fn ran_at(&mut self, when: i64) {
        self.last_ran = when;
        let _ = std::fs::write(&self.ran, when.to_string());
    }

    /// Your agent, as `command` starts it, its files beside `config`; to
    /// start on its own as letterbox opens, if new mail has come.
    pub fn beside(command: String, config: &std::path::Path) -> Self {
        let ran = config.with_file_name("agent-ran");
        let last_ran = std::fs::read_to_string(&ran)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0);
        AgentSet {
            command,
            log: config.with_file_name("agent.log"),
            ran,
            last_ran,
            due: true,
            mapped: Default::default(),
        }
    }
}

/// A report shown: the row picked, its proposals first, then its
/// questions.
#[derive(Clone, Debug)]
pub struct ReportView {
    pub report: Report,
    pub at: usize,
    /// When it was shown.
    pub shown: Instant,
}

impl ReportView {
    fn rows(&self) -> usize {
        self.report.proposals.len() + self.report.questions.len()
    }

    /// The question picked, if a question is.
    pub fn question(&self) -> Option<&str> {
        self.at
            .checked_sub(self.report.proposals.len())
            .and_then(|q| self.report.questions.get(q))
            .map(String::as_str)
    }
}

impl App {
    /// T: your agent as it is set now, which may have changed since letterbox
    /// opened, started; or, with none set, what to paste to set one.
    pub(super) fn agent_asked(&mut self) {
        if self.agent.is_some() {
            self.status = "your agent is still looking through new mail".to_string();
            return;
        }
        // The demo has none of the user's, as it has no mail of theirs.
        let command = if self.demo {
            Ok(None)
        } else {
            config::agent(&self.config)
        };
        match command {
            Ok(Some(command)) => match &mut self.agent_set {
                Some(set) => set.command = command,
                None => self.agent_set = Some(AgentSet::beside(command, &self.config)),
            },
            Ok(None) => self.agent_set = None,
            Err(why) => {
                self.status = format!("{why:#}");
                return;
            }
        }
        self.run_agent();
    }

    /// Start your agent, if one is set and it is not running already,
    /// told by `LETTERBOX_AGENT` that letterbox started it, what it says
    /// written to its log, only you able to read it.
    pub(super) fn run_agent(&mut self) {
        if self.agent.is_some() {
            self.status = "your agent is still looking through new mail".to_string();
            return;
        }
        let Some(set) = &mut self.agent_set else {
            self.dialog = Dialog::NoAgent;
            return;
        };
        set.due = false;
        let started = (|| -> anyhow::Result<std::process::Child> {
            let mut open = std::fs::OpenOptions::new();
            open.create(true).write(true).truncate(true);
            std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
            let out = open.open(&set.log)?;
            let err = out.try_clone()?;
            Ok(config::shell(&set.command)
                .env("LETTERBOX_AGENT", "1")
                .stdin(std::process::Stdio::null())
                .stdout(out)
                .stderr(err)
                .spawn()?)
        })();
        match started {
            Ok(child) => {
                self.agent = Some(AgentRun {
                    child,
                    reported: false,
                    started: jiff::Timestamp::now().as_second(),
                });
                self.status = "your agent is looking through new mail".to_string();
            }
            Err(e) => {
                self.status = format!("cannot start your agent: {}", first_line(&format!("{e:#}")))
            }
        }
    }

    /// The mail of `account` taken in, as letterbox opens: your agent
    /// started, once, if new mail came since it last ran; not once every
    /// account's has been, unasked, mid-work.
    pub fn agent_mapped(&mut self, account: usize) {
        let live = self.live().count();
        let Some(set) = &mut self.agent_set else {
            return;
        };
        if !set.due {
            return;
        }
        set.mapped.insert(account);
        let new = self
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .new_unread(set.last_ran + 1)
            .next()
            .is_some();
        if set.mapped.len() >= live {
            set.due = false;
        }
        if new {
            self.run_agent();
        }
    }

    /// Take in what an agent wrote down, what you confirmed to be learnt and
    /// what it reported, and say how the one started finished, once it has
    /// been a while; whether anything changed.
    pub(super) fn take_reports(&mut self) -> bool {
        if self.reports_looked.elapsed() < REPORTS_EVERY {
            return false;
        }
        self.reports_looked = Instant::now();
        let held = self.reports.len();
        let written = {
            let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            index.take_advice().and_then(|()| {
                index.take_written(crate::report::JOURNAL, |mut got: Vec<Report>| {
                    self.reports.append(&mut got);
                    Ok(())
                })
            })
        };
        let mut any = false;
        if let Err(e) = written {
            self.status = format!(
                "cannot read what your agent wrote down: {}",
                first_line(&format!("{e:#}"))
            );
            any = true;
        }
        if let Some(run) = &mut self.agent {
            run.reported |= self.reports.len() > held;
            if let Ok(Some(done)) = run.child.try_wait() {
                // One that failed, or that you quit before it ended, has
                // not looked through the mail: the next one does.
                if (done.success() || run.reported)
                    && let Some(set) = &mut self.agent_set
                {
                    set.ran_at(run.started);
                }
                if !run.reported {
                    let log = self
                        .agent_set
                        .as_ref()
                        .map(|s| tilde(&s.log))
                        .unwrap_or_default();
                    self.status = format!(
                        "your agent finished{} with no report · {log} says what it did",
                        if done.success() { "" } else { ", failing," },
                    );
                }
                self.agent = None;
                any = true;
            }
        }
        self.show_reports() || any
    }

    /// Put back, to be shown as letterbox next opens, what was reported and
    /// not dealt with as you quit.
    pub fn keep_reports(&mut self) -> anyhow::Result<()> {
        let mut left = std::mem::take(&mut self.reports);
        if let Dialog::Report(view) = std::mem::replace(&mut self.dialog, Dialog::Closed) {
            left.insert(0, view.report);
        }
        self.index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put_back_written(crate::report::JOURNAL, left)
    }

    /// Whether a report may come up: not over what is being asked or typed.
    fn can_show_report(&self) -> bool {
        matches!(self.dialog, Dialog::Closed)
            && self.compose.is_none()
            && !self.searching
            && self.menu.is_none()
    }

    /// Show the first report held, once nothing else is asked over the
    /// screen; whether one was.
    pub fn show_reports(&mut self) -> bool {
        if self.reports.is_empty() || !self.can_show_report() {
            return false;
        }
        let report = self.reports.remove(0);
        self.dialog = Dialog::Report(Box::new(ReportView {
            report,
            at: 0,
            shown: Instant::now(),
        }));
        true
    }

    /// A key in a report: ↑ ↓ go along it; space chooses the proposal
    /// picked, or lets it go; a chooses them all; ⏎ does those chosen; c
    /// copies the question picked, as ⏎ does with none chosen; esc or C-g
    /// leaves it, nothing done; any other key, taken by mistake, nothing.
    pub(super) fn report_key(&mut self, key: KeyEvent) {
        let Dialog::Report(view) = &mut self.dialog else {
            return;
        };
        if view.shown.elapsed() < GRACE {
            return;
        }
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        let last = view.rows().saturating_sub(1);
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => view.at = (view.at + 1).min(last),
            (Some(Act::Up), _) => view.at = view.at.saturating_sub(1),
            (_, KeyCode::Char(' ')) if plain => {
                if let Some(p) = view.report.proposals.get_mut(view.at) {
                    p.chosen = !p.chosen;
                }
                view.at = (view.at + 1).min(last);
            }
            (_, KeyCode::Char('a')) if plain => {
                let all = view.report.proposals.iter().all(|p| p.chosen);
                for p in &mut view.report.proposals {
                    p.chosen = !all;
                }
            }
            (_, KeyCode::Enter | KeyCode::Char('c'))
                if key.code == KeyCode::Char('c') && plain
                    || !view.report.proposals.iter().any(|p| p.chosen) =>
            {
                match view.question().map(str::to_string) {
                    Some(q) => {
                        self.launch = Some(Launch::Copy(q));
                        self.status = "copied the question: paste it to your agent".to_string();
                    }
                    None => {
                        self.status = "nothing chosen: space chooses the one picked".to_string()
                    }
                }
            }
            (_, KeyCode::Enter) => {
                let chosen: Vec<_> = view
                    .report
                    .proposals
                    .iter()
                    .filter(|p| p.chosen)
                    .cloned()
                    .collect();
                self.dialog = Dialog::Closed;
                self.do_proposed(chosen);
            }
            (_, KeyCode::Esc) => {
                self.dialog = Dialog::Closed;
                self.status = "left as they are: nothing done".to_string();
            }
            _ => {}
        }
    }

    /// Do what was chosen of what an agent proposed, each as you would
    /// have done it yourself, and learnt so: thrown away, or filed and
    /// kept. Each that cannot be is said.
    fn do_proposed(&mut self, chosen: Vec<crate::report::Proposal>) {
        let mut done = 0;
        let mut not: Vec<String> = Vec::new();
        let (mut trashed, mut filed) = (Vec::new(), Vec::new());
        for p in chosen {
            match self.live().find(|&a| self.accounts[a].ident == p.account) {
                None => not.push(format!("{}: its account is not open", p.subject)),
                Some(a) if p.action == Action::Trash => trashed.push((a, p)),
                Some(a) => filed.push((a, p)),
            }
        }
        for (a, p) in trashed {
            let Some(trash) = self.trash_of(a) else {
                not.push(format!(
                    "{}: no Trash known · d on it asks which",
                    p.subject
                ));
                continue;
            };
            if p.folder == trash {
                not.push(format!("{}: in the Trash already", p.subject));
                continue;
            }
            self.move_to_trash(a, p.folder, p.uid, p.message_id, trash);
            done += 1;
        }
        // Each label's together, from each folder, the index asked of once.
        let mut filing: std::collections::BTreeMap<
            (usize, String, String),
            Vec<crate::index::Key>,
        > = Default::default();
        {
            for (a, p) in filed {
                match self.key_of(a, &p.folder, p.uid, &p.message_id) {
                    Some(key) => filing.entry((a, p.folder, p.to)).or_default().push(key),
                    None => not.push(format!("{}: not on the map yet", p.subject)),
                }
            }
        }
        for ((a, folder, label), keys) in filing {
            let asked = keys.len();
            let filed = self.file_under(keys, &label, true, Some((a, folder)));
            done += filed;
            if filed < asked {
                not.push(format!(
                    "{} for {label}: not on the map yet, or not filed",
                    asked - filed
                ));
            }
        }
        self.status = match not.first() {
            None => format!("{done} done, as you chose, and learnt so"),
            Some(first) => format!(
                "{done} done · {} not: {first}{}",
                not.len(),
                if not.len() > 1 { ", …" } else { "" }
            ),
        };
    }
}
