//! The papers a message tells of, each put in paperstack.
use super::*;

/// The papers of the message read, as listed: where each stands with
/// paperstack, once it has said; the one picked; what is under way.
#[derive(Clone, Debug)]
pub struct PapersView {
    /// Which asking of paperstack it is: what it says of another is not
    /// its.
    pub asked: u64,
    pub papers: Vec<crate::papers::Paper>,
    pub standing: Vec<Option<crate::paperstack::Standing>>,
    pub at: usize,
    pub note: Option<String>,
}

/// Do `f` with paperstack, run first if it is not yet: what it gives.
pub(super) fn with_library<T>(
    lib: &Mutex<Option<Box<dyn crate::paperstack::Library>>>,
    f: impl FnOnce(&mut dyn crate::paperstack::Library) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut held = lib.lock().unwrap_or_else(|e| e.into_inner());
    if held.is_none() {
        *held = Some(crate::paperstack::connect()?);
    }
    let done = f(held.as_mut().expect("run").as_mut());
    // What paperstack said no to, it said; anything else, as one gone or
    // that answered what is not its, is run again next time.
    if done
        .as_ref()
        .is_err_and(|e| !e.to_string().starts_with("paperstack: "))
    {
        *held = None;
    }
    done
}

impl App {
    /// List the papers the message read tells of, and ask paperstack where
    /// each stands.
    pub(super) fn list_papers(&mut self) {
        let Some(m) = self.message().cloned() else {
            self.status = "nothing to look in: ⏎ reads the message picked".to_string();
            return;
        };
        let from = m
            .from
            .rsplit_once('<')
            .map_or(m.from.as_str(), |(_, a)| a.trim_end_matches('>'));
        let papers = crate::papers::in_message(from, m.html.as_deref(), &m.text);
        if papers.is_empty() {
            self.status = "no papers found in it".to_string();
            return;
        }
        self.papers_asked += 1;
        let asked = self.papers_asked;
        self.dialog = Dialog::Papers(Box::new(PapersView {
            asked,
            standing: vec![None; papers.len()],
            papers: papers.clone(),
            at: 0,
            note: Some("asking paperstack…".to_string()),
        }));
        let (lib, replies) = (self.library.clone(), self.replies_to.clone());
        self.pending += 1;
        std::thread::spawn(move || {
            let said = with_library(&lib, |l| {
                papers
                    .iter()
                    .map(|p| crate::paperstack::standing(l, p))
                    .collect()
            });
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Standings {
                    asked,
                    said: said.map_err(worker::text),
                },
            });
        });
    }

    /// A key among the papers listed: ⏎ puts the one picked in paperstack,
    /// starred; o opens it.
    pub(super) fn papers_key(&mut self, key: KeyEvent) {
        let Dialog::Papers(p) = &mut self.dialog else {
            return;
        };
        let n = p.papers.len();
        match (keymap::act(key), key.code) {
            (Some(Act::Down), _) => p.at = (p.at + 1).min(n.saturating_sub(1)),
            (Some(Act::Up), _) => p.at = p.at.saturating_sub(1),
            (Some(Act::Open), _) => self.shelve(),
            (_, KeyCode::Char('o')) => {
                if let Some(link) = p.papers.get(p.at).and_then(|x| x.link.clone()) {
                    self.launch = Some(Launch::Browse(link));
                }
            }
            _ => self.dialog = Dialog::Closed,
        }
    }

    /// Put the paper picked in paperstack, starred, as you liked it; or,
    /// there already, star it.
    pub(super) fn shelve(&mut self) {
        let Dialog::Papers(p) = &mut self.dialog else {
            return;
        };
        let Some(paper) = p.papers.get(p.at).cloned() else {
            return;
        };
        let have = match p.standing.get(p.at).and_then(Option::as_ref) {
            Some(crate::paperstack::Standing::Have { starred: true, .. }) => {
                p.note = Some("in your library, starred already".to_string());
                return;
            }
            Some(crate::paperstack::Standing::Have { key, .. }) => Some(key.clone()),
            _ => None,
        };
        p.note = Some(format!("putting it in paperstack: {}", paper.what));
        let asked = p.asked;
        let (lib, replies) = (self.library.clone(), self.replies_to.clone());
        self.pending += 1;
        std::thread::spawn(move || {
            let found = with_library(&lib, |l| {
                let f = match have {
                    Some(key) => crate::paperstack::Found {
                        key,
                        title: paper.title.clone(),
                        starred: true,
                    },
                    None => l.add(&paper.what)?,
                };
                l.star(&f.key)?;
                Ok(f)
            });
            let _ = replies.send(Reply {
                account: usize::MAX,
                answer: Answer::Shelved {
                    asked,
                    found: found.map_err(worker::text),
                },
            });
        });
    }
}
