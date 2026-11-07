//! Letters an agent is asked to write: kept in Drafts, never sent, and what
//! it is told of each, to go by without listing Drafts again.

use super::*;

impl Server {
    /// `draft_reply`, or `draft_message`, as `args` say.
    pub(super) fn draft(&mut self, name: &str, args: &Value) -> anyhow::Result<Value> {
        let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        let account = text("account");
        let files = attached(args, &self.config)?;
        let store = self.store(account.as_deref())?;
        let me = store.address().to_string();
        if me.is_empty() {
            bail!("this account has no address to write from");
        }
        let body = text("body").context("give what the letter says")?;
        let (raw, not_copied) = if name == "draft_reply" {
            let folder = text("folder").unwrap_or_else(|| "INBOX".to_string());
            let uid = args
                .get("uid")
                .and_then(Value::as_u64)
                .context("give the message's uid")? as u32;
            let all = args.get("all").and_then(Value::as_bool) == Some(true);
            let m = mail::message(uid, store.raw(&folder, uid)?)?;
            let left = if all {
                Vec::new()
            } else {
                crate::compose::not_copied(&m, &me)
            };
            (
                crate::compose::reply_draft(&m, &me, all, &body, &files),
                Some(left),
            )
        } else {
            let subject = text("subject").unwrap_or_default();
            let (to, cc) = (addresses(args, "to")?, addresses(args, "cc")?);
            (
                crate::compose::new_draft(&me, &to, &cc, &subject, &body, &files),
                None,
            )
        };
        let raw = raw.map_err(anyhow::Error::msg)?;
        // As it is kept, not as it was asked for: its headers alone, as its
        // files may be many bytes.
        let kept = mail::message(0, raw[..mail::header_end(&raw)].to_vec())?;
        // The bytes are in the letter now, and may be many.
        let attached: Vec<Value> = files
            .into_iter()
            .map(|f| json!({ "name": f.name, "size": f.bytes.len() }))
            .collect();
        let drafts = drafts_of(&store.folders()?)?;
        let uid = store.append_draft(&drafts, &raw)?;
        let mut said = "kept in Drafts, not sent: the user reads it, changes it and sends it \
                        themselves, with E on it in letterbox, or from their mail app, or says \
                        yes to send_draft"
            .to_string();
        if not_copied.as_ref().is_some_and(|n| !n.is_empty()) {
            said.push_str(
                ". It goes to the sender alone: tell the user who is not_copied, \
                 who a reply drafted with all would copy",
            );
        }
        let mut told = json!({
            "account": self.called(account.as_deref()),
            "uid": uid,
            "saved_in": drafts,
            "subject": kept.subject,
            "to": kept.to,
            "cc": kept.cc,
            "attached": attached,
            "said": said,
        });
        if let Some(left) = not_copied {
            told["not_copied"] = json!(left);
        }
        Ok(told)
    }
}

impl Server {
    /// `send_draft`: the draft of the uid `args` say, from the account's
    /// Drafts and no other folder, sent once the user, shown all that goes,
    /// says yes; its draft then kept in Trash. An agent's word is not enough:
    /// it reads mail from strangers, who may ask it for anything.
    pub(super) fn send_draft(&mut self, args: &Value) -> anyhow::Result<Value> {
        let account = args.get("account").and_then(Value::as_str);
        let uid = args
            .get("uid")
            .and_then(Value::as_u64)
            .context("give the draft's uid")? as u32;
        let store = self.store(account)?;
        let folders = store.folders()?;
        let drafts = drafts_of(&folders)?;
        if let Some(asked) = args.get("folder").and_then(Value::as_str)
            && asked != drafts
        {
            bail!(
                "only what is in {drafts} is sent: {asked} is not, and a uid is its folder's own"
            );
        }
        let raw = store
            .raw(&drafts, uid)
            .with_context(|| format!("there is no draft {uid} in {drafts}"))?;
        let m = mail::message(uid, raw)?;
        let (to, cc) = goes_as_it_is(&m)?;
        let me = store.address().to_string();
        let answer = self.confirm(&going(&m, &to, &cc))?;
        if let Some(why) = answer.refused("sent") {
            return Ok(json!({ "sent": false, "said": why }));
        }
        let store = self.store(account)?;
        let said_of_it = store.send(&crate::compose::to_send(&m.raw, &me))?;
        // It has gone: what follows is told of, never a failure to send.
        if let Some(id) = mail::field(&m.raw, "In-Reply-To") {
            self.answered(account, &mail::brackets(&id));
        }
        let draft = self
            .draft_gone(account, &folders, &drafts, &m)
            .unwrap_or_else(|e| format!("not told where it went from {drafts}: {}", said(&e)));
        Ok(json!({
            "sent": true,
            "account": self.called(account),
            "subject": m.subject,
            "to": to,
            "cc": cc,
            "said": said_of_it,
            "draft": draft,
        }))
    }

