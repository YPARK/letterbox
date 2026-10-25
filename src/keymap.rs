//! The keys: what each does, and what the foot and the help say of it, in
//! one table. The folders, the list and the message read take theirs from
//! it; the message being written is edit's to handle, and the table only
//! says its keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::sync::OnceLock;

/// What a key does, outside the message being written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Down,
    Up,
    Left,
    Right,
    NextPane,
    PreviousPane,
    Open,
    /// A page down, or up, in the message: space and b.
    ScreenDown,
    ScreenUp,
    /// The same, or five folders in the sidebar: the page keys.
    PageDown,
    PageUp,
    NextMessage,
    PreviousMessage,
    Top,
    End,
    Search,
    Back,
    Seen,
    Flag,
    Trash,
    Compose,
    Reply,
    ReplyAll,
    Forward,
    Browser,
    Files,
    SaveFiles,
    NewMail,
    AddAccount,
    Rename,
    Remove,
    Help,
    Quit,
    /// `r`: in the map, read mail shown or hidden; elsewhere, as it replies
    /// in other mail programs, says which keys do.
    ShowRead,
    /// The map, or the list, in the middle pane.
    MapView,
    ListView,
    PreviousCluster,
    NextCluster,
    /// The map laid out by another lens, back as far as another time, or
    /// coloured another way.
    Lens,
    Since,
    Colours,
    /// The areas with nothing new dimmed, or not.
    NewOnly,
    /// The cluster of the message picked marked, or let go.
    Mark,
    /// The message picked, or what is marked, moved onto a cluster; and, as
    /// it is, onto a new label.
    Move,
    NewLabel,
    /// The first of the label's suggestions filed, or all of them; or the
    /// first said not to be one.
    FileFirst,
    FileAll,
    NotIt,
    /// The map laid out again, to keep or drop once seen.
    Relayout,
}

/// Where the foot's hints are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Side,
    List,
    Reader,
    /// The message being written.
    Writing,
    /// Asking which folder is Trash.
    Picking,
    /// The map, in the middle pane.
    Map,
}

pub struct Binding {
    /// Each key, as `key_named` reads it, and what it does.
    pub keys: &'static [(&'static str, Act)],
    /// The keys as the help says them, each as the README does.
    pub name: &'static str,
    /// The keys as the foot says them, if it does; pressed when clicked, if
    /// one key.
    pub hint: &'static str,
    /// What the foot says it does, in each pane it is shown in. The foot
    /// keeps the table's order, and leaves out the last first to fit, but
    /// help and quit.
    pub foot: &'static [(Pane, &'static str)],
    /// What the help says it does; none, left out of the help.
    pub help: &'static str,
}

use Act::*;
use Pane::{List, Map, Picking, Reader, Side, Writing};

