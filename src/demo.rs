//! `letterbox --demo`: an account of made-up mail, in memory, dated from
//! now, to try letterbox with before setting up an account. Six letters
//! from the history of computing, and a lab's month beside them: a grant
//! being written, a paper under review, students, robots with their
//! `List-Id`, talks, travel and home; what is read and old is in Archive,
//! and the older grant letters in Projects/Grant, filed by hand.

use crate::index::labels;
use crate::index::record::Indexed;
use crate::mail::{self, Flags, Folder, Role, Summary};
use crate::store::{Change, Filed, Flag, Kind, Page, Status, Store};
use jiff::{ToSpan, Zoned};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

struct Letter {
    uid: u32,
    flags: Flags,
    raw: Vec<u8>,
    /// Its keywords: the demo keeps labels as a server of keywords does.
    keywords: Vec<String>,
    /// When it last changed, as CONDSTORE counts.
    modseq: u64,
}

impl Letter {
    fn new(uid: u32, flags: Flags, raw: Vec<u8>, modseq: u64) -> Letter {
        Letter {
            uid,
            flags,
            raw,
            keywords: Vec::new(),
            modseq,
        }
    }
}

/// The demo's mail, as a server holds it.
struct Mail {
    /// Each folder's messages, oldest first, as a server numbers them.
    folders: BTreeMap<String, Vec<Letter>>,
    /// The next UID, across every folder: a move gives the message a new one.
    next: u32,
    /// The mod-sequence of the last change.
    modseq: u64,
    /// The subjects of the messages it will not move or label, as a server
    /// that says no: for tests.
    refused: Vec<String>,
}

impl Mail {
    /// Whether the message `uid` of `folder` is one it says no to moving.
    fn refuses(&mut self, folder: &str, uid: u32) -> anyhow::Result<()> {
        if self.refused.is_empty() {
            return Ok(());
        }
        let refused = self.refused.clone();
        let letter = self.changing(folder, uid)?;
        if refused.contains(&summary(letter).subject) {
            anyhow::bail!("the server said no");
        }
        Ok(())
    }

    fn letters(&mut self, folder: &str) -> anyhow::Result<&mut Vec<Letter>> {
        self.folders
            .get_mut(folder)
            .ok_or_else(|| anyhow::anyhow!("no folder {folder}"))
    }

    /// The message `uid` of `folder`, changed now.
    fn changing(&mut self, folder: &str, uid: u32) -> anyhow::Result<&mut Letter> {
        self.modseq += 1;
        let modseq = self.modseq;
        let letter = self
            .letters(folder)?
            .iter_mut()
            .find(|l| l.uid == uid)
            .ok_or_else(|| anyhow::anyhow!("no message {uid} in {folder}"))?;
        letter.modseq = modseq;
        Ok(letter)
    }

    /// A new UID, and the mod-sequence of a message given it now.
    fn fresh(&mut self) -> (u32, u64) {
        let uid = self.next;
        self.next += 1;
        self.modseq += 1;
        (uid, self.modseq)
    }
}

/// The demo's account. Its twin, for the map, holds the same mail.
pub struct Demo {
    mail: Arc<Mutex<Mail>>,
    /// What it is known by in the index.
    ident: String,
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
    let id = format!("<{hours}.{}@demo.letterbox>", subject.len());
    dated(hours * 60, from, to, subject, &id, head, body)
}

