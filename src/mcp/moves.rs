//! Mail moved by an agent, once the user said yes: to Trash, or filed, from
//! one folder or from several, of one account or of several.

use super::*;

/// Some messages of one folder, to be moved to one place, as the user is to
/// be asked of them.
pub(super) struct Plan {
    /// The account, as it is called: to open, and to say in the question.
    account: Option<String>,
    folder: String,
    thrown: bool,
    moving: Vec<Summary>,
    to: String,
    /// Whether `to` is a folder to be made for them.
    made: bool,
    of_drafts: bool,
}

impl Plan {
    /// What the user is asked of it: where each goes, and what each is.
    pub(super) fn question(&self) -> String {
        // Named as the user reads them, and of which account.
        let (folder, to) = (mail::utf7_imap(&self.folder), mail::utf7_imap(&self.to));
        let n = self.moving.len();
        let ask = if self.thrown {
            let them = if n == 1 { "it" } else { "them" };
            format!("Move {n} from {folder} to {to}? letterbox can put {them} back.")
        } else if self.made {
            format!("File {n} from {folder} under {to}, a new folder made for them?")
        } else {
            format!("File {n} from {folder} under {to}?")
        };
        let what = match &self.account {
            Some(a) => format!("In {a}: {ask}"),
            None => ask,
        };
        listing(&what, &self.moving, self.of_drafts)
    }
}

/// A move as an agent asks for it: `uids` of `folder`, to Trash if
/// `thrown`, else filed under `to`, a new folder of that name made if
/// `create` and there is none.
pub(super) struct Wanted<'a> {
    pub(super) folder: &'a str,
    pub(super) uids: &'a [u32],
    pub(super) thrown: bool,
    pub(super) to: Option<&'a str>,
    pub(super) create: bool,
}

impl Server {
    /// The plan for the move `wanted` of the account named; or why it cannot
    /// be done. `folders` are the account's, as listed once for every move
    /// of it.
    pub(super) fn plan(
        &mut self,
        account: Option<&str>,
        folders: &[Folder],
        wanted: Wanted,
    ) -> anyhow::Result<Plan> {
        let Wanted {
            folder,
            uids,
            thrown,
            to,
            create,
        } = wanted;
        let moving = self.moving(account, folder, uids)?;
        let action = if thrown {
            crate::report::Action::Trash
        } else {
            crate::report::Action::File
        };
        let of_drafts = mail::role_is(folders, folder, mail::Role::Drafts);
        let (to, made) = match self.destination(account, folders, folder, action, to) {
            Ok(to) => (to, false),
            // Only where there is none of that name: never a second.
            // Why a new one cannot be made, as a folder of the name there
            // already, is what to say.
            Err(_) if create && !thrown => (new_folder(folders, to)?, true),
            Err(e) => return Err(e),
        };
        Ok(Plan {
            account: self.called(account),
            folder: folder.to_string(),
            thrown,
            moving,
            to,
            made,
            of_drafts,
        })
    }

    /// Move what `plan` says, the user having said yes to it, making the
    /// folder it files under first if it is to be made and is not yet: what
    /// was moved, what failed, and what was learnt of it.
    pub(super) fn carry(&mut self, plan: &Plan) -> anyhow::Result<Value> {
        let Plan {
            account,
            folder,
            thrown,
            moving,
            to,
            made,
            of_drafts,
        } = plan;
        let account = account.as_deref();
        // Made here only if no move before made it, whatever its case; filed
        // under it as it is spelled once there.
        let (to, created) = if *made {
            let store = self.store(account)?;
            let folders = store.folders()?;
            match mail::folder_named(&folders, &mail::utf7_imap(to)) {
                Some(f) => (f.path.clone(), false),
                None => {
                    store.create(to)?;
                    (to.clone(), true)
                }
            }
        } else {
            (to.clone(), false)
        };
        let to = &to;
        let mut moved = Vec::new();
        let mut failed = Vec::new();
        let mut trashed = Vec::new();
        let at = jiff::Timestamp::now().as_second();
        let store = self.store(account)?;
        let address = store.address().to_string();
        for s in moving {
            let done = if *thrown {
                store.relocate(folder, s.uid, to, &s.message_id).map(|uid| {
                    trashed.push(crate::trash::Entry {
                        account: address.clone(),
                        message_id: s.message_id.clone(),
                        folder: to.clone(),
                        uid,
                        origin: folder.clone(),
                        at,
                    })
                })
            } else {
                store
                    .file_from(folder, s.uid, to, &s.message_id, folder)
                    .map(|_| ())
            };
            match done {
                Ok(()) => moved.push(s.uid),
                Err(e) => failed.push(json!({ "uid": s.uid, "error": said(&e) })),
            }
        }
        // Moved, whether or not it is recorded to be put back: said apart.
        let unrecorded = self.record_trashed(trashed);
        let gone: Vec<&Summary> = moving.iter().filter(|s| moved.contains(&s.uid)).collect();
        // Nothing done with a draft is learnt, though All Mail holds it too,
        // marked so or not.
        let learnt = if *of_drafts {
            json!(0)
        } else {
            self.advise(account, folder, &gone, *thrown)
        };
        Ok(json!({
            "account": account,
            "folder": folder,
            "moved": moved.len(),
            "not_recorded_to_put_back": unrecorded,
            "made": created.then_some(to),
            "to": to,
            "uids": moved,
            "failed": failed,
            "learnt": learnt,
        }))
    }

