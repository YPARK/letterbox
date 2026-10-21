//! `letterbox --demo`: an account of made-up mail, in memory, dated from
//! now, to try letterbox with before setting up an account.

use crate::mail::{self, Flags, Folder, Role, Summary};
use crate::store::{Flag, Page, Removal, Store};
use jiff::{ToSpan, Zoned};
use std::collections::BTreeMap;

struct Letter {
    uid: u32,
    flags: Flags,
    raw: Vec<u8>,
}

pub struct Demo {
    /// Each folder's messages, oldest first, as a server numbers them.
    folders: BTreeMap<String, Vec<Letter>>,
    /// The next UID, across every folder: a move gives the message a new one.
    next: u32,
}

/// A message, `hours` ago.
fn letter(
    hours: i64,
    from: &str,
    to: &str,
    subject: &str,
    head: &str,
    body: &str,
) -> (i64, Vec<u8>) {
    let when = Zoned::now()
        .checked_sub(hours.hours())
        .unwrap_or_else(|_| Zoned::now());
    let date = when.strftime("%a, %d %b %Y %H:%M:%S %z");
    let raw = format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {date}\r\n\
         Message-ID: <{hours}.{}@demo.letterbox>\r\nMIME-Version: 1.0\r\n{head}\r\n{body}",
        subject.len()
    );
    (
        hours,
        raw.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes(),
    )
}

/// The paper Turing attaches: a page of it, as a PDF, to read in ohp.
const PAPER: &str = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2Jq\
CjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2Jq\
CjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIg\
NzkyXSAvQ29udGVudHMgNCAwIFIgL1Jlc291cmNlcyA8PCAvRm9udCA8PCAvRjEgNSAwIFIgPj4g\
Pj4gPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAxNDIgPj4Kc3RyZWFtCkJUIC9GMSAyMCBU\
ZiA3MiA3MjAgVGQgKE9uIGNvbXB1dGFibGUgbnVtYmVycykgVGogRVQKQlQgL0YxIDEyIFRmIDcy\
IDY5MCBUZCAoYiBOb25lIFAwLCBSIGMgICBjIE5vbmUgUiBlICAgZSBOb25lIFAxLCBSIGYgICBm\
IE5vbmUgUiBiKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL1R5cGUgL0ZvbnQg\
L1N1YnR5cGUgL1R5cGUxIC9CYXNlRm9udCAvSGVsdmV0aWNhID4+CmVuZG9iagp4cmVmCjAgNgow\
MDAwMDAwMDAwIDY1NTM1IGYgCjAwMDAwMDAwMDkgMDAwMDAgbiAKMDAwMDAwMDA1OCAwMDAwMCBu\
IAowMDAwMDAwMTE1IDAwMDAwIG4gCjAwMDAwMDAyNDEgMDAwMDAgbiAKMDAwMDAwMDQzMyAwMDAw\
MCBuIAp0cmFpbGVyCjw8IC9TaXplIDYgL1Jvb3QgMSAwIFIgPj4Kc3RhcnR4cmVmCjUwMwolJUVP\
Rgo=";

const PLAIN: &str = "Content-Type: text/plain; charset=utf-8\r\n";

