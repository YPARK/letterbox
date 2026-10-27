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
    /// The message read's attachments, to save.
    Save,
    /// The message read, presented by ohp.
    Present,
    /// What was thrown away on its own, and why.
    Debrief,
    /// The papers the message read tells of, to put in paperstack.
    Papers,
    /// The message read as its pages, or as its text again.
    Pages,
    /// Out of the Inbox, to Archive.
    Archive,
    NewMail,
    AddAccount,
    Rename,
    Remove,
    Help,
    /// The map's view, to be changed: the keys of `VIEW`.
    View,
    /// In the map, read mail shown or hidden.
    ShowRead,
    /// In the map, mail sent from here shown or hidden.
    ShowSent,
    /// In the map, only the mail not yet filed under a label, or all.
    Unfiled,
    /// The Inbox, looked through beside the map, shown or closed.
    Review,
    /// What was chosen for each message looked through, done.
    Apply,
    /// The message read made wider, or narrower.
    Wider,
    Narrower,
    /// The map, or the list, in the middle pane.
    MapView,
    ListView,
    /// The map, from the list; the list, from the map.
    SwitchView,
    PreviousCluster,
    NextCluster,
    /// The map laid out by another lens, or back as far as another time.
    Lens,
    Since,
    /// The areas with nothing new dimmed, or not.
    NewOnly,
    /// The cluster of the message picked marked, or let go.
    Mark,
    /// From the message picked, down or up, and all between marked, or
    /// selected in the Inbox looked through.
    MarkDown,
    MarkUp,
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
    /// The map looked at nearer, or farther, or all of it.
    ZoomIn,
    ZoomOut,
    ZoomAll,
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
    /// Picking the folder to save the message's files in.
    Saving,
    /// Naming the file to save.
    Naming,
    /// The map's view, being changed.
    Viewing,
    /// The map, in the middle pane.
    Map,
    /// The Inbox looked through, beside the map.
    Review,
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
use Pane::{
    Attaching, List, Map, Naming, Picking, Reader, Review as Reviewing, Saving, Side, Viewing,
    Writing,
};

