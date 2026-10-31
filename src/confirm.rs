//! What was asked of the servers, moving mail out of where it was: filed
//! under a label, archived, or put in the Trash. Each is kept till its
//! server says it is done and a look at the mail after finds it so; what
//! failed, with why, to be said and done again.

use crate::app::Choice;
use crate::index::Key;

/// How many looks at the mail, after its server said it was done, before
/// a message still where it was is taken to have failed.
pub const LOOKS: u8 = 2;

/// Where a message was asked to go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Goal {
    Filed(String),
    Archived,
    Trashed,
    /// To another account's folder, as said: `Grant · Work`.
    Brought(String),
}

impl Goal {
    /// What was asked, as said: `→ Grant`, `archived`, `to the Trash`.
    pub fn said(&self) -> String {
        match self {
            Goal::Filed(label) => format!("→ {label}"),
            Goal::Archived => "archived".to_string(),
            Goal::Trashed => "to the Trash".to_string(),
            Goal::Brought(to) => format!("→ {to}"),
        }
    }
}

/// How far a message has come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Asked of its server, not yet answered.
    Asked,
    /// Its server said it was done; `looks` looks at the mail since.
    Done {
        looks: u8,
    },
    /// The mail, looked at, has it where it was asked to go.
    Confirmed,
    Failed(String),
}

/// A message asked to go somewhere: on which account, from where, what
/// the index knows it by, its subject, where to, and what was chosen for
/// it in the Inbox looked through, if it was.
#[derive(Clone, Debug)]
pub struct Job {
    pub account: usize,
    pub folder: String,
    pub uid: u32,
    pub key: Option<Key>,
    pub subject: String,
    pub goal: Goal,
    pub choice: Option<Choice>,
    pub state: State,
}

/// What was asked together, or one after another while any of it was
/// still waited for; and whether it was said once it settled.
#[derive(Default)]
pub struct Batch {
    pub jobs: Vec<Job>,
    said: bool,
}

/// How many of a batch are where.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub all: usize,
    pub confirmed: usize,
    pub failed: usize,
}

impl Batch {
    /// Keep `job`, asked: in a new batch if the last has settled; in place
    /// of one asked before of the same message.
    pub fn ask(&mut self, job: Job) {
        if !self.busy() {
            self.jobs.clear();
            self.said = false;
        }
        self.jobs
            .retain(|j| (j.account, &j.folder, j.uid) != (job.account, &job.folder, job.uid));
        self.jobs.push(job);
    }

    /// Its server's answer for the message `uid` of `folder`: done, or
    /// failed and why. Whether it was one asked.
    pub fn answered(
        &mut self,
        account: usize,
        folder: &str,
        uid: u32,
        result: Result<(), String>,
    ) -> bool {
        let Some(job) = self.jobs.iter_mut().find(|j| {
            j.account == account && j.folder == folder && j.uid == uid && j.state == State::Asked
        }) else {
            return false;
        };
        job.state = match result {
            Ok(()) => State::Done { looks: 0 },
            Err(why) => State::Failed(why),
        };
        true
    }

    /// A look at `account`'s mail: `gone(job)` says whether the message
    /// has left where it was, if that can be told. Left, it is confirmed;
    /// still there after `LOOKS` looks, failed.
    pub fn looked(&mut self, account: usize, gone: impl Fn(&Job) -> Option<bool>) {
        for job in self.jobs.iter_mut().filter(|j| j.account == account) {
            let State::Done { looks } = job.state else {
                continue;
            };
            let looks = looks + 1;
            job.state = match gone(job) {
                Some(true) => State::Confirmed,
                Some(false) if looks >= LOOKS => State::Failed(
                    "its server said it was done, but it is still where it was".to_string(),
                ),
                // What cannot be told, as many looks after, its server's
                // word for it.
                None if looks >= LOOKS => State::Confirmed,
                _ => State::Done { looks },
            };
        }
    }

    /// What `account`'s server says it did, taken as done: all of it,
    /// where nothing looks at its mail; else, `unseen`, only what it did to
    /// a message no look can see, in a folder the index does not read, as
    /// Drafts.
    pub fn trust(&mut self, account: usize, unseen: bool) {
        for job in self.jobs.iter_mut().filter(|j| j.account == account) {
            if (!unseen || job.key.is_none()) && matches!(job.state, State::Done { .. }) {
                job.state = State::Confirmed;
            }
        }
    }

    /// Whether any of it is still waited for.
    pub fn busy(&self) -> bool {
        self.jobs
            .iter()
            .any(|j| matches!(j.state, State::Asked | State::Done { .. }))
    }

    pub fn counts(&self) -> Counts {
        Counts {
            all: self.jobs.len(),
            confirmed: self
                .jobs
                .iter()
                .filter(|j| j.state == State::Confirmed)
                .count(),
            failed: self
                .jobs
                .iter()
                .filter(|j| matches!(j.state, State::Failed(_)))
                .count(),
        }
    }

    /// How far it has come, in a line.
    pub fn said(&self) -> String {
        let c = self.counts();
        let mut said = if self.busy() {
            format!("applying: {} of {} confirmed", c.confirmed, c.all)
        } else {
            format!("{} of {} confirmed", c.confirmed, c.all)
        };
        if c.failed > 0 {
            said.push_str(&format!(" · {} failed", c.failed));
        }
        said
    }

    /// The jobs that failed.
    pub fn failed(&self) -> impl Iterator<Item = &Job> {
        self.jobs
            .iter()
            .filter(|j| matches!(j.state, State::Failed(_)))
    }

    /// Once all of it has settled, the first time it is asked: how it
    /// went, in a line.
    pub fn settled(&mut self) -> Option<String> {
        if self.jobs.is_empty() || self.busy() || self.said {
            return None;
        }
        self.said = true;
        Some(self.said())
    }
}