/// A message, `minutes` ago, with the `Message-ID` `id`; its place in a
/// folder is by how long ago it came, in minutes.
fn dated(
    minutes: i64,
    from: &str,
    to: &str,
    subject: &str,
    id: &str,
    head: &str,
    body: &str,
) -> (i64, Vec<u8>) {
    let when = Zoned::now()
        .checked_sub(minutes.minutes())
        .unwrap_or_else(|_| Zoned::now());
    let date = when.strftime("%a, %d %b %Y %H:%M:%S %z");
    let raw = format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {date}\r\n\
         Message-ID: {id}\r\nMIME-Version: 1.0\r\n{head}\r\n{body}"
    );
    (
        minutes,
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

/// A message as a folder is filled with: how many minutes ago it came, its
/// bytes, and its flags.
type Kept = (i64, Vec<u8>, Flags);

const PLAIN: &str = "Content-Type: text/plain; charset=utf-8\r\n";

/// A Google Scholar alert, as Scholar writes one: each paper's title a link
/// sent on through Scholar, who wrote it where under it, then its snippet.
const SCHOLAR_ALERT: &str = "<html><body>\
<h3><a href=\"https://scholar.google.com/scholar_url?url=https://www.nature.com/articles/s41586-026-11078-2&amp;hl=en&amp;oi=scholaralrt\" class=\"gse_alrt_title\">Population-scale immune <b>multiome</b> atlas reveals regulatory disease mechanisms</a></h3>\
<div style=\"color:#006621\">M Kanai, TM Delorey, J Honkanen - Nature, 2026</div>\
<div class=\"gse_alrt_sni\">An atlas of immune cells across a population, read by genotype&hellip;</div>\
<h3><a href=\"https://scholar.google.com/scholar_url?url=https://arxiv.org/abs/2410.01234&amp;hl=en&amp;oi=scholaralrt\" class=\"gse_alrt_title\">Learning gene programs from single-cell data with transformers</a></h3>\
<div style=\"color:#006621\">A Author, B Author - arXiv preprint, 2026</div>\
<div class=\"gse_alrt_sni\">Gene programs, learnt across cells&hellip;</div>\
<p><a href=\"https://scholar.google.com/scholar_alerts?view_op=cancel_alert_options\">Cancel alert</a></p>\
</body></html>\n";

fn inbox() -> Vec<Kept> {
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

/// The people and programs the lab's mail is from: a key, a name, an
/// address, and whether a person writes it, who signs it.
#[rustfmt::skip]
const PEOPLE: &[(&str, &str, &str, bool)] = &[
    ("mira", "Mira Okafor", "mira.okafor@uni.example", true),
    ("tomas", "Tomás Reyes", "treyes@med.example", true),
    ("hana", "Hana Sato", "hsato@inst.example", true),
    ("jun", "Jun Lee", "jun.lee@uni.example", true),
    ("priya", "Priya Nair", "pnair@uni.example", true),
    ("sam", "Sam Whitfield", "swhitfield@uni.example", true),
    ("eo", "Editorial Office", "eo@journal.example", false),
    ("brandt", "Lena Brandt", "l.brandt@journal.example", true),
    ("gh", "GitHub", "notifications@github.example", false),
    ("ci", "CI", "ci@runner.example", false),
    ("ro", "Research Office", "grants@uni.example", false),
    ("grad", "Graduate Office", "grad@uni.example", false),
    ("sem", "Dept Seminars", "seminars@uni.example", false),
    ("buy", "Purchasing", "purchasing@uni.example", false),
    ("travel", "Travel Desk", "travel@uni.example", false),
    ("conf", "Conference Office", "abstracts@meeting.example", false),
    ("shop", "Bookshop", "orders@books.example", false),
    ("hotel", "Harbour Hotel", "stay@hotel.example", false),
    ("ava", "Ava Chen", "ava.chen@uni.example", true),
    ("leo", "Leo Martins", "leo.m@uni.example", true),
    ("hyun", "Hyun Park", "hyun@home.example", true),
    ("dana", "Dana Wells", "dana.wells@mail.example", true),
];

/// A letter of the lab's: who from, its subject, its text, how many days
/// ago, whether it is unread and flagged, and who else it went to.
struct Lab {
    from: &'static str,
    subject: &'static str,
    body: &'static str,
    days: f64,
    unread: bool,
    flagged: bool,
    cc: &'static [&'static str],
}

const fn lab(
    from: &'static str,
    subject: &'static str,
    body: &'static str,
    days: f64,
    (unread, flagged): (bool, bool),
    cc: &'static [&'static str],
) -> Lab {
    Lab {
        from,
        subject,
        body,
        days,
        unread,
        flagged,
        cc,
    }
}

