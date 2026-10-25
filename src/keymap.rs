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
    Files,
    /// Out of the Inbox, to Archive.
    Archive,
    NewMail,
    AddAccount,
    Rename,
    Remove,
    Help,
    Quit,
    /// The map's view, to be changed: the keys of `VIEW`.
    View,
    /// In the map, read mail shown or hidden.
    ShowRead,
    /// In the map, mail sent from here shown or hidden.
    ShowSent,
    /// The message read made wider, or narrower.
    Wider,
    Narrower,
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
    /// Picking files to attach to the message being written.
    Attaching,
    /// The map's view, being changed.
    Viewing,
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
use Pane::{Attaching, List, Map, Picking, Reader, Side, Viewing, Writing};

/// What the map's view menu changes, by the key that changes it there.
pub const VIEW: &[(char, Act, &str)] = &[
    ('w', Lens, "lens"),
    ('t', Since, "since"),
    ('c', Colours, "colour"),
    ('r', ShowRead, "read mail"),
    ('s', ShowSent, "sent mail"),
    ('n', NewOnly, "new only"),
    ('l', Relayout, "lay it out again"),
];

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
            (Attaching, "attach"),
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
        help: "read the next message, or the one before; in the map, searched, the next found, or the one before",
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
        keys: &[("r", Reply), ("C-r", Reply)],
        name: "r C-r",
        hint: "r",
        foot: &[(List, "reply"), (Reader, "reply")],
        help: "reply to the sender",
    },
    Binding {
        keys: &[("a", ReplyAll)],
        name: "a",
        hint: "a",
        foot: &[(List, "reply all"), (Reader, "reply all")],
        help: "reply to all: the sender, and the others it went to, copied",
    },
    Binding {
        keys: &[("f", Forward), ("C-f", Forward)],
        name: "f C-f",
        hint: "f",
        foot: &[(List, "forward"), (Reader, "forward")],
        help: "forward it, its files with it",
    },
    Binding {
        keys: &[("/", Search)],
        name: "/",
        hint: "/",
        foot: &[(List, "search"), (Map, "search")],
        help: "search the messages listed as you type; ⏎ searches the server. In the map, what holds every word is lit as you type, the rest faded; n and p go from one found to the next",
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
        keys: &[("o", Files)],
        name: "o",
        hint: "o",
        foot: &[(Reader, "open")],
        help: "open the message in the browser, as its HTML, or one of its files and links: PDFs and markdown in ohp, a linked PDF downloaded first; s saves the one picked to Downloads, S all its files",
    },
    Binding {
        keys: &[("e", Archive)],
        name: "e",
        hint: "e",
        foot: &[(List, "archive"), (Reader, "archive"), (Map, "archive")],
        help: "archive it: out of the Inbox, to Archive; in the map, it or what is marked",
    },
    Binding {
        keys: &[("esc", Back)],
        name: "esc",
        hint: "esc",
        foot: &[
            (Reader, "back"),
            (Picking, "cancel"),
            (Attaching, "cancel"),
            (Viewing, "close"),
        ],
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
        keys: &[("C-n", NewMail)],
        name: "C-n",
        hint: "C-n",
        foot: &[(Side, "new mail"), (List, "new mail")],
        help: "look for new mail now; it is looked for every two minutes",
    },
    Binding {
        keys: &[("<", Wider), (">", Narrower)],
        name: "< >",
        hint: "</>",
        foot: &[(Reader, "width")],
        help: "make the message read wider, or narrower; or drag the line before it with the mouse",
    },
    // The map.
    Binding {
        keys: &[("M-g", MapView), ("s-g", MapView)],
        name: "M-g s-g",
        hint: "M-g",
        foot: &[(Side, "map"), (List, "map")],
        help: "the map: every account's mail laid out by what it is about and who it is from",
    },
    Binding {
        keys: &[("M-l", ListView), ("s-l", ListView)],
        name: "M-l s-l",
        hint: "M-l",
        foot: &[],
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
        foot: &[],
        help: "in the map, lay it out again from where it is, and show what moved: ⏎ keeps it, esc drops it. Said when the map keeps under 70% of each message's nearest near it",
    },
    Binding {
        keys: &[(",", View)],
        name: ",",
        hint: ",",
        foot: &[(Map, "view")],
        help: "in the map, what it shows and how: w its lens, t how far back, c its colours, r read mail, s sent mail, n the areas with nothing new dimmed, l lay it out again",
    },
    Binding {
        keys: &[("v", Mark)],
        name: "v",
        hint: "v",
        foot: &[(Map, "mark")],
        help: "in the map, mark the cluster of the message picked, or drag a box over some; m, d, e, u and s then act on all of it, and v again lets it go",
    },
    // The message being written: edit's keys.
    Binding {
        keys: &[],
        name: "C-c C-c",
        hint: "C-c C-c",
        foot: &[(Writing, "send")],
        help: "send the message being written",
    },
    Binding {
        keys: &[],
        name: "C-c C-k",
        hint: "C-c C-k",
        foot: &[(Writing, "discard")],
        help: "in the message being written, let it go unsent; changed, it asks first, as Emacs does",
    },
    Binding {
        keys: &[],
        name: "C-g",
        hint: "C-g",
        foot: &[],
        help: "in the message being written, let go of a search, the mark, or C-c, as Emacs's C-g does; never the message",
    },
    Binding {
        keys: &[],
        name: "C-c C-a",
        hint: "C-c C-a",
        foot: &[(Writing, "attach")],
        help: "in the message being written, attach files, picked a folder at a time: typing narrows, ⏎ goes into a folder, ← up, ~ home, ␣ marks; or drop them onto the terminal. Each is an Attach line, read as it is sent",
    },
    // Picking files to attach: the picker's keys.
    Binding {
        keys: &[],
        name: "␣",
        hint: "␣",
        foot: &[(Attaching, "mark")],
        help: "",
    },
    Binding {
        keys: &[],
        name: "←",
        hint: "←",
        foot: &[(Attaching, "up")],
        help: "",
    },
    Binding {
        keys: &[],
        name: "~",
        hint: "~",
        foot: &[(Attaching, "home")],
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
        keys: &[("?", Help), ("F1", Help)],
        name: "? F1",
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

/// The keys a hint names, each part of it with where it begins in the
/// hint and how wide it is, both in characters: `j/k` is two parts, a key
/// each; `C-c C-c` one, two keys pressed in turn. A part that names no
/// key, or more than two, is left out.
pub fn pressed(hint: &str) -> Vec<(usize, usize, Vec<KeyEvent>)> {
    let parts: Vec<&str> = if hint.len() > 1 && hint.contains('/') {
        hint.split('/').collect()
    } else {
        vec![hint]
    };
    let mut at = 0;
    let mut out = Vec::new();
    for part in parts {
        let width = part.chars().count();
        let keys: Option<Vec<KeyEvent>> = part.split(' ').map(key_named).collect();
        if let Some(keys) = keys.filter(|k| (1..=2).contains(&k.len())) {
            out.push((at, width, keys));
        }
        at += width + 1;
    }
    out
}

pub type Chord = (KeyCode, KeyModifiers);

/// A key as the table compares it. A letter with Ctrl, Alt or Super is in
/// lower case, Shift kept apart, as some terminals say Shift's letter in
/// upper case and some not; a letter alone is as typed, Shift in its case;
/// any other key is itself, whatever is held with it.
pub fn chord(key: KeyEvent) -> Chord {
    let held = key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
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
/// a page key, a character, or one after `C-`, `C-S-`, `M-` or `s-`, Super
/// (⌘ on a Mac, the Windows key elsewhere); none for a name that stands
/// for two, as `j/k`.
pub fn key_named(name: &str) -> Option<KeyEvent> {
    let none = KeyModifiers::NONE;
    let named = match name {
        "⏎" | "enter" => Some(KeyCode::Enter),
        "␣" | "space" => Some(KeyCode::Char(' ')),
        "esc" => Some(KeyCode::Esc),
        "tab" => Some(KeyCode::Tab),
        "S-tab" => Some(KeyCode::BackTab),
        "up" | "↑" => Some(KeyCode::Up),
        "down" | "↓" => Some(KeyCode::Down),
        "left" | "←" => Some(KeyCode::Left),
        "right" | "→" => Some(KeyCode::Right),
        "pgup" => Some(KeyCode::PageUp),
        "pgdn" => Some(KeyCode::PageDown),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "backspace" => Some(KeyCode::Backspace),
        "F1" => Some(KeyCode::F(1)),
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
    } else if let Some(k) = name.strip_prefix("s-") {
        (k, KeyModifiers::SUPER, false)
    } else {
        (name, none, false)
    };
    if rest == "SPC" {
        return Some(KeyEvent::new(KeyCode::Char(' '), with));
    }
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if upper => {
            Some(KeyEvent::new(KeyCode::Char(c.to_ascii_uppercase()), with))
        }
        (Some(c), None) => Some(KeyEvent::new(KeyCode::Char(c), with)),
        _ => None,
    }
}
