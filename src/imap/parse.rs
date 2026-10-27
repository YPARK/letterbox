//! What a server can do, as it says, and what it fetches read into the
//! index: UID sets, Gmail's IDs and labels, the UIDs a move gives.
use super::*;

/// What a connection can do, as far as the map asks.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Caps {
    /// Gmail's labels and IDs: `X-GM-EXT-1`.
    pub(super) gmail: bool,
    /// Mod-sequences: CONDSTORE, asked for on the connection.
    pub(super) condstore: bool,
}

/// What the index fetches of each message: what is listed, and its
/// `List-Id`; on Gmail, its IDs and labels too.
pub(crate) fn indexed_items(gmail: bool) -> String {
    let gmail = if gmail {
        " X-GM-MSGID X-GM-THRID X-GM-LABELS"
    } else {
        ""
    };
    format!(
        "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID \
         IN-REPLY-TO REFERENCES LIST-ID LIST-UNSUBSCRIBE PRECEDENCE CONTENT-TYPE \
         CONTENT-TRANSFER-ENCODING \
         MIME-VERSION)] \
         BODY.PEEK[TEXT]<0.2048>{gmail})"
    )
}

/// `uids` as an IMAP set, runs of them as ranges: `1:4,7,9:12`.
pub(crate) fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
        i += 1;
    }
    parts.join(",")
}

/// Each `FETCH` in a server's answer, with what it holds: read here, not
/// by the imap crate, which keeps Gmail's IDs to itself.
pub(super) fn fetched(buf: &[u8]) -> Vec<Vec<imap_proto::AttributeValue<'_>>> {
    let mut out = Vec::new();
    let mut rest = buf;
    while !rest.is_empty() {
        match imap_proto::parser::parse_response(rest) {
            Ok((left, imap_proto::Response::Fetch(_, attributes))) => {
                out.push(attributes);
                rest = left;
            }
            Ok((left, _)) => rest = left,
            Err(_) => break,
        }
    }
    out
}

/// A message as the index takes it, from what was fetched of it.
pub(super) fn indexed(attributes: Vec<imap_proto::AttributeValue<'_>>) -> Option<Indexed> {
    use imap_proto::{AttributeValue as A, MessageSection, SectionPath};
    let mut i = Indexed::default();
    let (mut uid, mut header, mut text) = (None, Vec::new(), Vec::new());
    for a in attributes {
        match a {
            A::Uid(u) => uid = Some(u),
            A::Flags(names) => {
                let (flags, keywords) = flags_named(names.iter().map(|n| n.as_ref()));
                i.flags = flags;
                i.labels.extend(keywords);
            }
            A::Rfc822Size(n) => i.size = n,
            A::GmailLabels(labels) => i.labels.extend(labels.iter().map(|l| l.to_string())),
            A::GmailMsgId(id) => i.gmail = Some(id),
            A::GmailThrId(id) => i.thread = Some(id),
            A::ModSeq(m) => i.modseq = Some(m),
            A::BodySection {
                section: Some(SectionPath::Full(MessageSection::Header)),
                data: Some(d),
                ..
            } => header = d.to_vec(),
            A::BodySection {
                section: Some(SectionPath::Full(MessageSection::Text)),
                data: Some(d),
                ..
            } => text = d.to_vec(),
            _ => {}
        }
    }
    i.uid = uid?;
    header.extend(text);
    i.raw = header;
    Some(i)
}

pub(super) fn gmail(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "imap.gmail.com" | "imap.googlemail.com"
    )
}

/// Where a moved message was found again, or why it was not.
pub(super) enum Found {
    Uid(u32),
    /// It has no `Message-ID`, and was left unmoved.
    NoId,
    /// It was moved, and the search did not find it.
    Lost,
}

/// The one UID `COPYUID` names as the copy, when the response has one.
pub(super) fn copied_uid(response: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(response);
    let rest = text.split("[COPYUID ").nth(1)?;
    let inside = rest.split(']').next()?;
    let mut parts = inside.split_whitespace();
    parts.next()?;
    parts.next()?;
    let dest = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if let Ok(uid) = dest.parse() {
        return Some(uid);
    }
    let (start, end) = dest.split_once(':')?;
    let start: u32 = start.parse().ok()?;
    let end: u32 = end.parse().ok()?;
    (start == end).then_some(start)
}

/// `UID MOVE` or `UID COPY`, and the UID the copy was given when the
/// server says it. `uid_mv` checks the response and then drops it.
pub(super) fn transferred(
    s: &mut Session,
    command: &str,
    uid: u32,
    to: &str,
) -> imap::Result<Option<u32>> {
    let (buf, _) = s.run(format!("UID {command} {uid} {}", astring(to)))?;
    Ok(copied_uid(&buf))
}

/// The message's `Message-ID`, from its header only. The search for a
/// moved message uses it when the server did not send `COPYUID`.
pub(super) fn message_id_of(s: &mut Session, uid: u32) -> imap::Result<Option<String>> {
    let fetches = s.uid_fetch(uid.to_string(), "BODY.PEEK[HEADER]")?;
    let raw = fetches.iter().find_map(|f| f.header()).unwrap_or_default();
    Ok(mail::field(raw, "Message-ID"))
}