    /// The message of the Message-ID `id`, answered: marked so where the map
    /// knows it to be, else in the Inbox, and learnt as kept, as answering
    /// teaches in letterbox.
    fn answered(&mut self, account: Option<&str>, id: &str) {
        let Ok(store) = self.store(account) else {
            return;
        };
        let ident = store.ident();
        let mapped = self.mapped().ok().and_then(|m| {
            let p = m.index.by_message_id(&ident, id)?.places.first()?;
            Some((p.folder.clone(), p.uid))
        });
        let Ok(store) = self.store(account) else {
            return;
        };
        let place = mapped.or_else(|| {
            let uid = store.uid_of("INBOX", id).ok().flatten()?;
            Some(("INBOX".to_string(), uid))
        });
        let Some((folder, uid)) = place else {
            return;
        };
        let _ = store.set(&folder, uid, Flag::Answered, true);
        if let Ok(found) = self.moving(account, &folder, &[uid]) {
            let found: Vec<&Summary> = found.iter().collect();
            self.advise(account, &folder, &found, false);
        }
    }

    /// The draft `m`, sent, moved to Trash as one sent in letterbox is, to be
    /// put back, and nothing learnt: what became of it.
    fn draft_gone(
        &mut self,
        account: Option<&str>,
        folders: &[Folder],
        drafts: &str,
        m: &mail::Message,
    ) -> anyhow::Result<String> {
        let to = self.destination(account, folders, drafts, crate::report::Action::Trash, None)?;
        let id = m
            .message_id
            .as_deref()
            .map(mail::brackets)
            .unwrap_or_default();
        let store = self.store(account)?;
        let address = store.address().to_string();
        let uid = store.relocate(drafts, m.uid, &to, &id)?;
        let entry = crate::trash::Entry {
            account: address,
            message_id: id,
            folder: to.clone(),
            uid,
            origin: drafts.to_string(),
            at: jiff::Timestamp::now().as_second(),
        };
        Ok(match self.record_trashed(vec![entry]) {
            Some(why) => format!("moved to {to}, not recorded to put back: {why}"),
            None => format!("moved to {to}"),
        })
    }

    /// `trashed` recorded, to be put back; why not, if it could not be.
    pub(super) fn record_trashed(&self, trashed: Vec<crate::trash::Entry>) -> Option<String> {
        (!self.demo && !trashed.is_empty())
            .then(|| crate::trash::Ledger::load().record_many(trashed).err())
            .flatten()
            .map(|e| said(&e))
    }
}

/// The path of the account's Drafts, among its `folders`.
fn drafts_of(folders: &[Folder]) -> anyhow::Result<String> {
    Ok(mail::role_folder(folders, mail::Role::Drafts)
        .context("this account has no Drafts")?
        .path
        .clone())
}

/// The most of a draft's text the user can be shown in asking to send it.
const SHOWN_CHARS: usize = 20_000;

/// Who the draft `m` goes to, To and Cc, as it will be sent: or why it is left
/// for the user, as what they are shown must be all that goes, unchanged.
fn goes_as_it_is(m: &mail::Message) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    const THEIRS: &str = "the user sends it themselves, with E in letterbox or from their mail app";
    let raw = &m.raw;
    if mail::field(raw, "Bcc").is_some() {
        bail!("it has a Bcc, which letterbox cannot send: {THEIRS}");
    }
    if m.html.is_some() {
        bail!("it has an HTML part, which cannot be shown to the user here: {THEIRS}");
    }
    if m.text.chars().count() > SHOWN_CHARS {
        bail!("it is too long to show the user whole here: {THEIRS}");
    }
    if std::str::from_utf8(raw).is_err() {
        bail!("it is not UTF-8, and would not go as it is: {THEIRS}");
    }
    let multipart = mail::field(raw, "Content-Type")
        .is_some_and(|t| t.to_ascii_lowercase().starts_with("multipart/"));
    if multipart && !raw.is_ascii() {
        bail!("it has 8-bit parts, and may not go as it is: {THEIRS}");
    }
    let (to, cc) = (
        crate::smtp::envelope(raw, "To")?,
        crate::smtp::envelope(raw, "Cc")?,
    );
    if to.is_empty() && cc.is_empty() {
        bail!("it goes to no one");
    }
    if to.len() + cc.len() > crate::compose::MANY {
        bail!(
            "it goes to more than {} people, which an agent does not send: {THEIRS}",
            crate::compose::MANY
        );
    }
    Ok((to, cc))
}