fn inbox() -> Vec<(i64, Vec<u8>, Flags)> {
    let unread = Flags::default();
    let read = Flags {
        seen: true,
        ..Flags::default()
    };
    vec![
        {
            let (h, r) = letter(
                1,
                "Ada Lovelace <ada@analytical.example>",
                "You <you@example.com>",
                "Notes on the engine, and a table of Bernoulli numbers",
                PLAIN,
                "Hello,\n\nI have finished the notes on the engine. Note G sets out how it \
                 would compute the Bernoulli numbers, step by step, with a table of the \
                 operations and the variables each one changes.\n\nCould you read it before \
                 Thursday? The translation itself is done; the notes run to three times its \
                 length. The draft is at \
                 https://example.org/papers/lovelace/notes-on-the-analytical-engine-1843.pdf \
                 if you would rather read it there.\n\nAda\n",
            );
            (h, r, unread)
        },
        {
            let (h, r) = letter(
                3,
                "Grace Hopper <grace@navy.example>",
                "You <you@example.com>",
                "Re: the compiler",
                PLAIN,
                "That is the idea: let them write in words, and have the machine \
                 translate.\n\nIt is much easier to apologise than to get permission.\n\n\
                 Grace\n\nOn Monday, you wrote:\n> Will anyone trust a program that writes \
                 programs?\n> Most think a computer can only do arithmetic.\n",
            );
            (h, r, unread)
        },
        {
            let (h, r) = letter(
                26,
                "=?utf-8?q?Emmy_N=C3=B6ther?= <noether@goettingen.example>",
                "You <you@example.com>",
                "=?utf-8?q?Symmetry_and_conservation_=E2=80=94_the_draft?=",
                "Content-Type: multipart/alternative; boundary=\"b1\"\r\n",
                "--b1\nContent-Type: text/plain; charset=utf-8\n\nThe theorem, in a \
                 sentence: every differentiable symmetry of the action has a conserved \
                 quantity.\n\nTime gives energy; space, momentum; rotation, angular \
                 momentum.\n\n--b1\nContent-Type: text/html; charset=utf-8\n\n<p>The \
                 theorem, in a sentence: <b>every differentiable symmetry of the action \
                 has a conserved quantity</b>.</p><p>Time gives energy; space, momentum; \
                 rotation, angular momentum.</p>\n--b1--\n",
            );
            (h, r, read)
        },
        {
            let (h, r) = letter(
                50,
                "Alan Turing <alan@bletchley.example>",
                "You <you@example.com>",
                "Computable numbers: the paper, and the figures",
                "Content-Type: multipart/mixed; boundary=\"m1\"\r\n",
                &format!(
                    "--m1\nContent-Type: text/plain; charset=utf-8\n\nThe paper is attached, \
                 with the table of the machine's states. Press f to read it in ohp, or a to \
                 save them to your Downloads.\n\nAlan\n\n--m1\nContent-Type: text/plain; name=\"states.txt\"\n\
                 Content-Disposition: attachment; filename=\"states.txt\"\n\n\
                 b  None  P0, R   c\nc  None  R       e\ne  None  P1, R   f\n\
                 f  None  R       b\n--m1\nContent-Type: application/pdf; \
                 name=\"computable-numbers.pdf\"\nContent-Disposition: attachment; \
                 filename=\"computable-numbers.pdf\"\nContent-Transfer-Encoding: base64\n\n\
                 {PAPER}\n--m1--\n"
                ),
            );
            (
                h,
                r,
                Flags {
                    seen: true,
                    flagged: true,
                    ..Flags::default()
                },
            )
        },
        {
            let (h, r) = letter(
                120,
                "Rosalind Franklin <rf@kings.example>",
                "You <you@example.com>",
                "Photo 51",
                PLAIN,
                "The B form, at last: the cross is unmistakable. The spacing gives the \
                 helix's pitch, 34 Å, and its turns ten bases each.\n\nNot to be shown \
                 around yet.\n\nRosalind\n",
            );
            (
                h,
                r,
                Flags {
                    seen: true,
                    answered: true,
                    ..Flags::default()
                },
            )
        },
        {
            let (h, r) = letter(
                400,
                "Claude Shannon <shannon@bell.example>",
                "You <you@example.com>",
                "A mathematical theory of communication",
                PLAIN,
                "The fundamental problem of communication is that of reproducing at one \
                 point either exactly or approximately a message selected at another \
                 point.\n\n-- \nClaude\n",
            );
            (h, r, read)
        },
    ]
}

impl Demo {
    pub fn new() -> Self {
        let mut folders: BTreeMap<String, Vec<Letter>> = BTreeMap::new();
        let mut uid = 0;
        let mut file = |folder: &str, mut letters: Vec<(i64, Vec<u8>, Flags)>| {
            letters.sort_by_key(|l| std::cmp::Reverse(l.0));
            let list = folders.entry(folder.to_string()).or_default();
            for (_, raw, flags) in letters {
                uid += 1;
                list.push(Letter { uid, flags, raw });
            }
        };
        file("INBOX", inbox());
        let read = Flags {
            seen: true,
            ..Flags::default()
        };
        let sent = letter(
            30,
            "You <you@example.com>",
            "Rosalind Franklin <rf@kings.example>",
            "Re: Photo 51",
            PLAIN,
            "Extraordinary. I will not show it to anyone.\n",
        );
        file("Sent", vec![(sent.0, sent.1, read)]);
        let draft = letter(
            2,
            "You <you@example.com>",
            "Ada Lovelace <ada@analytical.example>",
            "Re: Notes on the engine",
            PLAIN,
            "Ada, I read Note G twice: \n",
        );
        file("Drafts", vec![(draft.0, draft.1, read)]);
        for f in ["Archive", "Junk", "Trash", "Projects", "Projects/Grant"] {
            file(f, Vec::new());
        }
        Demo {
            next: uid + 1,
            folders,
        }
    }