    /// The moves `args` give, of any folders of any accounts, each planned
    /// before the user is asked once of all of them; carried out only if
    /// they say yes.
    pub(super) fn move_all(&mut self, args: &Value) -> anyhow::Result<Value> {
        let moves = args
            .get("moves")
            .and_then(Value::as_array)
            .filter(|m| !m.is_empty())
            .context("give the moves, as a list of at least one")?;
        let create = flag(args, "create");
        let mut plans = Vec::new();
        let mut count = 0;
        let mut seen = std::collections::HashSet::new();
        // Each account's folders, listed once for all its moves.
        let mut listed: std::collections::HashMap<Option<String>, Vec<Folder>> =
            std::collections::HashMap::new();
        for m in moves {
            let text = |k: &str| m.get(k).and_then(Value::as_str);
            let thrown = match text("action") {
                Some("trash") => true,
                Some("file") => false,
                _ => bail!("each move's action is trash or file"),
            };
            let uids = uids(m)?;
            let folder = text("folder").unwrap_or("INBOX");
            let account = self.called(text("account"));
            for &uid in &uids {
                if !seen.insert((account.clone(), folder.to_string(), uid)) {
                    bail!("message {uid} of {folder} is in two moves: give each once");
                }
            }
            count += uids.len();
            if count > MOVED_LIMIT {
                bail!("give at most {MOVED_LIMIT} messages in all");
            }
            let folders = match listed.remove(&account) {
                Some(f) => f,
                None => self.store(text("account"))?.folders()?,
            };
            let wanted = Wanted {
                folder,
                uids: &uids,
                thrown,
                to: text("to"),
                create,
            };
            plans.push(self.plan(text("account"), &folders, wanted)?);
            listed.insert(account, folders);
        }
        let question = plans
            .iter()
            .map(Plan::question)
            .collect::<Vec<_>>()
            .join("\n\n");
        let answer = self.confirm(&question)?;
        if let Some(why) = answer.refused("moved") {
            return Ok(json!({ "moved": 0, "said": why }));
        }
        let done: Vec<Value> = plans
            .iter()
            .map(|p| {
                self.carry(p)
                    .unwrap_or_else(|e| json!({ "moved": 0, "error": said(&e) }))
            })
            .collect();
        let moved: u64 = done.iter().filter_map(|d| d["moved"].as_u64()).sum();
        // What was made, as it was: never what a move only meant to make.
        let made: Vec<&Value> = done
            .iter()
            .map(|d| &d["made"])
            .filter(|m| m.is_string())
            .collect();
        Ok(json!({ "moved": moved, "made": made, "moves": done }))
    }

    /// Every message of the folder `from` moved into `into`, both folders
    /// mail is filed in, and `from` removed once nothing is left in it: the
    /// user asked once, nothing done unless they say yes. Nothing is learnt
    /// of it: it is the folders that change, not what the mail is.
    pub(super) fn merge(
        &mut self,
        account: Option<&str>,
        from: Option<String>,
        into: Option<String>,
    ) -> anyhow::Result<Value> {
        let (from, into) = (
            from.context("give the folder to merge, as from")?,
            into.context("give the folder to merge it into, as into")?,
        );
        if from == into {
            bail!("{from} is the one folder: give two");
        }
        let folders = self.store(account)?.folders()?;
        // Asked of only what can be done whole: a folder with one under it
        // would be left, so it is not merged away.
        if let Some(under) = folders.iter().find(|f| f.is_within(&from)) {
            bail!("{from} has {} under it: merge that first", under.path);
        }
        for (path, emptied) in [(&from, true), (&into, false)] {
            let f = folders
                .iter()
                .find(|f| &f.path == path && f.selectable)
                .with_context(|| format!("there is no folder {path}: list_accounts lists them"))?;
            // Archive is kept for its role: merged into, never emptied away.
            let archived = !emptied && f.role == mail::Role::Archive && !f.all;
            if !f.is_filed() && !archived {
                bail!("{path} is not a folder mail is filed in and can be merged away");
            }
        }
        let store = self.store(account)?;
        let (n, _) = store.counts(&from)?;
        let answer = self.confirm(&format!(
            "Move all {n} messages of {from} into {into}, then remove {from}, empty? \
             No message is deleted."
        ))?;
        if let Some(why) = answer.refused("moved") {
            return Ok(json!({ "moved": 0, "said": why }));
        }
        let store = self.store(account)?;
        // All at once, a message without a Message-ID too.
        let moved = store.move_every(&from, &into)?;
        // Removed only once empty, and only where nothing is under it: the
        // store refuses to remove one that is not, and says why.
        let removed = store.delete_folder(&from);
        Ok(json!({
            "account": self.called(account),
            "from": from,
            "into": into,
            "moved": moved,
            "removed": removed.is_ok(),
            "not_removed": removed.err().map(|e| said(&e)),
        }))
    }
}
