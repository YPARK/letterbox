//! Deleting: to the Trash, which folder that is asked where the server
//! names none, put back, and removed for good once kept long enough.
use super::*;

/// The existing folders letterbox is asking which of is Trash, the one
/// highlighted, for an account whose server marks none.
pub struct TrashPick {
    pub account: usize,
    pub folders: Vec<(String, String)>,
    pub at: usize,
}

/// A delete waiting until the folder that is Trash is known.
pub(super) struct Held {
    pub(super) account: usize,
    pub(super) from: String,
    pub(super) uid: u32,
    pub(super) message_id: String,
}

/// How a folder is named when asking which one is Trash: its own name, and
/// its path when that says where it sits.
pub(super) fn trash_label(folder: &Folder) -> String {
    if folder.path.eq_ignore_ascii_case(&folder.name) || folder.role == Role::Inbox {
        folder.name.clone()
    } else {
        format!("{} · {}", folder.name, folder.path)
    }
}

impl App {
    /// Archive the message picked: out of the Inbox, to Archive.
    pub(super) fn archive(&mut self) {
        let Some(Listed {
            account,
            folder,
            summary,
        }) = self.picked()
        else {
            self.status = "no message to archive".to_string();
            return;
        };
        let archived = self.accounts[account]
            .folders
            .iter()
            .any(|f| f.path == folder && f.role == Role::Archive);
        if archived {
            self.status = "in Archive already".to_string();
            return;
        }
        // Archived, it is kept: learnt as not thrown away.
        self.teach_at(account, &folder, summary.uid, false);
        self.drop_listed(account, &folder, summary.uid);
        self.status = "archived, out of the Inbox".to_string();
        self.ask(
            account,
            Ask::Archive {
                folder,
                uid: summary.uid,
                message_id: summary.message_id,
            },
        );
    }

    /// Move what is picked to Trash, or, in Trash, back where it was.
    pub(super) fn delete(&mut self) {
        // The message picked: on the map, the map's, the one read; never
        // the list's behind it.
        let Some(Listed {
            account,
            folder,
            summary,
        }) = self.picked()
        else {
            return;
        };
        let uid = summary.uid;
        if self.is_trash(account, &folder) {
            let email = self.accounts[account].email.clone();
            let known = self
                .ledger
                .origin(&email, &folder, &summary.message_id, uid);
            let dest = known.clone().unwrap_or_else(|| self.inbox(account));
            let name = self.folder_label(account, &dest);
            // Put back, it was not to be thrown away: learnt as kept.
            let ident = self.accounts[account].ident.clone();
            let _ = self
                .index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .put_back(&ident, &summary.message_id);
            self.drop_listed(account, &folder, uid);
            self.status = if known.is_some() {
                format!("put back in {name}")
            } else {
                format!("put back in {name}; letterbox had not moved it")
            };
            self.ask(
                account,
                Ask::Move {
                    from: folder,
                    uid,
                    to: dest,
                    message_id: summary.message_id,
                    trashed: false,
                },
            );
            return;
        }
        let Some(trash) = self.trash_of(account) else {
            self.ask_which_trash(account, folder, summary);
            return;
        };
        self.move_to_trash(account, folder, summary.uid, summary.message_id, trash);
    }

    /// List the account's other folders and ask which one `d` should move
    /// mail to. A server that marks a Trash folder is not asked.
    pub(super) fn ask_which_trash(&mut self, account: usize, folder: String, summary: Summary) {
        let folders: Vec<(String, String)> = self
            .accounts
            .get(account)
            .map(|a| {
                // Never where mail is kept or written: ⏎ on the first
                // must not throw it into one of them.
                a.folders
                    .iter()
                    .filter(|f| f.selectable && f.path != folder)
                    .filter(|f| !matches!(f.role, Role::Inbox | Role::Sent | Role::Drafts))
                    .map(|f| (f.path.clone(), trash_label(f)))
                    .collect()
            })
            .unwrap_or_default();
        if folders.is_empty() {
            self.status = "there is no other folder to use as Trash".to_string();
            return;
        }
        self.held = Some(Held {
            account,
            from: folder,
            uid: summary.uid,
            message_id: summary.message_id,
        });
        self.dialog = Dialog::Trash(TrashPick {
            account,
            folders,
            at: 0,
        });
        self.status = "which folder should be Trash?".to_string();
    }