/// What the user is asked of a draft to send: all that goes, and to the
/// addresses it goes to, not the names they come with.
fn going(m: &mail::Message, to: &[String], cc: &[String]) -> String {
    let mut text = "Send this now? It cannot be taken back.".to_string();
    text.push_str(&format!("\nFrom: {}\nTo: {}", m.from, to.join(", ")));
    if !cc.is_empty() {
        text.push_str(&format!("\nCc: {}", cc.join(", ")));
    }
    text.push_str(&format!("\nSubject: {}", m.subject));
    for f in &m.attachments {
        text.push_str(&format!("\nFile: {}, {}", f.name, crate::ui::size(f.size)));
    }
    text.push_str(&format!("\n\n{}", m.text));
    mail::clean_lines(&text)
}

/// What tells a draft from another of the same subject, as the user is asked
/// about it: when it was written, and how big it is.
pub(super) fn told_apart(s: &Summary) -> String {
    let when = s.date.map(crate::ui::long_date).unwrap_or_default();
    format!(" [{when}, {}]", crate::ui::size(s.size as usize))
}

/// The most a draft's files may come to, as most servers take a message of
/// 25 MB, and base64 makes a file a third larger.
pub(crate) const ATTACHED_AT_MOST: u64 = 18 * 1024 * 1024;

/// Whether `real`, a path links are resolved in, is something no agent attaches:
/// hidden, within what is hidden, or among what the system or letterbox keeps
/// to itself, the sign-ins and the keychain among it. Files in the cloud folders
/// of `~/Library` are the user's own.
pub(crate) fn off_limits(real: &Path, config: &Path) -> bool {
    // macOS's other name for the same files: resolving a path leaves it so.
    let real = match real.strip_prefix("/System/Volumes/Data") {
        Ok(rest) => Path::new("/").join(rest),
        Err(_) => real.to_path_buf(),
    };
    let hidden = real
        .components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
    let raw_home = crate::compose::home();
    let home = raw_home.canonicalize().unwrap_or(raw_home);
    // The folder of a file of letterbox's, unless that is all there is.
    let beside = |file: &Path| {
        file.parent()
            .filter(|p| p.parent().is_some() && *p != home)
            .map(Path::to_path_buf)
    };
    let vault = crate::vault::path();
    let own = [
        Some(config.to_path_buf()),
        beside(config),
        beside(&vault),
        Some(vault),
        Some(crate::vault::key_path()),
        Some(crate::index::key::dir()),
    ];
    let library = real.strip_prefix(home.join("Library")).is_ok_and(|rest| {
        !rest.starts_with("Mobile Documents") && !rest.starts_with("CloudStorage")
    });
    hidden
        || library
        || ["/proc", "/sys", "/dev", "/etc", "/private/etc"]
            .iter()
            .any(|p| real.starts_with(p))
        || own
            .into_iter()
            .flatten()
            .filter_map(|p| p.canonicalize().ok())
            .any(|p| real.starts_with(p))
}

/// The files a letter is to take, as an agent names them. A file named in
/// mail an agent read could be anything on this machine, so only a whole path,
/// to a file not `off_limits`, whatever a link makes it look like; none of it,
/// nothing is drafted.
fn attached(args: &Value, config: &Path) -> anyhow::Result<Vec<crate::compose::File>> {
    let Some(given) = args.get("attachments") else {
        return Ok(Vec::new());
    };
    let paths: Vec<String> =
        serde_json::from_value(given.clone()).context("attachments is a list of paths")?;
    let mut total = 0;
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        if !path.starts_with('/') && !path.starts_with("~/") {
            bail!("{path}: give its whole path");
        }
        let named = crate::compose::path_of(path);
        let cannot = || format!("{path}: cannot read it");
        let real = named.canonicalize().with_context(cannot)?;
        if off_limits(&real, config) {
            bail!(
                "{path} is hidden, or among what this machine keeps to itself: the user \
                 attaches it themselves, in letterbox"
            );
        }
        let about = real.metadata().with_context(cannot)?;
        if !about.is_file() {
            bail!("{path} is not a file");
        }
        total += about.len();
        if total > ATTACHED_AT_MOST {
            bail!(
                "too much to attach: {} MB at most",
                ATTACHED_AT_MOST / (1024 * 1024)
            );
        }
        // By the name it was given, not that of what a link leads to.
        files.push(crate::compose::read_file(&named).with_context(cannot)?);
    }
    Ok(files)
}

/// The addresses `args` give as `key`: a list of them, or none given.
fn addresses(args: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    let Some(given) = args.get(key) else {
        return Ok(Vec::new());
    };
    serde_json::from_value(given.clone()).with_context(|| format!("{key} is a list of addresses"))
}
