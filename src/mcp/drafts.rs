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
        let head = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map_or(raw.len(), |at| at + 4);
        let kept = mail::message(0, raw[..head].to_vec())?;
        let attached: Vec<Value> = files
            .iter()
            .map(|f| json!({ "name": f.name, "size": f.bytes.len() }))
            .collect();
        drop(files);
        let drafts = mail::role_folder(&store.folders()?, mail::Role::Drafts)
            .context("this account has no Drafts")?
            .path
            .clone();
        let uid = store.append_draft(&drafts, &raw)?;
        let mut said = "kept in Drafts, not sent: the user reads it, changes it and sends it \
                        themselves, with E on it in letterbox, or from their mail app"
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

/// What tells a draft from another of the same subject, as the user is asked
/// about it: when it was written, and how big it is.
pub(super) fn told_apart(s: &Summary) -> String {
    let when = s.date.and_then(|d| jiff::Timestamp::from_second(d).ok());
    let when = when.map(|t| {
        t.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M, ")
            .to_string()
    });
    format!(
        " [{}{} KB]",
        when.unwrap_or_default(),
        s.size.div_ceil(1024).max(1)
    )
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