pub const KEYS: &[Binding] = &[
    Binding {
        keys: &[("enter", Open)],
        name: "enter",
        hint: "⏎",
        foot: &[
            (Side, "open"),
            (List, "read"),
            (Picking, "this folder"),
            (Map, "read"),
        ],
        help: "open the folder, or read the message",
    },
    Binding {
        keys: &[("j", Down), ("down", Down), ("k", Up), ("up", Up)],
        name: "j k",
        hint: "j/k",
        foot: &[(Side, "move"), (Picking, "move"), (Map, "move")],
        help: "move; in a message, scroll; in the map, to the nearest message that way, as h and l; the arrows too",
    },
    Binding {
        keys: &[
            ("h", Left),
            ("left", Left),
            ("l", Right),
            ("right", Right),
            ("tab", NextPane),
            ("S-tab", PreviousPane),
        ],
        name: "h l tab",
        hint: "",
        foot: &[],
        help: "move between folders, messages and the message read; tab and S-tab in the map",
    },
    Binding {
        keys: &[
            ("space", ScreenDown),
            ("pgdn", PageDown),
            ("b", ScreenUp),
            ("pgup", PageUp),
        ],
        name: "space b",
        hint: "␣",
        foot: &[(Reader, "page")],
        help: "a page down, or up, in the message",
    },
    Binding {
        keys: &[("n", NextMessage), ("p", PreviousMessage)],
        name: "n p",
        hint: "n/p",
        foot: &[(Reader, "next/previous")],
        help: "read the next message, or the one before",
    },
    Binding {
        keys: &[("g", Top), ("home", Top), ("G", End), ("end", End)],
        name: "g G",
        hint: "",
        foot: &[],
        help: "go to the top, or the end",
    },
    Binding {
        keys: &[("d", Trash)],
        name: "d",
        hint: "d",
        foot: &[(List, "trash"), (Reader, "trash"), (Map, "trash")],
        help: "move to Trash; in Trash, put it back. If the server marks no Trash folder, you say which one. Kept for 7 days",
    },
    Binding {
        keys: &[("c", Compose)],
        name: "c",
        hint: "c",
        foot: &[(List, "compose")],
        help: "write a message",
    },
    Binding {
        keys: &[("C-r", Reply)],
        name: "C-r",
        hint: "C-r",
        foot: &[(List, "reply"), (Reader, "reply")],
        help: "reply to the sender",
    },
    Binding {
        keys: &[("e", ReplyAll), ("C-S-r", ReplyAll)],
        name: "e C-S-r",
        hint: "e",
        foot: &[(List, "reply all"), (Reader, "reply all")],
        help: "reply to all: the sender, and the others it went to, copied",
    },
    Binding {
        keys: &[("C-f", Forward)],
        name: "C-f",
        hint: "C-f",
        foot: &[(List, "forward"), (Reader, "forward")],
        help: "forward it, its files with it",
    },
    Binding {
        keys: &[("/", Search)],
        name: "/",
        hint: "/",
        foot: &[(List, "search")],
        help: "search the messages listed as you type; ⏎ searches the server",
    },
    Binding {
        keys: &[("u", Seen)],
        name: "u",
        hint: "u",
        foot: &[
            (List, "read/unread"),
            (Reader, "unread"),
            (Map, "read/unread"),
        ],
        help: "mark read, or unread",
    },
    Binding {
        keys: &[("s", Flag)],
        name: "s",
        hint: "s",
        foot: &[(List, "flag"), (Reader, "flag"), (Map, "flag")],
        help: "flag, or unflag",
    },
    Binding {
        keys: &[("f", Files)],
        name: "f",
        hint: "f",
        foot: &[(Reader, "files & links")],
        help: "its files and links: PDFs and markdown in ohp, a linked PDF downloaded first",
    },
    Binding {
        keys: &[("o", Browser)],
        name: "o",
        hint: "o",
        foot: &[(Reader, "browser")],
        help: "open the message in the browser, as its HTML",
    },
    Binding {
        keys: &[("a", SaveFiles)],
        name: "a",
        hint: "a",
        foot: &[(Reader, "save files"), (Map, "archive")],
        help: "save its attachments to Downloads; in the map, take it, or what is marked, out of the Inbox",
    },
    Binding {
        keys: &[("esc", Back)],
        name: "esc",
        hint: "esc",
        foot: &[(Reader, "back"), (Picking, "cancel")],
        help: "back to the list; clear the search",
    },
    Binding {
        keys: &[("A", AddAccount)],
        name: "A",
        hint: "A",
        foot: &[(Side, "add account")],
        help: "add an account: its address and password",
    },
    Binding {
        keys: &[("R", Rename)],
        name: "R",
        hint: "R",
        foot: &[(Side, "rename")],
        help: "rename an account: the one picked in the sidebar, else the one listed; in the map, the label picked, onto another merging them",
    },
    Binding {
        keys: &[("D", Remove)],
        name: "D",
        hint: "D",
        foot: &[(Side, "remove")],
        help: "remove an account: the one picked in the sidebar, else the one listed",
    },
    Binding {
        keys: &[("C-n", NewMail), ("C-S-n", NewMail)],
        name: "C-n",
        hint: "C-n",
        foot: &[(Side, "new mail"), (List, "new mail")],
        help: "look for new mail now; it is looked for every two minutes",
    },
    // The map.
    Binding {
        keys: &[("M-g", MapView)],
        name: "M-g",
        hint: "M-g",
        foot: &[(Side, "map"), (List, "map")],
        help: "the map: every account's mail laid out by what it is about and who it is from",
    },
    Binding {
        keys: &[("M-l", ListView)],
        name: "M-l",
        hint: "M-l",
        foot: &[(Map, "list")],
        help: "back to the list, of what the sidebar picks: a folder, a label, a cluster, or the map's every cluster",
    },
    Binding {
        keys: &[("[", PreviousCluster), ("]", NextCluster)],
        name: "[ ]",
        hint: "[/]",
        foot: &[(Map, "cluster")],
        help: "in the map, narrow to the cluster before, or after",
    },
    Binding {
        keys: &[("m", Move)],
        name: "m",
        hint: "m",
        foot: &[(Map, "move to")],
        help: "in the map, move the message picked, or what is marked, onto a cluster or a label: then ⏎ on one, or a click on its anchor. It is given the label and taken out of the Inbox",
    },
    Binding {
        keys: &[("N", NewLabel)],
        name: "N",
        hint: "",
        foot: &[],
        help: "while moving, a new label for it alone: its name is asked, its commonest word given",
    },
    Binding {
        keys: &[("y", FileFirst), ("Y", FileAll), ("-", NotIt)],
        name: "y Y -",
        hint: "y/Y",
        foot: &[(Map, "file too")],
        help: "once filed, the label's first suggestion filed too, or all of them; - says the first is not one, and the map learns it",
    },
    Binding {
        keys: &[("M-r", Relayout)],
        name: "M-r",
        hint: "M-r",
        foot: &[(Map, "lay out again")],
        help: "in the map, lay it out again from where it is, and show what moved: ⏎ keeps it, esc drops it. Said when the map keeps under 70% of each message's nearest near it",
    },
    Binding {
        keys: &[("w", Lens)],
        name: "w",
        hint: "w",
        foot: &[(Map, "lens")],
        help: "in the map, lay it out by words and people, by words alone, or by people alone",
    },
    Binding {
        keys: &[("t", Since)],
        name: "t",
        hint: "t",
        foot: &[(Map, "since")],
        help: "in the map, all of it, this week's, or today's",
    },
    Binding {
        keys: &[("C", Colours)],
        name: "C",
        hint: "C",
        foot: &[(Map, "colour")],
        help: "in the map, colour by what it is about, by how near the message picked, or by its state",
    },
    Binding {
        keys: &[("r", ShowRead)],
        name: "r",
        hint: "r",
        foot: &[(Map, "read")],
        help: "in the map, show or hide read mail; elsewhere, say which keys reply",
    },
    Binding {
        keys: &[("v", Mark)],
        name: "v",
        hint: "v",
        foot: &[(Map, "mark")],
        help: "in the map, mark the cluster of the message picked, or drag a box over some; m, d, a, u and s then act on all of it, and v again lets it go",
    },
    Binding {
        keys: &[("!", NewOnly)],
        name: "!",
        hint: "!",
        foot: &[(Map, "new")],
        help: "in the map, dim the areas with nothing new: new is come since the map was last looked at, and still in the Inbox",
    },
    // The message being written: edit's keys.
    Binding {
        keys: &[],
        name: "C-c C-c",
        hint: "C-c C-c",
        foot: &[(Writing, "send")],
        help: "send the message being written; C-g or C-c C-k leaves it unsent",
    },
    Binding {
        keys: &[],
        name: "C-g",
        hint: "C-g",
        foot: &[(Writing, "cancel")],
        help: "",
    },
    Binding {
        keys: &[],
        name: "C-SPC",
        hint: "C-SPC",
        foot: &[(Writing, "mark")],
        help: "in the message being written, set the mark: C-w cuts from it to the cursor, M-w copies, C-y puts it back",
    },
    Binding {
        keys: &[],
        name: "C-w M-w",
        hint: "C-w/M-w",
        foot: &[(Writing, "cut/copy")],
        help: "",
    },
    Binding {
        keys: &[],
        name: "C-y",
        hint: "C-y",
        foot: &[(Writing, "yank")],
        help: "",
    },
    Binding {
        keys: &[],
        name: "C-s C-r",
        hint: "C-s/C-r",
        foot: &[(Writing, "search")],
        help: "in the message being written, search forward, or back, as you type; C-s again finds the next, ⏎ stays, C-g goes back",
    },
    Binding {
        keys: &[],
        name: "C-k",
        hint: "C-k",
        foot: &[(Writing, "kill")],
        help: "",
    },
    Binding {
        keys: &[("?", Help)],
        name: "?",
        hint: "?",
        foot: &[
            (Side, "help"),
            (List, "help"),
            (Reader, "help"),
            (Map, "help"),
        ],
        help: "all the keys",
    },
    Binding {
        keys: &[("q", Quit)],
        name: "q",
        hint: "q",
        foot: &[
            (Side, "quit"),
            (List, "quit"),
            (Reader, "quit"),
            (Map, "quit"),
        ],
        help: "quit",
    },
];

