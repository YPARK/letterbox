//! What the servers answer, taken in; and what was asked of them, moving
//! mail, followed till it is confirmed or failed.
use super::*;

impl App {
    /// What `ask` asks of `account`, to be followed till confirmed, if it
    /// moves mail where it was asked to: filed, archived or trashed.
    pub(super) fn job_of(&self, account: usize, ask: &Ask) -> Option<crate::confirm::Job> {
        use crate::confirm::{Goal, Job, State};
        if self.quiet {
            return None;
        }
        let (folder, uid, goal, key) = match ask {
            Ask::Move {
                from,
                uid,
                trashed: true,
                ..
            } => (from, *uid, Goal::Trashed, None),
            Ask::File {
                folder,
                uid,
                label,
                key,
                ..
            } => (folder, *uid, Goal::Filed(label.clone()), Some(key.clone())),
            Ask::Archive { folder, uid, .. } => (folder, *uid, Goal::Archived, None),
            _ => return None,
        };
        let key = key.or_else(|| {
            let ident = &self.accounts.get(account)?.ident;
            self.index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .key_at(ident, folder, uid)
        });
        let subject = key
            .as_ref()
            .and_then(|k| {
                let s = self.map.snapshot.as_ref()?;
                Some(s.dots[s.find(k)?].subject.clone())
            })
            .or_else(|| {
                self.shown
                    .iter()
                    .find(|l| l.account == account && &l.folder == folder && l.summary.uid == uid)
                    .map(|l| l.summary.subject.clone())
            })
            .unwrap_or_else(|| "a message".to_string());
        Some(Job {
            account,
            folder: folder.clone(),
            uid,
            choice: key.as_ref().and_then(|k| self.choosing.get(k).cloned()),
            key,
            subject,
            goal,
            state: State::Asked,
        })
    }

    /// Its server's answer for the message `uid` of `folder` of `account`,
    /// if it was followed: where nothing looks at the account's mail after,
    /// its word taken; failed, what was chosen for it in the Inbox looked
    /// through, chosen again, to be done again.
    pub(super) fn took(
        &mut self,
        account: usize,
        folder: &str,
        uid: u32,
        result: Result<(), &str>,
    ) {
        let result = result.map_err(|e| first_line(e).to_string());
        let failed = result.as_ref().err().cloned();
        if !self.batch.answered(account, folder, uid, result) {
            return;
        }
        // Failed: said at once, not only once the rest has settled.
        if let Some(why) = failed
            && let Some(j) = self
                .batch
                .jobs
                .iter()
                .find(|j| j.account == account && j.folder == folder && j.uid == uid)
        {
            self.status = format!("✗ {} {}: {why}", j.subject, j.goal.said());
        }
        if self.syncs.get(account).and_then(Option::as_ref).is_none() {
            self.batch.trust(account);
        }
        self.choose_again();
        self.say_settled();
    }

    /// A look at `account`'s mail: what it was asked to do, confirmed where
    /// it has left where it was; failed where, looked at again, it has not.
    pub(super) fn looked_again(&mut self, account: usize) {
        use crate::confirm::Goal;
        {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            self.batch.looked(account, |job| {
                let key = job.key.as_ref()?;
                let Some(record) = index.records.get(key) else {
                    return Some(true);
                };
                Some(match job.goal {
                    Goal::Trashed | Goal::Brought(_) => !record
                        .places
                        .iter()
                        .any(|p| p.folder == job.folder && p.uid == job.uid),
                    // As the server says it, not as filed before it does.
                    Goal::Filed(_) | Goal::Archived => {
                        !crate::index::labels::system(record, &index.folders).inbox
                    }
                })
            });
        }
        self.choose_again();
        self.say_settled();
    }

    /// What failed that was chosen in the Inbox looked through, chosen
    /// again, as it was.
    pub(super) fn choose_again(&mut self) {
        let again: Vec<(crate::index::Key, Choice)> = self
            .batch
            .failed()
            .filter_map(|j| Some((j.key.clone()?, j.choice.clone()?)))
            .collect();
        for (key, choice) in again {
            match &mut self.review {
                Some(r) => r.chosen.entry(key).or_insert(choice),
                None => self.review_kept.entry(key).or_insert(choice),
            };
        }
    }

