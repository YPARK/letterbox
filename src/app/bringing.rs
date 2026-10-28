//! A message moved to another account's folder: its original read whole,
//! copied there and found there by its Message-ID, and only then put in
//! its own account's Trash, as `d` puts one, to be put back from there.
//! Followed till confirmed, as the rest of what moves mail is; what fails
//! on the way leaves the original where it was.
use super::*;
use crate::confirm::{Goal, Job, State};

/// A message on its way to another account: the account and folder it goes
/// to, its Message-ID to be found by there, whether it was read, and its own
/// account's Trash, where its original goes once it is copied.
pub(super) struct Bringing {
    to: usize,
    folder: String,
    message_id: String,
    seen: bool,
    trash: String,
}

impl App {
    /// The folders of the accounts but `a` a message of it may be moved to,
    /// each as `(account, path, name)`: those mail is filed in, not the
    /// Inbox, Sent, Drafts, Junk, Trash nor All Mail.
    pub(super) fn elsewhere(&self, a: usize) -> Vec<(usize, String, String)> {
        let mut out = Vec::new();
        for b in self.live().filter(|&b| b != a) {
            for f in &self.accounts[b].folders {
                if f.selectable && !f.all && matches!(f.role, Role::Other | Role::Archive) {
                    out.push((b, f.path.clone(), f.name.clone()));
                }
            }
        }
        out
    }

    /// Move the message `dot` to the folder `folder` of the account `to`:
    /// copied there first, as `choice`, if it was chosen in the Inbox
    /// looked through.
    pub(super) fn bring(
        &mut self,
        dot: &map::Dot,
        to: usize,
        folder: &str,
        choice: Option<Choice>,
    ) {
        let Some((a, from, uid)) = self.place_of(dot) else {
            self.status = "that message is on no account here".to_string();
            return;
        };
        let Some(trash) = self.trash_of(a) else {
            self.status = format!(
                "{}'s Trash is not known yet: d on one of its messages asks which it is",
                self.accounts[a].name
            );
            return;
        };
        let name = self.accounts[to]
            .folders
            .iter()
            .find(|f| f.path == folder)
            .map_or_else(|| folder.to_string(), |f| f.name.clone());
        self.batch.ask(Job {
            account: a,
            folder: from.clone(),
            uid,
            key: Some(dot.key.clone()),
            subject: dot.subject.clone(),
            goal: Goal::Brought(format!("{name} · {}", self.accounts[to].name)),
            choice,
            state: State::Asked,
        });
        self.bringing.insert(
            (a, from.clone(), uid),
            Bringing {
                to,
                folder: folder.to_string(),
                message_id: dot.message_id.clone(),
                seen: dot.flags.seen,
                trash,
            },
        );
        self.status = format!("moving it to {name} · {}…", self.accounts[to].name);
        // Brought, as the key asks; its copy and its trashing follow the
        // servers' answers. C-x u does not bring it back, and says so.
        if !self.quiet_moves && !self.replying {
            self.remember(super::undoing::Noted::Elsewhere);
        }
        self.ask(a, Ask::Raw { folder: from, uid });
    }

    /// The original, read whole: copied to where it goes.
    pub(super) fn brought_raw(
        &mut self,
        a: usize,
        folder: String,
        uid: u32,
        raw: Result<Vec<u8>, String>,
    ) {
        let place = (a, folder.clone(), uid);
        let Some(b) = self.bringing.get(&place) else {
            return;
        };
        match raw {
            Ok(raw) => {
                let ask = Ask::Bring {
                    to: b.folder.clone(),
                    raw,
                    seen: b.seen,
                    message_id: b.message_id.clone(),
                    from: place,
                };
                let to = b.to;
                self.ask(to, ask);
            }
            Err(why) => {
                self.bringing.remove(&place);
                self.took(a, &folder, uid, Err(&why));
            }
        }
    }

    /// Copied and found where it goes, or not: its original, then, to its
    /// own Trash, under the move's own following; else left where it was.
    pub(super) fn brought(&mut self, from: (usize, String, u32), result: Result<(), String>) {
        let Some(b) = self.bringing.remove(&from) else {
            return;
        };
        let (a, folder, uid) = from;
        match result {
            Ok(()) => {
                self.quiet = true;
                self.trash_quietly(a, folder, uid, b.message_id, b.trash);
                self.quiet = false;
                self.reload(b.to);
                self.map_pass(b.to);
            }
            Err(why) => self.took(a, &folder, uid, Err(&why)),
        }
    }
}