    pub(super) fn cancel_trash(&mut self) {
        self.dialog = Dialog::Closed;
        self.held = None;
        self.status = "not moved".to_string();
    }

    /// The folder highlighted is Trash from now on, and the delete that
    /// was waiting is moved there.
    pub(super) fn choose_trash(&mut self) {
        let Dialog::Trash(pick) = std::mem::take(&mut self.dialog) else {
            return;
        };
        let Some((path, _)) = pick.folders.get(pick.at).cloned() else {
            self.held = None;
            self.status = "not moved".to_string();
            return;
        };
        if let Some(folder) = self.folder_mut(pick.account, &path) {
            folder.role = Role::Trash;
        }
        self.accounts[pick.account].trash = Some(path.clone());
        let remembered = if self.editable {
            let name = self.accounts[pick.account].name.clone();
            config::set_trash(&self.config, &name, &path)
        } else {
            Ok(())
        };
        let moved = self.resume_held(pick.account);
        if let Err(e) = remembered
            && moved
        {
            self.status = format!(
                "{} · not remembered: {}",
                self.status,
                first_line(&format!("{e:#}"))
            );
        }
    }

    /// Learn from what was done with the message of `account` at `uid` in
    /// `folder`: thrown away, or kept.
    pub(super) fn teach_at(&mut self, account: usize, folder: &str, uid: u32, thrown: bool) {
        let Some(ident) = self.accounts.get(account).map(|a| a.ident.clone()) else {
            return;
        };
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = index.key_at(&ident, folder, uid) {
            let _ = index.teach(&key, thrown);
        }
    }