    /// Once what was asked has all settled, and nothing else is open, a
    /// pop-up saying how it went; one message alone, done, said at the foot.
    pub(super) fn say_settled(&mut self) {
        if self.dialog.is_open() || self.compose.is_some() {
            return;
        }
        let Some(said) = self.batch.settled() else {
            return;
        };
        let counts = self.batch.counts();
        if counts.all == 1 && counts.failed == 0 {
            self.status = if self.status.is_empty() {
                format!("✓ confirmed: {}", self.batch.jobs[0].goal.said())
            } else {
                format!("{} · ✓ confirmed", self.status)
            };
            return;
        }
        let failed = self
            .batch
            .failed()
            .map(|j| {
                let why = match &j.state {
                    crate::confirm::State::Failed(why) => why.clone(),
                    _ => String::new(),
                };
                format!("{} {}: {why}", j.subject, j.goal.said())
            })
            .collect();
        self.dialog = Dialog::Applied(Box::new(Applied { said, failed }));
    }

    /// Take a server's answer: what it asks in turn is not remembered for
    /// C-x u, as it follows the answer, not a key.
    pub(super) fn take_reply(&mut self, reply: Reply) {
        let was = std::mem::replace(&mut self.replying, true);
        self.answer_reply(reply);
        self.replying = was;
    }