/// What `key` does, by the table.
pub fn act(key: KeyEvent) -> Option<Act> {
    static CHORDS: OnceLock<HashMap<Chord, Act>> = OnceLock::new();
    let chords = CHORDS.get_or_init(|| {
        KEYS.iter()
            .flat_map(|b| b.keys)
            .filter_map(|&(name, act)| Some((chord(key_named(name)?), act)))
            .collect()
    });
    chords.get(&chord(key)).copied()
}

/// The keys that do `act`, as the table names them.
pub fn keys_for(act: Act) -> Vec<&'static str> {
    KEYS.iter()
        .flat_map(|b| b.keys)
        .filter(|&&(_, a)| a == act)
        .map(|&(name, _)| name)
        .collect()
}

/// A hint at the foot: the key, what it does, and whether it is kept when
/// there is no room for every hint.
pub struct Hint {
    pub key: &'static str,
    pub what: &'static str,
    pub kept: bool,
}

/// The foot's hints in `pane`.
pub fn foot(pane: Pane) -> Vec<Hint> {
    KEYS.iter()
        .filter_map(|b| {
            let &(_, what) = b.foot.iter().find(|(p, _)| *p == pane)?;
            let kept = b.keys.iter().any(|&(_, a)| matches!(a, Help | Quit));
            Some(Hint {
                key: b.hint,
                what,
                kept,
            })
        })
        .collect()
}