const NEW: (bool, bool) = (true, false);
const READ: (bool, bool) = (false, false);
const NEW_FLAGGED: (bool, bool) = (true, true);
const FLAGGED: (bool, bool) = (false, true);

/// The mail of a lab's month: a grant being written, a paper under review,
/// the lab's own, students', robots', talks, travel and home.
#[rustfmt::skip]
const LAB: &[Lab] = &[
    lab("ro", "R01 resubmission: internal deadline 3 November", "The research office needs the final budget and the routing form a week before the sponsor deadline.", 0.2, NEW_FLAGGED, &[]),
    lab("mira", "Aims page, third pass", "I cut Aim 3 to two sentences and moved the spatial work into Aim 2. Have a look before Monday?", 0.4, NEW, &["hana", "tomas"]),
    lab("mira", "Budget justification: the sequencing line", "The sequencing line is 40% of the direct costs; we should say why in one paragraph.", 1.3, READ, &[]),
    lab("ro", "Subaward paperwork for the second site", "The subaward needs a statement of work and a budget from the second site.", 3.2, READ, &[]),
    lab("tomas", "Letter of support, signed", "Signed and attached. Tell me if the wording on shared data needs to change.", 4.1, READ, &["mira"]),
    lab("hana", "Re: Aim 2 and the spatial data", "The pilot slides from June are enough for preliminary data, I think.", 5.0, READ, &["mira"]),
    lab("ro", "Annual progress report due 15 November", "Your progress report for the current award is due in three weeks.", 9.0, READ, &[]),
    lab("mira", "Biosketch format changed again", "The new biosketch form drops the limit on the personal statement. Here is mine as an example.", 12.0, READ, &[]),
    lab("buy", "Budget line 4 is overspent", "Line 4 (supplies) is over by 2,310. A transfer from line 6 would cover it.", 6.0, NEW, &[]),
    lab("eo", "Decision on your manuscript: major revision", "The reviewers find the method promising but ask for a comparison on a second data set.", 2.2, FLAGGED, &[]),
    lab("brandt", "Invitation to review a manuscript", "Would you review a manuscript on cell-type deconvolution in spatial data? Reports are due in 21 days.", 1.1, NEW, &[]),
    lab("eo", "Reminder: your report is due in 3 days", "Your report on manuscript GB-2026-118 is due on 27 October.", 0.6, NEW, &[]),
    lab("hana", "Response to reviewers, draft 2", "I took reviewer 2's points one by one; the batch-effect one needs a new figure.", 3.0, READ, &["jun", "tomas"]),
    lab("jun", "Revised Figure 3 for the rebuttal", "Panels b and c redone with the second data set, in the same colours as before.", 2.9, READ, &["hana"]),
    lab("eo", "Your revision has been received", "Thank you for your revised manuscript, which has gone back to the reviewers.", 40.0, READ, &[]),
    lab("tomas", "Re: reviewer 2 on batch effects", "We could answer with the permutation test rather than a new experiment.", 3.5, READ, &["hana", "jun"]),
    lab("jun", "Lab meeting moved to Thursday, 2 pm", "The seminar room is taken on Wednesday, so lab meeting is Thursday this week.", 0.3, NEW, &["priya", "sam"]),
    lab("priya", "Cluster quota at 94%", "The project space on the cluster is at 94%. I can move the old BAMs to tape.", 1.2, NEW_FLAGGED, &["sam"]),
    lab("sam", "The new 10x run is on the server", "Eight samples, all passed QC, in /data/runs/2026-10-21.", 2.0, READ, &["priya", "jun"]),
    lab("priya", "Re: ATAC peaks look odd on chr7", "It was the blacklist file: the old one was for hg19.", 4.0, READ, &["sam"]),
    lab("jun", "Pizza on Thursday?", "Same as last time, plus one without cheese.", 0.9, READ, &["priya", "sam"]),
    lab("sam", "The -80 freezer alarm: all fine now", "The door was not shut after the Friday delivery. Nothing thawed.", 6.0, READ, &["priya", "jun"]),
    lab("priya", "A notebook for the QC thresholds", "The thresholds we agreed on, with plots, so the next run uses the same ones.", 8.0, READ, &["sam"]),
    lab("jun", "Journal club: who presents next", "I have the next two weeks; we need someone for 6 November.", 10.0, READ, &["priya", "sam"]),
    lab("sam", "Re: storage for the spatial slides", "The slides fit on the shared drive if we drop the raw TIFFs.", 33.0, READ, &["priya"]),
    lab("ava", "Thesis chapter 2, for your comments", "The methods are complete; the discussion is still thin.", 1.6, NEW, &[]),
    lab("leo", "Committee meeting: possible dates", "The weeks of 9 and 16 November work for the other two members.", 2.5, NEW, &[]),
    lab("ava", "Re: chapter 2, the methods section", "Fixed the notation in section 2.3 as you said.", 7.5, READ, &[]),
    lab("grad", "Committee form needs your signature", "Leo Martins's committee form is waiting for your signature in the portal.", 4.6, NEW, &[]),
    lab("leo", "Qualifying exam slides", "Here is the deck for the practice talk.", 15.0, READ, &[]),
    lab("gh", "[letterbox] Issue #41: a map of the inbox", "Could the message list be a map instead, mail laid out by what it is about?", 0.1, NEW, &[]),
    lab("gh", "[letterbox] PR #38 merged: forward with files", "Merged into main.", 1.4, READ, &[]),
    lab("ci", "Build failed: letterbox main (clippy)", "error: this `if` has identical blocks, src/ui.rs:812.", 0.7, NEW, &[]),
    lab("gh", "[senna] Issue #112: legend overlaps the map", "At small widths the legend covers the upper-right corner.", 3.3, READ, &[]),
    lab("gh", "[faba] PR #77: faster pileup", "Reads each BAM block once rather than once per site.", 5.5, READ, &[]),
    lab("ci", "Build passed: letterbox main", "All checks passed.", 0.65, READ, &[]),
    lab("gh", "[letterbox] Issue #40: Outlook token not renewed", "After 90 days the token is not renewed and signing in fails.", 8.5, READ, &[]),
    lab("gh", "[pinto] Release v0.9.0", "The link-community model now takes batches.", 20.0, READ, &[]),
    lab("ci", "Build failed: faba (test timeout)", "test pileup::large_region timed out after 60 s.", 5.4, READ, &[]),
    lab("hana", "Invitation: keynote at the spatial omics workshop", "Would you give the opening talk in April? Travel and two nights are covered.", 1.8, NEW_FLAGGED, &[]),
    lab("sem", "This week: N. Adeyemi on gene regulation", "Thursday 4 pm, Life Sciences 201. Coffee from 3:45.", 2.4, READ, &[]),
    lab("sem", "Seminar schedule, spring term", "Speakers are confirmed through March; two slots are open.", 14.0, READ, &[]),
    lab("tomas", "Would you speak in our series in March?", "A 45-minute talk, any Tuesday in March, in person or remote.", 6.5, READ, &[]),
    lab("conf", "Your abstract was accepted", "Your abstract is accepted as a talk in session 4B.", 11.0, READ, &[]),
    lab("travel", "Itinerary: Vancouver to Boston, 12 November", "Flight AC 718 at 8:05, returning on the 15th.", 2.7, READ, &[]),
    lab("buy", "PO 88213 approved", "Your purchase order for pipette tips is approved.", 7.0, READ, &[]),
    lab("travel", "Expense claim returned: missing receipt", "The claim for the September trip needs the hotel receipt.", 3.8, NEW, &[]),
    lab("shop", "Your order has shipped", "Pattern Recognition and Machine Learning, arriving Tuesday.", 4.9, READ, &[]),
    lab("hotel", "Booking confirmed: 3 nights in Boston", "Check-in 12 November, check-out 15 November.", 2.6, READ, &[]),
    lab("hyun", "Photos from Saturday", "The kids at the beach, and the one of you asleep on the ferry.", 0.5, NEW, &[]),
    lab("dana", "Long time! In Boston next month?", "Saw you are speaking in Boston. Dinner while you are in town?", 1.0, NEW, &[]),
];