    fn answer_reply(&mut self, reply: Reply) {
        let a = reply.account;
        // How to sign in comes while signing in, not in answer to an ask.
        if let Answer::Waiting(waiting) = reply.answer {
            if let Some(form) = self.dialog.form_mut() {
                form.note = None;
                form.waiting = Some(waiting);
            }
            return;
        }
        // How far the map's sync has got is said as it goes.
        if let Answer::Mapping(said) = reply.answer {
            self.mapping = said;
            return;
        }
        // The address is said as the thread starts, not in answer to an ask.
        if let Answer::Address(email) = reply.answer {
            if let Some(account) = self.accounts.get_mut(a) {
                account.email = email;
            }
            self.sweep(a);
            return;
        }
        self.pending = self.pending.saturating_sub(1);
        if reply.answer.of_a_change() {
            self.changing = self.changing.saturating_sub(1);
        }
        if let Answer::Added(added) = reply.answer {
            return self.added(added);
        }
        if let Answer::Drawn(snapshot) = reply.answer {
            return self.drawn_map(*snapshot);
        }
        if let Answer::Relaid { lens, keys, at } = reply.answer {
            return self.relaid(lens, &keys, &at);
        }
        if let Answer::Checked { text, edits } = reply.answer {
            return self.checked(&text, edits);
        }
        if let Answer::Raw { folder, uid, raw } = reply.answer {
            return self.brought_raw(a, folder, uid, raw);
        }
        if let Answer::Brought { from, result } = reply.answer {
            return self.brought(from, result);
        }
        #[cfg(feature = "pages")]
        if let Answer::Paged { place, pages } = reply.answer {
            // Let go, or of a message no longer read: not shown, nor said.
            if self.typesetting.as_ref() != Some(&place) {
                return;
            }
            self.typesetting = None;
            if self.read_place().as_ref() != Some(&place) {
                return;
            }
            match pages {
                Ok(pages) if !pages.is_empty() => {
                    self.paged = Some(crate::pages::Paged::new(place, pages));
                    self.status = "its pages: j k turn them, V its text".to_string();
                }
                Ok(_) => self.status = "it has no pages to show".to_string(),
                Err(e) => self.status = format!("cannot typeset it: {}", first_line(&e)),
            }
            return;
        }
        if let Answer::Standings { asked, said } = reply.answer {
            // Of the papers listed, as they still are, alone.
            if let Dialog::Papers(p) = &mut self.dialog
                && p.asked == asked
                && said.as_ref().map_or(true, |s| s.len() == p.papers.len())
            {
                match said {
                    Ok(standing) => {
                        p.standing = standing.into_iter().map(Some).collect();
                        p.note = None;
                    }
                    Err(e) => p.note = Some(first_line(&e).to_string()),
                }
            }
            return;
        }
        if let Answer::Shelved { asked, found } = reply.answer {
            match (found, &mut self.dialog) {
                // Done: the list put away, and what was done said below.
                (Ok(f), dialog) => {
                    if matches!(dialog, Dialog::Papers(p) if p.asked == asked) {
                        self.dialog = Dialog::Closed;
                    }
                    self.status = format!("in paperstack, starred, as {}", f.key);
                }
                (Err(e), Dialog::Papers(p)) if p.asked == asked => {
                    p.note = Some(first_line(&e).to_string());
                }
                (Err(e), _) => self.status = first_line(&e).to_string(),
            }
            return;
        }
        if let Answer::Fetched(got) = reply.answer {
            match got {
                Ok(path) => {
                    if let Some(files) = self.dialog.files_mut() {
                        files.note = None;
                    }
                    self.launch = Some(Launch::Ohp(path, None));
                }
                Err(e) => match self.dialog.files_mut() {
                    Some(files) => files.note = Some(e),
                    None => self.status = first_line(&e).to_string(),
                },
            }
            return;
        }
        if let Answer::Realm { email, microsoft } = reply.answer {
            if let Some(form) = self.dialog.form_mut()
                && microsoft
                && form.fields[EMAIL].trim() == email
                && !form.checking
            {
                form.microsoft();
            }
            return;
        }
        if self.accounts.get(a).is_none_or(|x| x.asks.is_none()) {
            return;
        }
        match reply.answer {
            Answer::Folders(Ok(folders)) => {
                let first = self.accounts.iter().all(|x| x.folders.is_empty());
                self.accounts[a].folders = folders;
                self.remember_trash(a);
                self.accounts[a].state = State::Ready;
                self.rebuild();
                if first && self.listing.is_none() {
                    let inbox = self.accounts[a]
                        .folders
                        .iter()
                        .position(|f| f.selectable)
                        .and_then(|i| self.rows.iter().position(|r| *r == Row::Folder(a, i)));
                    if let Some(row) = inbox {
                        self.row = row;
                        self.open_folder();
                    }
                }
                let paths: Vec<String> = self.accounts[a]
                    .folders
                    .iter()
                    .filter(|f| f.selectable)
                    .map(|f| f.path.clone())
                    .collect();
                for p in paths {
                    self.ask(a, Ask::Count(p));
                }
                self.join_view(a);
                self.sweep(a);
                self.resume_held(a);
            }
            Answer::Folders(Err(e)) => {
                self.status = format!("{}: {}", self.accounts[a].name, first_line(&e));
                self.accounts[a].state = State::Failed(e);
                self.rebuild();
            }
            Answer::Count {
                folder,
                counts: Ok((total, unseen)),
            } => {
                if let Some(f) = self.folder_mut(a, &folder) {
                    f.total = Some(total);
                    f.unseen = Some(unseen);
                }
            }
            Answer::Count { counts: Err(_), .. }
            | Answer::Skipped
            | Answer::Added(_)
            | Answer::Waiting(_)
            | Answer::Address(_)
            | Answer::Fetched(_)
            | Answer::Standings { .. }
            | Answer::Shelved { .. }
            | Answer::Mapping(_)
            | Answer::Drawn(_)
            | Answer::Relaid { .. }
            | Answer::Checked { .. }
            | Answer::Raw { .. }
            | Answer::Brought { .. }
            | Answer::Realm { .. } => {}
            #[cfg(feature = "pages")]
            Answer::Paged { .. } => {}
            Answer::Filed {
                key,
                label,
                folder,
                uid,
                filed,
            } => {
                let result: Result<(), String> = filed.as_ref().map(|_| ()).map_err(Clone::clone);
                let landed = filed.as_ref().map(|f| f.moved.clone()).map_err(|_| ());
                self.move_answered(a, &folder, uid, landed);
                if result.is_ok() {
                    // Out of it: what it holds, and holds unread, counted again.
                    self.ask(a, Ask::Count(folder.clone()));
                }
                self.filed(a, key, label, filed);
                self.took(
                    a,
                    &folder,
                    uid,
                    result.as_ref().map(|_| ()).map_err(String::as_str),
                );
            }
            Answer::LabelRenamed { old, result } => match result {
                Ok(()) => self.map_pass(a),
                Err(e) => self.status = format!("cannot rename {old}: {}", first_line(&e)),
            },
            Answer::Archived {
                folder,
                uid,
                result,
            } => {
                let landed = result.as_ref().map(|()| None).map_err(|_| ());
                self.move_answered(a, &folder, uid, landed);
                match &result {
                    Ok(()) => {
                        self.ask(a, Ask::Count(folder.clone()));
                        self.reload(a);
                        self.map_pass(a);
                    }
                    Err(e) => {
                        self.status = format!("cannot archive it: {}", first_line(e));
                        self.reload(a);
                    }
                }
                self.took(
                    a,
                    &folder,
                    uid,
                    result.as_ref().map(|_| ()).map_err(String::as_str),
                );
            }
            Answer::Mapped(passed) => {
                // A look that failed is one that cannot tell: as many such,
                // and its server's word is taken.
                if passed.is_ok() {
                    self.moves_looked(a);
                    self.looked_again(a);
                } else {
                    self.batch.looked(a, |_| None);
                    self.say_settled();
                }
                self.mapping = match passed {
                    Ok(_) => String::new(),
                    Err(e) => format!(
                        "the map cannot read {}: {}",
                        self.accounts[a].name,
                        first_line(&e)
                    ),
                };
            }
            Answer::Page {
                folder,
                offset,
                page,
            } => {
                let Some(l) = self.listing.as_mut().and_then(|l| l.part_mut(a, &folder)) else {
                    return;
                };
                l.loading = false;
                match page {
                    Ok(page) => {
                        l.error = None;
                        l.total = page.total;
                        if offset == 0 {
                            // The newest, and those listed before older than them.
                            let oldest = page.summaries.iter().map(|s| s.uid).min();
                            let full = page.summaries.len() >= PAGE;
                            let mut messages = page.summaries;
                            if full && let Some(oldest) = oldest {
                                messages.extend(l.messages.drain(..).filter(|m| m.uid < oldest));
                            }
                            l.messages = messages;
                        } else {
                            let have: std::collections::HashSet<u32> =
                                l.messages.iter().map(|m| m.uid).collect();
                            l.messages.extend(
                                page.summaries
                                    .into_iter()
                                    .filter(|s| !have.contains(&s.uid)),
                            );
                        }
                        let total = l.total;
                        if let Some(f) = self.folder_mut(a, &folder) {
                            f.total = Some(total);
                        }
                    }
                    Err(e) => {
                        self.status = first_line(&e).to_string();
                        l.error = Some(e);
                    }
                }
                self.narrow();
            }
            Answer::Opened {
                folder,
                uid,
                message,
                marked,
            } => {
                if let Ok(m) = &message {
                    self.read.insert((a, folder.clone(), uid), m.clone());
                }
                if marked {
                    self.marked(a, &folder, uid, Flag::Seen, true);
                }
                if let Some(r) = self
                    .reading
                    .as_mut()
                    .filter(|r| r.account == a && r.folder == folder && r.uid == uid)
                {
                    r.body = match message {
                        Ok(m) => Body::Ready(m),
                        Err(e) => Body::Failed(e),
                    };
                }
                if let Some(p) = self
                    .reply_when
                    .take_if(|p| p.account == a && p.folder == folder && p.uid == uid)
                    && let Some(m) = self.read.get(&(a, folder.clone(), uid))
                {
                    self.open_reply(a, &folder, &m, p.how);
                    if let Some(path) = p.attach {
                        self.attach(&[path]);
                    }
                }
            }
            Answer::Set {
                folder,
                uid,
                flag,
                on,
                result,
            } => {
                if let Err(e) = result {
                    self.status = format!("cannot mark it: {}", first_line(&e));
                    self.marked(a, &folder, uid, flag, !on);
                }
            }
            Answer::Found {
                folder,
                words,
                found,
            } => {
                let current = self.listing.as_ref().is_some_and(|l| l.has(a, &folder));
                if !current || words != self.query.trim() {
                    return;
                }
                match found {
                    Ok(found) => {
                        // Each account's answer joins those come before.
                        let mut all = self.found.take().unwrap_or_default();
                        all.retain(|m| m.account != a || m.folder != folder);
                        all.extend(found.into_iter().map(|s| Listed::new(a, &folder, s)));
                        // In the list's order: folders together newest first.
                        if self.listing.as_ref().is_some_and(Listing::mixed) {
                            all.sort_by_key(|m| newest(m));
                        }
                        self.status = format!("{} found in {}", all.len(), self.folder_name());
                        self.found = Some(all);
                    }
                    Err(e) => self.status = format!("cannot search: {}", first_line(&e)),
                }
                self.narrow();
            }
            Answer::Moved {
                from,
                uid,
                to,
                message_id,
                trashed,
                new_uid,
                email,
            } => {
                // Its following taken last, as what it says of a failure is
                // the more: the message, and what was asked.
                let result: Result<(), String> = new_uid.as_ref().map(|_| ()).map_err(Clone::clone);
                let landed = new_uid
                    .as_ref()
                    .map(|&n| Some((to.clone(), n)))
                    .map_err(|_| ());
                self.move_answered(a, &from, uid, landed);
                let was = from.clone();
                match new_uid {
                    Ok(new_uid) => {
                        if !email.is_empty() {
                            self.accounts[a].email = email.clone();
                        }
                        if trashed {
                            let at = jiff::Timestamp::now().as_second();
                            let entry = Entry {
                                account: email,
                                message_id,
                                folder: to.clone(),
                                uid: new_uid,
                                origin: from.clone(),
                                at,
                            };
                            if let Err(e) = self.ledger.record(entry) {
                                self.status = format!("moved to Trash, and not recorded: {e:#}");
                            }
                        } else {
                            let email = self.accounts[a].email.clone();
                            self.forget_trashed(&email, &from, uid);
                            self.refile_moved(a, &from, uid, Ok((to.clone(), new_uid, message_id)));
                        }
                        self.ask(a, Ask::Count(from));
                        self.ask(a, Ask::Count(to));
                        self.reload(a);
                        self.map_pass(a);
                    }
                    Err(e) => {
                        if !trashed {
                            self.refile_moved(a, &from, uid, Err(e.clone()));
                        }
                        self.status = format!("cannot move it: {}", first_line(&e));
                        self.reload(a);
                    }
                }
                self.took(
                    a,
                    &was,
                    uid,
                    result.as_ref().map(|_| ()).map_err(String::as_str),
                );
            }
            Answer::FolderMade { path, result } => self.refile_folder(a, &path, true, result),
            Answer::FolderDeleted { path, result } => {
                if let Err(e) = &result {
                    self.status = format!("{path} kept: {}", first_line(e));
                }
                self.refile_folder(a, &path, false, result)
            }
            Answer::Relabelled {
                folder,
                uid,
                off,
                on,
                result,
            } => {
                self.refile_relabelled(a, folder, uid, off, on, result);
                self.map_pass(a);
            }
            Answer::Sent { said, answer } => {
                if let Some(compose) = &mut self.compose {
                    compose.sending = false;
                }
                match said {
                    Ok(said) => {
                        if let Some((folder, uid)) = answer {
                            self.marked(a, &folder, uid, Flag::Answered, true);
                        }
                        self.compose = None;
                        self.status = match self.send_note.take() {
                            Some(note) => format!("{said} · {note}"),
                            None => said,
                        };
                        self.reload(a);
                    }
                    Err(e) => self.status = first_line(&e).to_string(),
                }
            }
        }
    }
}