    fn letters(&mut self, folder: &str) -> anyhow::Result<&mut Vec<Letter>> {
        self.folders
            .get_mut(folder)
            .ok_or_else(|| anyhow::anyhow!("no folder {folder}"))
    }
}

fn summary(l: &Letter) -> Summary {
    mail::summary(l.uid, l.flags, l.raw.len() as u32, &l.raw)
}

impl Store for Demo {
    fn folders(&mut self) -> anyhow::Result<Vec<Folder>> {
        let mut folders: Vec<Folder> = self
            .folders
            .keys()
            .map(|f| Folder::new(f, Some("/"), None, true))
            .collect();
        mail::arrange(&mut folders);
        debug_assert!(folders[0].role == Role::Inbox);
        Ok(folders)
    }

    fn counts(&mut self, folder: &str) -> anyhow::Result<(u32, u32)> {
        let letters = self.letters(folder)?;
        let unseen = letters.iter().filter(|l| !l.flags.seen).count();
        Ok((letters.len() as u32, unseen as u32))
    }

    fn page(&mut self, folder: &str, offset: usize, n: usize) -> anyhow::Result<Page> {
        let letters = self.letters(folder)?;
        Ok(Page {
            summaries: letters
                .iter()
                .rev()
                .skip(offset)
                .take(n)
                .map(summary)
                .collect(),
            total: letters.len() as u32,
        })
    }

    fn raw(&mut self, folder: &str, uid: u32) -> anyhow::Result<Vec<u8>> {
        self.letters(folder)?
            .iter()
            .find(|l| l.uid == uid)
            .map(|l| l.raw.clone())
            .ok_or_else(|| anyhow::anyhow!("no message {uid} in {folder}"))
    }

    fn set(&mut self, folder: &str, uid: u32, flag: Flag, on: bool) -> anyhow::Result<()> {
        let letter = self
            .letters(folder)?
            .iter_mut()
            .find(|l| l.uid == uid)
            .ok_or_else(|| anyhow::anyhow!("no message {uid} in {folder}"))?;
        match flag {
            Flag::Seen => letter.flags.seen = on,
            Flag::Flagged => letter.flags.flagged = on,
            Flag::Answered => letter.flags.answered = on,
        }
        Ok(())
    }

    fn relocate(
        &mut self,
        from: &str,
        uid: u32,
        to: &str,
        _message_id: &str,
    ) -> anyhow::Result<u32> {
        if !self.folders.contains_key(to) {
            anyhow::bail!("no folder {to}");
        }
        let letter = {
            let letters = self.letters(from)?;
            let at = letters
                .iter()
                .position(|l| l.uid == uid)
                .ok_or_else(|| anyhow::anyhow!("no message {uid} in {from}"))?;
            letters.remove(at)
        };
        let uid = self.next;
        self.next += 1;
        self.folders
            .get_mut(to)
            .expect("checked")
            .push(Letter { uid, ..letter });
        Ok(uid)
    }

    fn expunge(&mut self, folder: &str, uid: u32) -> anyhow::Result<Removal> {
        let letters = self.letters(folder)?;
        let n = letters.len();
        letters.retain(|l| l.uid != uid);
        Ok(if letters.len() == n {
            Removal::Missing
        } else {
            Removal::Gone
        })
    }

    fn append(&mut self, folder: &str, raw: &[u8], seen: bool) -> anyhow::Result<()> {
        let uid = self.next;
        self.next += 1;
        let mut flags = Flags::default();
        flags.seen = seen;
        self.letters(folder)?.push(Letter {
            uid,
            flags,
            raw: raw.to_vec(),
        });
        Ok(())
    }

    fn create(&mut self, path: &str) -> anyhow::Result<()> {
        self.folders.entry(path.to_string()).or_default();
        Ok(())
    }

    fn address(&self) -> &str {
        "you@example.com"
    }

    fn send(&mut self, raw: &[u8]) -> anyhow::Result<String> {
        self.append("Sent", raw, true)?;
        Ok("kept in Sent; not delivered".to_string())
    }

    fn search(&mut self, folder: &str, words: &str, n: usize) -> anyhow::Result<Vec<Summary>> {
        let words: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
        Ok(self
            .letters(folder)?
            .iter()
            .rev()
            .filter(|l| {
                let text = String::from_utf8_lossy(&l.raw).to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .take(n)
            .map(summary)
            .collect())
    }
}