    /// Learn from what was done with the messages `keys`: thrown away, or
    /// kept.
    pub(super) fn teach(&mut self, keys: &[crate::index::Key], thrown: bool) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        for key in keys {
            let _ = index.teach(key, thrown);
        }
    }

    pub(super) fn move_to_trash(
        &mut self,
        account: usize,
        from: String,
        uid: u32,
        message_id: String,
        trash: String,
    ) {
        // Thrown away: what is learnt of what you throw away.
        self.teach_at(account, &from, uid, true);
        let name = self.folder_label(account, &trash);
        self.status = format!("moved to {name} · d there puts it back");
        self.trash_quietly(account, from, uid, message_id, trash);
    }

    /// Move the message to the account's Trash, recorded to be put back,
    /// and nothing learnt of it.
    pub(super) fn trash_quietly(
        &mut self,
        account: usize,
        from: String,
        uid: u32,
        message_id: String,
        trash: String,
    ) {
        self.drop_listed(account, &from, uid);
        self.ask(
            account,
            Ask::Move {
                from,
                uid,
                to: trash,
                message_id,
                trashed: true,
            },
        );
    }

    /// Move the delete that was waiting. False when there was nothing to
    /// move, or Trash is still not known.
    pub(super) fn resume_held(&mut self, account: usize) -> bool {
        let Some(held) = self.held.take() else {
            return false;
        };
        if held.account != account {
            self.held = Some(held);
            return false;
        }
        let Some(trash) = self.trash_of(account) else {
            // Still waiting on which folder that is.
            if matches!(self.dialog, Dialog::Trash(_)) {
                self.held = Some(held);
            } else {
                self.status = "no Trash folder".to_string();
            }
            return false;
        };
        self.move_to_trash(account, held.from, held.uid, held.message_id, trash);
        true
    }

    /// The folder mail is moved to: the one chosen for this account, when
    /// it is still there, else the one the server marks as Trash.
    pub(super) fn trash_of(&self, account: usize) -> Option<String> {
        let account = self.accounts.get(account)?;
        if let Some(path) = &account.trash
            && account
                .folders
                .iter()
                .any(|f| f.path == *path && f.selectable)
        {
            return Some(path.clone());
        }
        account
            .folders
            .iter()
            .find(|f| f.role == Role::Trash && f.selectable)
            .map(|f| f.path.clone())
    }

    /// Give the chosen folder the Trash role, once the server has listed it.
    pub(super) fn remember_trash(&mut self, account: usize) {
        let Some(path) = self.accounts[account].trash.clone() else {
            return;
        };
        if let Some(folder) = self.folder_mut(account, &path)
            && folder.selectable
        {
            folder.role = Role::Trash;
        }
    }

    pub(super) fn inbox(&self, account: usize) -> String {
        self.accounts
            .get(account)
            .and_then(|a| {
                a.folders
                    .iter()
                    .find(|f| f.role == Role::Inbox)
                    .map(|f| f.path.clone())
            })
            .unwrap_or_else(|| "INBOX".to_string())
    }

    pub(super) fn folder_label(&self, account: usize, path: &str) -> String {
        self.folder_of(account, path)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| path.to_string())
    }

    /// Let go of the records of messages letterbox trashed more than a
    /// month ago: the messages stay in Trash, as its server keeps them.
    /// Nothing is ever removed for good by letterbox.
    pub(super) fn sweep(&mut self, account: usize) {
        let email = self
            .accounts
            .get(account)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        if email.is_empty() {
            return;
        }
        let now = jiff::Timestamp::now().as_second();
        let gone: Vec<(String, String, u32)> = self
            .ledger
            .stale(&email, now)
            .into_iter()
            .map(|e| (e.account, e.folder, e.uid))
            .collect();
        if !gone.is_empty()
            && let Err(e) = self.ledger.forget_many(&gone)
        {
            self.status = format!(
                "cannot update the trash record: {}",
                first_line(&format!("{e:#}"))
            );
        }
    }

    pub(super) fn forget_trashed(&mut self, email: &str, folder: &str, uid: u32) {
        if let Err(e) = self.ledger.forget(email, folder, uid) {
            self.status = format!(
                "cannot update the trash record: {}",
                first_line(&format!("{e:#}"))
            );
        }
    }

    pub(super) fn drop_listed(&mut self, account: usize, folder: &str, uid: u32) {
        self.drop_all_listed(&[(account, folder.to_string(), uid)]);
    }

    /// Take each of `gone`, by account, folder and UID, out of what is
    /// listed, and out of the reader; what is shown narrowed once, after.
    pub(super) fn drop_all_listed(&mut self, gone: &[(usize, String, u32)]) {
        for (account, folder, uid) in gone {
            let (account, uid) = (*account, *uid);
            if let Some(listing) = self
                .listing
                .as_mut()
                .and_then(|l| l.part_mut(account, folder))
            {
                let before = listing.messages.len();
                listing.messages.retain(|m| m.uid != uid);
                if listing.messages.len() != before {
                    listing.total = listing.total.saturating_sub(1);
                }
            }
            if let Some((messages, total)) = self.listed.get_mut(&(account, folder.clone())) {
                let before = messages.len();
                messages.retain(|m| m.uid != uid);
                if messages.len() != before {
                    *total = total.saturating_sub(1);
                }
            }
            if self
                .reading
                .as_ref()
                .is_some_and(|r| r.account == account && r.folder == *folder && r.uid == uid)
            {
                self.reading = None;
                self.drawn = None;
            }
        }
        if !gone.is_empty() {
            self.narrow();
        }
    }

    pub(super) fn reload(&mut self, account: usize) {
        let folders = self.listing.as_ref().map(Listing::keys).unwrap_or_default();
        for (_, folder) in folders.into_iter().filter(|(a, _)| *a == account) {
            self.ask_page(account, folder, 0);
        }
    }
}