/// The older grant letters, kept in Projects/Grant as someone filing by
/// hand would have.
const GRANT: &[&str] = &[
    "Subaward paperwork for the second site",
    "Letter of support, signed",
    "Annual progress report due 15 November",
    "Biosketch format changed again",
];

/// Your answers, in Sent: to whom, the subject, the text, how many days
/// ago.
#[rustfmt::skip]
const ANSWERS: &[(&str, &str, &str, f64)] = &[
    ("mira", "Re: Biosketch format changed again", "Thanks; I will follow yours for the personal statement.", 11.5),
    ("hana", "Re: Response to reviewers, draft 2", "Good. Jun can make the batch-effect figure from the second data set.", 2.8),
    ("ava", "Re: Thesis chapter 2, notation in section 2.3", "Section 2.3 mixes two notations for the same matrix; keep the bold one.", 8.0),
];

/// One of the demo's people, by their key.
fn person(key: &str) -> (&'static str, &'static str, bool) {
    PEOPLE
        .iter()
        .find(|p| p.0 == key)
        .map(|&(_, name, address, signs)| (name, address, signs))
        .expect("one of the demo's people")
}

/// Who `key` is, as a `From` or `To` has it.
fn addressed(key: &str) -> String {
    let (name, address, _) = person(key);
    format!("{name} <{address}>")
}

