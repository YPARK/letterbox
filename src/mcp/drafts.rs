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
