//! What the server reads of letterbox's map, and writes down for it to learn.

use super::*;

/// The map's index, read only, and the snapshot it makes; and where what
/// the user said yes to is written down, with the key's public half.
pub(super) struct Mapped {
    pub(super) index: crate::index::Index,
    pub(super) snapshot: crate::map::Snapshot,
    pub(super) advise: Option<(std::path::PathBuf, age::x25519::Recipient)>,
    /// Whom the user wrote to, as the filter reads beside each message.
    pub(super) known: crate::filter::Known,
    /// Each message by where it is: account, folder and UID.
    pub(super) places: std::collections::HashMap<(String, String, u32), crate::index::Key>,
}

impl Mapped {
    /// The message `uid` of `folder` of the account known as `ident`.
    pub(super) fn key(&self, ident: &str, folder: &str, uid: u32) -> Option<&crate::index::Key> {
        self.places
            .get(&(ident.to_string(), folder.to_string(), uid))
    }

    /// The message `s` of `folder` of the account known as `ident`.
    pub(super) fn key_of(
        &self,
        ident: &str,
        folder: &str,
        s: &Summary,
    ) -> Option<crate::index::Key> {
        self.key(ident, folder, s.uid)
            .cloned()
            .or_else(|| self.index.key_in_all(ident, &s.message_id))
    }

    /// How likely the message `key` is to be thrown away, as letterbox has
    /// learnt; whether it is sure enough to say so; and why.
    pub(super) fn junk(&self, key: &crate::index::Key) -> Option<(f32, bool, Vec<String>)> {
        let r = self.index.records.get(key)?;
        let f = &self.index.filter;
        let tokens = crate::filter::Filter::tokens(r, &self.known);
        let odds = f.odds(&tokens);
        Some((
            odds,
            f.ready() && odds >= crate::filter::SURE,
            f.why(&tokens),
        ))
    }
}

impl Server {
    /// The map's index, read only, and its snapshot: the demo's made from
    /// its mail; read as it is first asked of.
    pub(super) fn mapped(&mut self) -> anyhow::Result<&Mapped> {
        if self.map.is_none() {
            let made = self.read_map().map_err(|e| format!("{e:#}"));
            self.map = Some((made, std::time::Instant::now()));
        }
        match &self.map.as_ref().expect("read above").0 {
            Ok(m) => Ok(m),
            Err(e) => bail!("{e}"),
        }
    }