/// The `List-Id` a robot's mail has: a repository's, the CI's, the
/// seminars'.
fn list_id(from: &str, subject: &str) -> Option<String> {
    match from {
        "gh" => {
            let repo = subject.strip_prefix('[')?.split(']').next()?;
            Some(format!("<{repo}.YPARK.github.example>"))
        }
        "ci" => Some("<ci.runner.example>".to_string()),
        "sem" => Some("<seminars.uni.example>".to_string()),
        _ => None,
    }
}

fn minutes(days: f64) -> i64 {
    (days * 24.0 * 60.0).round() as i64
}

/// The lab's mail, by the folder each is in: the Inbox holds what is
/// unread or new, Projects/Grant the older grant letters, and Archive the
/// rest, as a mailbox kept tidy would.
fn labs() -> Vec<(&'static str, Kept)> {
    LAB.iter()
        .enumerate()
        .map(|(i, l)| {
            let mut head = String::from(PLAIN);
            if !l.cc.is_empty() {
                let cc: Vec<String> = l.cc.iter().map(|k| addressed(k)).collect();
                head.push_str(&format!("Cc: {}\r\n", cc.join(", ")));
            }
            if let Some(list) = list_id(l.from, l.subject) {
                head.push_str(&format!("List-Id: {list}\r\n"));
                head.push_str("List-Unsubscribe: <mailto:unsubscribe@lists.example>\r\n");
            }
            let (name, _, signs) = person(l.from);
            let body = if signs {
                let first = name.split(' ').next().unwrap_or(name);
                format!("{}\n\n{first}\n", l.body)
            } else {
                format!("{}\n", l.body)
            };
            let (at, raw) = dated(
                minutes(l.days),
                &addressed(l.from),
                "You <you@example.com>",
                l.subject,
                &format!("<lab.{i}@demo.letterbox>"),
                &head,
                &body,
            );
            let flags = Flags {
                seen: !l.unread,
                flagged: l.flagged,
                ..Flags::default()
            };
            let folder = if GRANT.contains(&l.subject) {
                "Projects/Grant"
            } else if l.unread || l.days <= 3.0 {
                "INBOX"
            } else {
                "Archive"
            };
            (folder, (at, raw, flags))
        })
        .collect()
}

