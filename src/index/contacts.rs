//! Who the user has written to and heard from, as the index knows it: worked
//! out from the mail each time, so it cannot drift from it.

use super::Index;
use super::labels::{self, System};
use super::record::Record;
use crate::filter::takes_no_answer;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Someone written to or heard from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Contact {
    pub address: String,
    /// As they sign their mail; empty if they never wrote.
    pub name: String,
    /// How many messages went to them, and how many came from them.
    pub sent: u32,
    pub received: u32,
    /// Seconds since 1970 of the last of them.
    pub last: Option<i64>,
}

impl Contact {
    /// As it goes on a letter: a name with anything a header would read
    /// otherwise, as a comma, quoted.
    pub fn addressed(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            return self.address.clone();
        }
        let plain = name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '\'' | '_'));
        if plain {
            format!("{name} <{}>", self.address)
        } else {
            let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{quoted}\" <{}>", self.address)
        }
    }

    /// Whether all of `words`, lower case, are in its name or address; and
    /// whether one begins a word of them, for those to come first.
    fn matches(&self, words: &[String]) -> Option<bool> {
        let name = self.name.to_lowercase();
        let held = |w: &String| name.contains(w.as_str()) || self.address.contains(w.as_str());
        if !words.iter().all(held) {
            return None;
        }
        let begins = |w: &String| {
            name.split_whitespace().any(|n| n.starts_with(w.as_str()))
                || self.address.starts_with(w.as_str())
        };
        Some(words.iter().any(begins))
    }
}

impl Index {
    /// Each person of the account known as `account`, else of every one: those
    /// written to first, then the latest. Not the user, drafts, lists, bulk
    /// mail, nor any who take no answer.
    pub fn contacts(&self, account: Option<&str>) -> Vec<Contact> {
        let mine = |r: &&Record| account.is_none_or(|a| r.key.account == a);
        let records: Vec<(&Record, System)> = self
            .records
            .values()
            .filter(mine)
            .map(|r| (r, labels::system(r, &self.folders)))
            .filter(|(_, s)| !s.draft)
            .collect();
        let own: HashSet<&str> = records
            .iter()
            .filter(|(_, s)| s.sent)
            .map(|(r, _)| r.address.as_str())
            .collect();
        let mut people: HashMap<String, (Contact, i64)> = HashMap::new();
        let person = |people: &mut HashMap<String, (Contact, i64)>, address: &str| -> Option<()> {
            let address = address.to_lowercase();
            if address.is_empty() || own.contains(address.as_str()) || takes_no_answer(&address) {
                return None;
            }
            people.entry(address.clone()).or_insert_with(|| {
                let contact = Contact {
                    address,
                    name: String::new(),
                    sent: 0,
                    received: 0,
                    last: None,
                };
                (contact, i64::MIN)
            });
            Some(())
        };
        for (r, system) in &records {
            if system.sent {
                for address in r.to.iter().chain(&r.cc) {
                    if person(&mut people, address).is_some() {
                        let (c, _) = people.get_mut(&address.to_lowercase()).expect("just kept");
                        c.sent += 1;
                        c.last = c.last.max(r.date);
                    }
                }
            } else if !r.bulk && r.list.is_empty() && person(&mut people, &r.address).is_some() {
                let (c, named) = people
                    .get_mut(&r.address.to_lowercase())
                    .expect("just kept");
                c.received += 1;
                c.last = c.last.max(r.date);
                // As they last signed.
                let date = r.date.unwrap_or(i64::MIN + 1);
                if r.from != r.address && !r.from.is_empty() && date >= *named {
                    c.name = r.from.clone();
                    *named = date;
                }
            }
        }
        let mut all: Vec<Contact> = people.into_values().map(|(c, _)| c).collect();
        all.sort_by(|a, b| {
            (b.sent > 0)
                .cmp(&(a.sent > 0))
                .then(b.last.cmp(&a.last))
                .then(a.address.cmp(&b.address))
        });
        all
    }

    /// Those of `contacts` with all of `words` in their name or address, those
    /// where one begins a word first; at most `limit`.
    pub fn find_contacts(&self, words: &str, account: Option<&str>, limit: usize) -> Vec<Contact> {
        found(&self.contacts(account), words, limit)
    }
}

/// Those of `all` with all of `words` in their name or address, as
/// `Index::find_contacts` says.
pub fn found(all: &[Contact], words: &str, limit: usize) -> Vec<Contact> {
    let words: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
    let mut held: Vec<(bool, &Contact)> = all
        .iter()
        .filter_map(|c| Some((c.matches(&words)?, c)))
        .collect();
    held.sort_by_key(|(begins, _)| !*begins);
    held.into_iter()
        .take(limit)
        .map(|(_, c)| c.clone())
        .collect()
}