/// What the map's view menu changes, by the key that changes it there.
pub const VIEW: &[(char, Act, &str)] = &[
    ('w', Lens, "lens"),
    ('t', Since, "since"),
    ('r', ShowRead, "read mail"),
    ('s', ShowSent, "sent mail"),
    ('f', Unfiled, "not yet filed"),
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
            (Saving, "into · on . save"),
            (Naming, "save"),
            (Reviewing, "read"),
        ],
        help: "open the folder, or read the message",
    },
    Binding {
        keys: &[
            ("j", Down),
            ("down", Down),
            ("C-n", Down),
            ("k", Up),
            ("up", Up),
            ("C-p", Up),
        ],
        name: "j k C-n C-p",
        hint: "j/k",
        foot: &[
            (Side, "move"),
            (Picking, "move"),
            (Map, "move"),
            (Reviewing, "move"),
        ],
        help: "move; in a message, scroll; in the map, to the nearest message that way, as h and l; the arrows too, and C-n and C-p, as Emacs has them",
    },
    Binding {
        keys: &[
            ("h", Left),
            ("left", Left),
            ("C-b", Left),
            ("l", Right),
            ("right", Right),
            ("C-f", Right),
            ("tab", NextPane),
            ("S-tab", PreviousPane),
        ],
        name: "h l C-b C-f tab",
        hint: "",
        foot: &[],
        help: "move between folders, messages and the message read; tab and S-tab in the map; C-b and C-f as h and l",
    },
    Binding {
        keys: &[
            ("space", ScreenDown),
            ("pgdn", PageDown),
            ("C-v", PageDown),
            ("b", ScreenUp),
            ("pgup", PageUp),
            ("M-v", PageUp),
        ],
        name: "space b C-v M-v",
        hint: "␣",
        foot: &[(Reader, "page"), (List, "select"), (Reviewing, "keep")],
        help: "a page down, or up, in the message; in the list, space marks the message picked and goes on, as v; in the Inbox looked through, keep it there. C-v and M-v, as the page keys, a page there",
    },
    Binding {
        keys: &[("n", NextMessage), ("p", PreviousMessage)],
        name: "n p",
        hint: "n/p",
        foot: &[(Reader, "next/previous")],
        help: "read the next message, or the one before; in the map, searched, the next found, or the one before",
    },
    Binding {
        keys: &[
            ("g", Top),
            ("home", Top),
            ("M-<", Top),
            ("G", End),
            ("end", End),
            ("M->", End),
        ],
        name: "g G M-< M->",
        hint: "",
        foot: &[],
        help: "go to the top, or the end; M-< and M-> as Emacs has them",
    },
    Binding {
        keys: &[("d", Trash)],
        name: "d",
        hint: "d",
        foot: &[
            (List, "trash"),
            (Reader, "trash"),
            (Map, "trash"),
            (Reviewing, "discard"),
        ],
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
        keys: &[("f", Forward)],
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
        help: "open the message in the browser, as its HTML, or one of its files and links: PDFs and markdown in ohp, a linked PDF downloaded first; s and S there save as S does",
    },
    Binding {
        keys: &[("S", Save)],
        name: "S",
        hint: "S",
        foot: &[(Reader, "save")],
        help: "save the message read's attachments: ⏎ the one picked, a all of them, into a folder picked as files to attach are, begun where the last was saved, else in Downloads: ⏎ goes into a folder, and on . saves there, a name typed that no folder has made. The one picked is named first: its name but the extension picked, typed over, ← → to edit it, ⏎ to keep it",
    },
    Binding {
        keys: &[("V", Pages)],
        name: "V",
        hint: "V",
        foot: &[],
        help: "the message read as its pages, typeset by ohp and drawn in the terminal, where it draws images, as kitty, WezTerm, Ghostty and iTerm2 do; j k or space b turn them; V again its text. Built --features pages",
    },
    Binding {
        keys: &[("P", Papers)],
        name: "P",
        hint: "P",
        foot: &[(Reader, "papers")],
        help: "the papers the message read tells of, a Scholar alert's or a journal's, each said to be in your paperstack, like what you starred there, or new: ⏎ puts the one picked there, starred, o opens it",
    },
    Binding {
        keys: &[("B", Debrief)],
        name: "B",
        hint: "B",
        foot: &[],
        help: "what was thrown away on its own in the last 7 days, and why: ⏎ puts the one picked back, u all of them, each learnt as kept. Told of as letterbox opens too",
    },
    Binding {
        keys: &[("O", Present)],
        name: "O",
        hint: "O",
        foot: &[(Reader, "ohp")],
        help: "present the message read with ohp, typeset as a page, its images in it; quit ohp to come back",
    },
    Binding {
        keys: &[("e", Archive)],
        name: "e",
        hint: "e",
        foot: &[(List, "archive"), (Reader, "archive"), (Map, "archive")],
        help: "archive it: out of the Inbox, to Archive; in the map, it or what is marked",
    },
    Binding {
        keys: &[("esc", Back), ("C-g", Back)],
        name: "esc",
        hint: "esc",
        foot: &[
            (Reader, "back"),
            (Picking, "cancel"),
            (Attaching, "cancel"),
            (Saving, "cancel"),
            (Naming, "back"),
            (Viewing, "close"),
        ],
        help: "back to the list; clear the search; out of a cluster gone into, a level. C-g too, as Emacs has it",
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
        keys: &[("U", NewMail)],
        name: "U",
        hint: "U",
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
        keys: &[("M", SwitchView)],
        name: "M",
        hint: "M",
        foot: &[(Side, "map"), (List, "map"), (Map, "list")],
        help: "the map, or back to the list from it, in any terminal: as M-g and M-l",
    },
    Binding {
        keys: &[("M-g", MapView), ("s-g", MapView)],
        name: "M-g s-g",
        hint: "M-g",
        foot: &[],
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
        foot: &[(Map, "move to"), (Reviewing, "move to")],
        help: "in the map, move the message picked, or what is marked, onto a cluster or a label: then ⏎ on one, or a click on its anchor. It is given the label and taken out of the Inbox",
    },
    Binding {
        keys: &[("N", NewLabel)],
        name: "N",
        hint: "N",
        foot: &[(Reviewing, "new label")],
        help: "while moving, or in the Inbox looked through, a new label for it alone: its name is asked, its commonest word given; made as it is filed",
    },
    Binding {
        keys: &[("y", FileFirst), ("Y", FileAll), ("-", NotIt)],
        name: "y Y -",
        hint: "y/Y",
        foot: &[(Reviewing, "as suggested")],
        help: "a message not yet filed, filed where it would go, or, Y, all like it, marked first; once filed, the label's first suggestion filed too, or all of them; - says it is not one, and the map learns it",
    },
    Binding {
        keys: &[("M-r", Relayout)],
        name: "M-r",
        hint: "M-r",
        foot: &[],
        help: "in the map, lay it out again from where it is, and show what moved: ⏎ keeps it, esc drops it. Said when the map keeps under 70% of each message's nearest near it",
    },
    Binding {
        keys: &[("z", ZoomIn), ("Z", ZoomOut), ("0", ZoomAll)],
        name: "z Z 0",
        hint: "z/Z",
        foot: &[(Map, "in/out")],
        help: "in the map, a mosaic of the clusters, each a block as big as how many it holds, beside those it lies by: z, ⏎ or a click goes into the block picked, its parts a mosaic too, and theirs, down to the threads of a part too small to split, as a tree gone along by j and k, into by l or ⏎, out of by h; Z or esc goes out a level, 0 to all the clusters. The wheel up over a block goes into it, and down out a level",
    },
    Binding {
        keys: &[("i", Review)],
        name: "i",
        hint: "i",
        foot: &[(Map, "review"), (Reviewing, "close")],
        help: "the Inbox, beside the map, to look through as far as you like: each message with what is suggested for it, none chosen until you choose; m drops down the labels nearest it to move it to, narrowed as you type, space keeps it, d discards it, y takes what is suggested; ⏎ reads it",
    },
    Binding {
        keys: &[("$", Apply)],
        name: "$",
        hint: "$",
        foot: &[(Reviewing, "apply")],
        help: "in the Inbox looked through, do what was chosen, once asked: filed, kept, or discarded; those not chosen are left as they are",
    },
    Binding {
        keys: &[(",", View)],
        name: ",",
        hint: ",",
        foot: &[(Map, "view")],
        help: "in the map, what it shows and how: w its lens, t how far back, r read mail, s sent mail, f only the mail not yet filed, n the areas with nothing new dimmed, l lay it out again",
    },
    Binding {
        keys: &[("v", Mark)],
        name: "v",
        hint: "v",
        foot: &[(Map, "mark")],
        help: "in the map, mark the cluster of the message picked, its block boxed; in the list, mark the message picked, or let it go, as space does; m, d, e, u and s then act on all of it, and esc lets it go",
    },
    Binding {
        keys: &[("S-down", MarkDown), ("S-up", MarkUp)],
        name: "S-down S-up",
        hint: "S-↓/S-↑",
        foot: &[(List, "select"), (Reviewing, "select")],
        help: "in the list, mark from the message picked down, or up, to where it goes, and back fewer; in the Inbox looked through, select so: m, d, space or y then choose for all of them, and esc lets them go",
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
        foot: &[(Attaching, "up"), (Saving, "up")],
        help: "",
    },
    Binding {
        keys: &[],
        name: "~",
        hint: "~",
        foot: &[(Attaching, "home"), (Saving, "home")],
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
        keys: &[],
        name: "C-c C-s",
        hint: "C-c C-s",
        foot: &[(Writing, "grammar")],
        help: "in the message being written, check its grammar with a small model from Hugging Face, or the Grammar button on its top edge: each change asked in a pop-up, y takes it, n skips it, a accepts all, i ignores all; C-c C-c checks it too before it is sent, and asks if it is to go; built with --features grammar",
    },
    Binding {
        keys: &[],
        name: "tab S-tab",
        hint: "",
        foot: &[],
        help: "in the message being written, from field to field, To, Cc, Subject, then what is written, and round; S-tab back",
    },
    Binding {
        keys: &[],
        name: "M-$",
        hint: "",
        foot: &[],
        help: "in the message being written, as C-c C-s: Emacs's key to check spelling",
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
        // A chord the app reads itself, as Emacs has it: q alone was too
        // easily pressed.
        keys: &[],
        name: QUIT,
        hint: QUIT,
        foot: &[
            (Side, "quit"),
            (List, "quit"),
            (Reader, "quit"),
            (Map, "quit"),
        ],
        help: "quit",
    },
];

/// The keys that quit, pressed in turn, as Emacs has them.
pub const QUIT: &str = "C-x C-c";

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
            let kept = b.keys.iter().any(|&(_, a)| a == Help) || b.name == QUIT;
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
            // Shift on a letter alone: `<` is Shift's already, said or not.
            let shift = c.is_alphabetic()
                && (key.modifiers.contains(KeyModifiers::SHIFT) || c.is_uppercase());
            let shift = if shift {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            (KeyCode::Char(c.to_ascii_lowercase()), held | shift)
        }
        // Shift and an arrow up or down marks as it goes: a key of its own.
        code @ (KeyCode::Up | KeyCode::Down) if key.modifiers.contains(KeyModifiers::SHIFT) => {
            (code, KeyModifiers::SHIFT)
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
        "S-up" | "S-↑" => return Some(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT)),
        "S-down" | "S-↓" => return Some(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT)),
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
