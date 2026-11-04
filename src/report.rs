//! What an AI agent reports of new mail, for letterbox to show as it next
//! can: what came, in a few lines; what to ask the agent next; and what it
//! would throw away or file, each done only as you choose it in letterbox.
//! The agent writes it with `report_triage`, over MCP, to the journal
//! `report` beside the map, with the map's public key alone; letterbox,
//! open, takes it in, and it is gone from there once read.

use serde::{Deserialize, Serialize};

/// The journal a report is written to.
pub const JOURNAL: &str = "report";
/// The most proposals and questions a report holds.
pub const PROPOSED: usize = 50;
pub const ASKED: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// When it was written, in seconds since 1970.
    pub at: i64,
    /// What came, as the agent sums it up.
    pub summary: String,
    /// What it would do with each of a few messages.
    #[serde(default)]
    pub proposals: Vec<Proposal>,
    /// What you might ask it next.
    #[serde(default)]
    pub questions: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    #[default]
    Trash,
    File,
}

/// A message, as letterbox read it, and what the agent would do with it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    /// The account, as the index knows it.
    pub account: String,
    pub folder: String,
    pub uid: u32,
    #[serde(default)]
    pub message_id: String,
    /// Who sent it and what about, as letterbox read it: never as the
    /// agent says.
    pub from: String,
    pub subject: String,
    pub action: Action,
    /// The label it would be filed under.
    #[serde(default)]
    pub to: String,
    /// Why, as the agent says.
    #[serde(default)]
    pub why: String,
    /// Chosen to be done, in letterbox.
    #[serde(skip)]
    pub chosen: bool,
}