/// Your answers to the lab, as Sent has them.
fn answers() -> Vec<Kept> {
    let read = Flags {
        seen: true,
        ..Flags::default()
    };
    ANSWERS
        .iter()
        .enumerate()
        .map(|(i, &(to, subject, body, days))| {
            let (at, raw) = dated(
                minutes(days),
                "You <you@example.com>",
                &addressed(to),
                subject,
                &format!("<sent.{i}@demo.letterbox>"),
                PLAIN,
                &format!("{body}\n"),
            );
            (at, raw, read)
        })
        .collect()
}

impl Demo {
    pub fn new() -> Self {
        let mut folders: BTreeMap<String, Vec<Letter>> = BTreeMap::new();
        let mut uid = 0;
        let mut file = |folder: &str, mut letters: Vec<Kept>| {
            letters.sort_by_key(|l| std::cmp::Reverse(l.0));
            let list = folders.entry(folder.to_string()).or_default();
            for (_, raw, flags) in letters {
                uid += 1;
                list.push(Letter::new(uid, flags, raw, 1));
            }
        };
        let mut lab: BTreeMap<&str, Vec<Kept>> = BTreeMap::new();
        for (folder, letter) in labs() {
            lab.entry(folder).or_default().push(letter);
        }
        let mut take = |folder: &str| lab.remove(folder).unwrap_or_default();
        let mut inbox = inbox();
        inbox.extend(take("INBOX"));
        file("INBOX", inbox);
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
        let mut sent = vec![(sent.0, sent.1, read)];
        sent.extend(answers());
        file("Sent", sent);
        // A Scholar alert, its papers to put in paperstack with P.
        let mut archive = take("Archive");
        let alert = letter(
            2900,
            "Google Scholar Alerts <scholaralerts-noreply@google.com>",
            "You <you@example.com>",
            "Kanai M - new related research",
            "Content-Type: text/html; charset=utf-8\r\n",
            SCHOLAR_ALERT,
        );
        archive.push((alert.0, alert.1, read));
        file("Archive", archive);
        file("Projects/Grant", take("Projects/Grant"));
        let draft = letter(
            2,
            "You <you@example.com>",
            "Ada Lovelace <ada@analytical.example>",
            "Re: Notes on the engine",
            PLAIN,
            "Ada, I read Note G twice: \n",
        );
        file("Drafts", vec![(draft.0, draft.1, read)]);
        for f in ["Junk", "Trash", "Projects"] {
            file(f, Vec::new());
        }
        Demo {
            mail: Arc::new(Mutex::new(Mail {
                next: uid + 1,
                folders,
                modseq: 1,
                refused: Vec::new(),
            })),
            ident: "demo".to_string(),
        }
    }

    /// The message with `subject` not moved nor labelled, as a server
    /// that says no: for tests.
    #[cfg(test)]
    pub fn refuse(&self, subject: &str) {
        self.mail().refused.push(subject.to_string());
    }

    /// A hand on the same mail, to see where a message is: for tests.
    #[cfg(test)]
    pub fn handle(&self) -> Demo {
        Demo {
            mail: self.mail.clone(),
            ident: self.ident.clone(),
        }
    }

    /// The demo known by `ident` in the index, as a second account of it
    /// is in tests.
    #[cfg(test)]
    pub fn called(mut self, ident: &str) -> Self {
        self.ident = ident.to_string();
        self
    }

    fn mail(&self) -> MutexGuard<'_, Mail> {
        self.mail.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn summary(l: &Letter) -> Summary {
    mail::summary(l.uid, l.flags, l.raw.len() as u32, &l.raw)
}

impl Store for Demo {
    fn folders(&mut self) -> anyhow::Result<Vec<Folder>> {
        let mut folders: Vec<Folder> = self
            .mail()
            .folders
            .keys()
            .map(|f| Folder::new(f, Some("/"), None, true))
            .collect();
        mail::arrange(&mut folders);
        debug_assert!(folders[0].role == Role::Inbox);
        Ok(folders)
    }