    /// Let the map go once it is old, to be read again as it is next asked
    /// of: only between tools, so that a place on it found in one stays the
    /// same message to the end of it.
    pub(super) fn forget_stale_map(&mut self) {
        if !self.demo
            && self
                .map
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed().as_secs() >= MAP_FRESH)
        {
            self.map = None;
        }
    }

    pub(super) fn read_map(&mut self) -> anyhow::Result<Mapped> {
        let mut advise = None;
        let index = if self.demo {
            let shared = std::sync::Mutex::new(crate::index::Index::memory());
            let mut twin = self.store(None)?.twin().context("the demo's mail")?;
            let stop = std::sync::atomic::AtomicBool::new(false);
            let now = jiff::Timestamp::now().as_second();
            crate::index::sync::pass(twin.as_mut(), &shared, now, &stop, &mut |_| {})?;
            shared.into_inner().unwrap_or_else(|e| e.into_inner())
        } else {
            // Its key in a vault with a key file is read without asking.
            if !crate::vault::is_open() {
                let _ = crate::vault::unlock_by_key();
            }
            let dir = crate::index::key::dir();
            let card = crate::index::key::card(&dir)?
                .context("the map is not begun: open letterbox and press M to begin it")?;
            let identity = crate::index::key::identity(&card, secret::find)?;
            advise = Some((dir.clone(), identity.to_public()));
            crate::index::Index::read_only(&dir, &identity)
        };
        let snapshot = crate::map::compute(crate::map::input(
            &index,
            crate::index::Lens::Both,
            None,
            None,
            false,
        ));
        let places = index
            .records
            .values()
            .flat_map(|r| {
                r.places.iter().map(|p| {
                    (
                        (r.key.account.clone(), p.folder.clone(), p.uid),
                        r.key.clone(),
                    )
                })
            })
            .collect();
        Ok(Mapped {
            known: index.known(),
            places,
            index,
            snapshot,
            advise,
        })
    }

    /// The message `uid` of `folder` of the account named, by its place in
    /// the map.
    pub(super) fn on_map(
        &mut self,
        account: Option<&str>,
        folder: &str,
        uid: u32,
    ) -> anyhow::Result<usize> {
        let ident = self.store(account)?.ident();
        if self.mapped()?.key(&ident, folder, uid).is_none() {
            let s = self.moving(account, folder, &[uid])?;
            return self.summary_on_map(account, folder, &s[0]);
        }
        let s = Summary {
            uid,
            ..Default::default()
        };
        self.summary_on_map(account, folder, &s)
    }

    /// As `on_map`, the message listed already.
    pub(super) fn summary_on_map(
        &mut self,
        account: Option<&str>,
        folder: &str,
        s: &Summary,
    ) -> anyhow::Result<usize> {
        let ident = self.store(account)?.ident();
        let m = self.mapped()?;
        m.key_of(&ident, folder, s)
            .and_then(|k| m.snapshot.find(&k))
            .with_context(|| format!("message {} of {folder} is not on the map yet", s.uid))
    }

    /// Write down that the user said yes to throwing away, or keeping, the
    /// `messages` of `folder`, as the map read them, for letterbox to
    /// learn; how many were, or why none could be.
    pub(super) fn advise(
        &mut self,
        account: Option<&str>,
        folder: &str,
        messages: &[&Summary],
        thrown: bool,
    ) -> Value {
        match self.written_down(account, folder, messages, thrown, false) {
            Ok(n) => json!(n),
            Err(e) => json!(format!("none: {}", said(&e))),
        }
    }

    /// Write down what the user decided of the messages `uids`, as `advise`
    /// says, `asked` if they answered as they were asked; how many.
    pub(super) fn written_down(
        &mut self,
        account: Option<&str>,
        folder: &str,
        messages: &[&Summary],
        thrown: bool,
        asked: bool,
    ) -> anyhow::Result<usize> {
        let ident = self.store(account)?.ident();
        let m = self.mapped()?;
        let at = jiff::Timestamp::now().as_second();
        let taught: Vec<crate::filter::Taught> = messages
            .iter()
            .filter_map(|s| m.key_of(&ident, folder, s))
            .filter_map(|key| m.index.lesson(&key, thrown, &m.known))
            .map(|t| crate::filter::Taught {
                agent: true,
                asked,
                at,
                ..t
            })
            .collect();
        let n = taught.len();
        match m.advise.clone() {
            Some((dir, recipient)) => crate::index::advise(&dir, &recipient, &taught)?,
            None => self.learnt.extend(taught.iter().cloned()),
        }
        // Seen here at once, before letterbox has taken it in.
        if let Some((Ok(m), _)) = &mut self.map {
            for t in taught {
                m.index.filter.learn(t);
            }
        }
        Ok(n)
    }

    /// What is worth asking the user, as letterbox reads it: at most
    /// `limit`, each by where it is.
    pub(super) fn uncertain(&mut self, limit: usize) -> anyhow::Result<Value> {
        let now = jiff::Timestamp::now().as_second();
        let (edges, days) = {
            let m = self.mapped()?;
            (
                m.index.edges(limit, now, &m.known),
                m.index.days_since_asked(now),
            )
        };
        let mut messages = Vec::new();
        for e in edges {
            messages.push(json!({
                "account": self.name_of(&e.key.account),
                "folder": e.place.as_ref().map(|p| p.folder.clone()),
                "uid": e.place.as_ref().map(|p| p.uid),
                "from": mail::clean(&e.from),
                "subject": mail::clean(&e.subject),
                "preview": mail::clean(&e.preview),
                "odds": e.odds,
                "why": e.why,
                "user_decided": e.decided.map(|t| if t { "threw it away" } else { "kept it" }),
            }));
        }
        Ok(json!({
            "due": days.is_none_or(|d| d >= ASK_EVERY),
            "days_since_asked": days,
            "messages": messages,
        }))
    }

    /// Ask the user of each of `moving`, one at a time, through letterbox:
    /// junk, or to keep, or to pass over. Learnt, never moved. As they stop
    /// answering, it stops.
    pub(super) fn ask_about(
        &mut self,
        account: Option<&str>,
        folder: &str,
        moving: &[Summary],
    ) -> anyhow::Result<Value> {
        // Asked before they are asked: their answers are not lost to it.
        let folders = self.store(account)?.folders()?;
        let of_drafts = mail::role_is(&folders, folder, mail::Role::Drafts);
        let schema = json!({ "answer": {
            "type": "string",
            "title": "Junk, or keep?",
            "enum": ["junk", "keep", "skip"]
        } });
        let (mut junk, mut keep) = (Vec::new(), Vec::new());
        let mut passed = 0;
        for (i, s) in moving.iter().enumerate() {
            let message = format!(
                "So letterbox learns: is this junk, or to keep? Nothing is moved.\n{}: {}\n{}",
                mail::clean(&s.from),
                mail::clean(&s.subject),
                mail::clean(&s.preview),
            );
            // As the program closes, what was answered is kept all the same.
            let result = match self.elicit(&message, schema.clone(), Some("answer")) {
                Ok(result) => result,
                Err(_) if i > 0 => break,
                Err(e) => return Err(e),
            };
            if result["action"] != "accept" {
                break;
            }
            match result["content"]["answer"].as_str() {
                Some("junk") => junk.push(s),
                Some("keep") => keep.push(s),
                _ => passed += 1,
            }
        }
        // Nothing said of a draft is learnt, though All Mail holds it too,
        // marked so or not.
        let learnt = if of_drafts {
            0
        } else {
            self.written_down(account, folder, &junk, true, true)?
                + self.written_down(account, folder, &keep, false, true)?
        };
        Ok(json!({
            "junk": junk.len(),
            "keep": keep.len(),
            "passed": passed,
            "not_asked": moving.len() - junk.len() - keep.len() - passed,
            "learnt": learnt,
        }))
    }
}