/// The help's rows: the keys, and what they do.
pub fn help() -> impl Iterator<Item = (&'static str, &'static str)> {
    KEYS.iter()
        .filter(|b| !b.help.is_empty())
        .map(|b| (b.name, b.help))
}

pub type Chord = (KeyCode, KeyModifiers);

/// A key as the table compares it. A letter with Ctrl or Alt is in lower
/// case, Shift kept apart, as some terminals say Shift's letter in upper
/// case and some not; a letter alone is as typed, Shift in its case; any
/// other key is itself, whatever is held with it.
pub fn chord(key: KeyEvent) -> Chord {
    let held = key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT);
    match key.code {
        KeyCode::Char(c) if !held.is_empty() => {
            let shift = key.modifiers.contains(KeyModifiers::SHIFT) || c.is_uppercase();
            let shift = if shift {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            (KeyCode::Char(c.to_ascii_lowercase()), held | shift)
        }
        code => (code, KeyModifiers::NONE),
    }
}

/// The key a name stands for, as the table, the foot and the help name
/// them: `⏎` or `enter`, `␣` or `space`, `esc`, `tab`, `S-tab`, an arrow or
/// a page key, a character, or one after `C-`, `C-S-` or `M-`; none for a
/// name that stands for two, as `j/k`.
pub fn key_named(name: &str) -> Option<KeyEvent> {
    let none = KeyModifiers::NONE;
    let named = match name {
        "⏎" | "enter" => Some(KeyCode::Enter),
        "␣" | "space" => Some(KeyCode::Char(' ')),
        "esc" => Some(KeyCode::Esc),
        "tab" => Some(KeyCode::Tab),
        "S-tab" => Some(KeyCode::BackTab),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "pgup" => Some(KeyCode::PageUp),
        "pgdn" => Some(KeyCode::PageDown),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "backspace" => Some(KeyCode::Backspace),
        _ => None,
    };
    if let Some(code) = named {
        return Some(KeyEvent::new(code, none));
    }
    // Ctrl and Shift, as a terminal that tells them apart says them: the
    // letter in upper case.
    let (rest, with, upper) = if let Some(k) = name.strip_prefix("C-S-") {
        (k, KeyModifiers::CONTROL | KeyModifiers::SHIFT, true)
    } else if let Some(k) = name.strip_prefix("C-") {
        (k, KeyModifiers::CONTROL, false)
    } else if let Some(k) = name.strip_prefix("M-") {
        (k, KeyModifiers::ALT, false)
    } else {
        (name, none, false)
    };
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if upper => {
            Some(KeyEvent::new(KeyCode::Char(c.to_ascii_uppercase()), with))
        }
        (Some(c), None) => Some(KeyEvent::new(KeyCode::Char(c), with)),
        _ => None,
    }
}