    fn counts(&mut self, folder: &str) -> anyhow::Result<(u32, u32)> {
        let mut mail = self.mail();
        let letters = mail.letters(folder)?;
        let unseen = letters.iter().filter(|l| !l.flags.seen).count();
        Ok((letters.len() as u32, unseen as u32))
    }

    fn page(&mut self, folder: &str, offset: usize, n: usize) -> anyhow::Result<Page> {
        let mut mail = self.mail();
        let letters = mail.letters(folder)?;
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
        self.mail()
            .letters(folder)?
            .iter()
            .find(|l| l.uid == uid)
            .map(|l| l.raw.clone())
            .ok_or_else(|| anyhow::anyhow!("no message {uid} in {folder}"))
    }

    fn set(&mut self, folder: &str, uid: u32, flag: Flag, on: bool) -> anyhow::Result<()> {
        let mut mail = self.mail();
        let letter = mail.changing(folder, uid)?;
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
        let mut mail = self.mail();
        if !mail.folders.contains_key(to) {
            anyhow::bail!("no folder {to}");
        }
        mail.refuses(from, uid)?;
        let letter = {
            let letters = mail.letters(from)?;
            let at = letters
                .iter()
                .position(|l| l.uid == uid)
                .ok_or_else(|| anyhow::anyhow!("no message {uid} in {from}"))?;
            letters.remove(at)
        };
        let (uid, modseq) = mail.fresh();
        mail.folders.get_mut(to).expect("checked").push(Letter {
            uid,
            modseq,
            ..letter
        });
        Ok(uid)
    }

    fn append(&mut self, folder: &str, raw: &[u8], seen: bool) -> anyhow::Result<()> {
        let mut mail = self.mail();
        let subject = crate::mail::field(raw, "Subject").unwrap_or_default();
        if mail.refused.contains(&subject) {
            anyhow::bail!("the server said no");
        }
        mail.letters(folder)?;
        let (uid, modseq) = mail.fresh();
        let flags = Flags {
            seen,
            ..Flags::default()
        };
        mail.letters(folder)?
            .push(Letter::new(uid, flags, raw.to_vec(), modseq));
        Ok(())
    }

    fn holds(&mut self, folder: &str, message_id: &str) -> anyhow::Result<bool> {
        let bare = |s: &str| s.trim().trim_matches(['<', '>']).to_lowercase();
        let id = bare(message_id);
        let mut mail = self.mail();
        Ok(mail
            .letters(folder)?
            .iter()
            .any(|l| crate::mail::field(&l.raw, "Message-ID").is_some_and(|m| bare(&m) == id)))
    }

    fn create(&mut self, path: &str) -> anyhow::Result<()> {
        self.mail().folders.entry(path.to_string()).or_default();
        Ok(())
    }

    fn delete_folder(&mut self, path: &str) -> anyhow::Result<()> {
        let mut mail = self.mail();
        let Some(letters) = mail.folders.get(path) else {
            anyhow::bail!("no folder {path}");
        };
        if !letters.is_empty() {
            anyhow::bail!("{path} is not empty: {} messages", letters.len());
        }
        let under = format!("{path}/");
        if mail.folders.keys().any(|f| f.starts_with(&under)) {
            anyhow::bail!("{path} has a folder under it");
        }
        mail.folders.remove(path);
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
            .mail()
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

    fn twin(&self) -> Option<Box<dyn Store>> {
        Some(Box::new(Demo {
            mail: self.mail.clone(),
            ident: self.ident.clone(),
        }))
    }

    fn ident(&self) -> String {
        self.ident.clone()
    }

    fn kind(&mut self) -> anyhow::Result<Kind> {
        Ok(Kind::Keywords)
    }

    fn status(&mut self, folder: &str) -> anyhow::Result<Status> {
        let mut mail = self.mail();
        let next = mail.next;
        let letters = mail.letters(folder)?;
        Ok(Status {
            messages: letters.len() as u32,
            validity: 1,
            next,
            modseq: Some(letters.iter().map(|l| l.modseq).max().unwrap_or(1)),
        })
    }

    fn fetch_index(&mut self, folder: &str, uids: &[u32]) -> anyhow::Result<Vec<Indexed>> {
        Ok(self
            .mail()
            .letters(folder)?
            .iter()
            .filter(|l| uids.contains(&l.uid))
            .map(|l| Indexed {
                uid: l.uid,
                flags: l.flags,
                labels: l.keywords.clone(),
                modseq: Some(l.modseq),
                size: l.raw.len() as u32,
                raw: l.raw.clone(),
                ..Indexed::default()
            })
            .collect())
    }

    fn changed(
        &mut self,
        folder: &str,
        since: Option<u64>,
        uids: &[u32],
    ) -> anyhow::Result<Vec<Change>> {
        Ok(self
            .mail()
            .letters(folder)?
            .iter()
            .filter(|l| match since {
                Some(m) => l.modseq > m,
                None => uids.contains(&l.uid),
            })
            .map(|l| Change {
                uid: l.uid,
                flags: l.flags,
                labels: l.keywords.clone(),
                modseq: Some(l.modseq),
            })
            .collect())
    }

    fn uids(&mut self, folder: &str) -> anyhow::Result<Vec<u32>> {
        Ok(self.mail().letters(folder)?.iter().map(|l| l.uid).collect())
    }

    fn label(&mut self, folder: &str, uid: u32, label: &str, on: bool) -> anyhow::Result<()> {
        let keyword = labels::keyword(label);
        let mut mail = self.mail();
        mail.refuses(folder, uid)?;
        let letter = mail.changing(folder, uid)?;
        letter
            .keywords
            .retain(|k| !k.eq_ignore_ascii_case(&keyword));
        if on {
            letter.keywords.push(keyword);
        }
        Ok(())
    }

    fn file(
        &mut self,
        folder: &str,
        uid: u32,
        label: &str,
        message_id: &str,
    ) -> anyhow::Result<Filed> {
        self.label(folder, uid, label, true)?;
        if !folder.eq_ignore_ascii_case("INBOX") {
            return Ok(Filed {
                moved: None,
                on_server: true,
            });
        }
        let new = self.relocate(folder, uid, "Archive", message_id)?;
        Ok(Filed {
            moved: Some(("Archive".to_string(), new)),
            on_server: true,
        })
    }

    fn rename_label(&mut self, old: &str, new: &str) -> anyhow::Result<()> {
        let (from, to) = (labels::keyword(old), labels::keyword(new));
        let mut mail = self.mail();
        mail.modseq += 1;
        let modseq = mail.modseq;
        let mut done = false;
        for letter in mail.folders.values_mut().flatten() {
            if letter
                .keywords
                .iter()
                .any(|k| k.eq_ignore_ascii_case(&from))
            {
                letter.keywords.retain(|k| !k.eq_ignore_ascii_case(&from));
                letter.keywords.push(to.clone());
                letter.modseq = modseq;
                done = true;
            }
        }
        // A folder the label is, renamed where it is, under its parent.
        let folders: Vec<String> = mail.folders.keys().cloned().collect();
        for path in folders {
            let (parent, name) = match path.rsplit_once('/') {
                Some((p, n)) => (Some(p), n),
                None => (None, path.as_str()),
            };
            if name.eq_ignore_ascii_case(old) {
                let to = match parent {
                    Some(p) => format!("{p}/{new}"),
                    None => new.to_string(),
                };
                let letters = mail.folders.remove(&path).unwrap_or_default();
                mail.folders.insert(to, letters);
                done = true;
            }
        }
        if !done {
            anyhow::bail!("there is no label {old}");
        }
        Ok(())
    }
}
