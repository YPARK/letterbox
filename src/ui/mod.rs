//! The screen, laid out as a mail app's: the accounts and their folders on
//! a tinted sidebar, the folder's messages, and the message read, side by
//! side where there is room. In a narrow window, as an Emacs buffer split
//! in two, one of them at a time.

mod blocks;
mod dialogs;
mod foot;
mod form;
mod list;
mod mappane;
mod reader;
mod review;
mod writing;
use self::blocks::*;
use self::dialogs::*;
use self::foot::*;
use self::form::*;
use self::list::*;
use self::mappane::*;
use self::reader::*;
use self::review::*;
use self::writing::*;

use crate::app::{
    AGAIN, App, Body, Button, Dialog, EMAIL, FIELDS, Failure, Files, Focus, Form, Item, KEEP,
    PASSPHRASE, PASSWORD, Row, SERVER, SIGN_IN, State, Target, TrashPick, View, ohp_reads,
};
use crate::config::{self, Auth};
use crate::keymap::{self, Pane};
use crate::mail::{Message, Role};
use crate::secret::Keeper;
use crate::theme::Theme;
use crate::vault;
use jiff::{Timestamp, ToSpan, Zoned};
use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph,
};
use std::sync::Arc;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SIDEBAR: u16 = 26;
const LIST: u16 = 46;
/// The narrowest screen with the sidebar, the list and the message read
/// side by side; and with the sidebar beside the list.
const WIDE: u16 = 120;
const MEDIUM: u16 = 70;
/// How narrow the middle pane, or the message read, may be dragged.
const NARROWEST: u16 = 24;
const LABEL: usize = 6;
/// The most lines the keys take at the foot.
const FOOT: usize = 2;
const KEY: usize = 11;

/// How wide the sidebar, the list and the message read are drawn, on a
/// screen `width` wide, with `focus` on one of them.
pub fn panes(width: u16, focus: Focus) -> [u16; 3] {
    if width >= WIDE {
        [SIDEBAR, LIST, width - SIDEBAR - LIST]
    } else if width >= MEDIUM {
        match focus {
            Focus::Reader => [0, 0, width],
            _ => [SIDEBAR, width - SIDEBAR, 0],
        }
    } else {
        match focus {
            Focus::Side => [width, 0, 0],
            Focus::List => [0, width, 0],
            Focus::Reader => [0, 0, width],
        }
    }
}

/// The panes' widths with the map in the middle: no sidebar, for the map
/// shows what it would, its clusters and labels, on itself; the map in its
/// room and the list's, as wide as the message read where there is room.
pub fn map_panes(width: u16, focus: Focus) -> [u16; 3] {
    match panes(width, focus) {
        [side, list, read] if list > 0 && read > 0 => {
            let map = ((list + read) / 2).max(list);
            [0, side + map, list + read - map]
        }
        [_, _, read] if read == width => [0, 0, width],
        _ => [0, width, 0],
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.keep_reading_in_view();
    let t = app.theme;
    app.targets.clear();
    f.render_widget(
        Block::new().style(Style::new().bg(t.page).fg(t.text)),
        f.area(),
    );
    let (foot_lines, spots) = foot(app, f.area().width);
    let [top, main, foot_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(foot_lines.len() as u16),
    ])
    .areas(f.area());
    let [side_w, mut list_w, mut read_w] = if app.map.mode == crate::map::Mode::Map {
        map_panes(main.width, app.focus)
    } else {
        panes(main.width, app.focus)
    };
    // The message as wide as it was dragged, with room left for the rest.
    if list_w > 0
        && read_w > 0
        && let Some(w) = app.reader_width
    {
        let both = list_w + read_w;
        read_w = w.clamp(NARROWEST.min(both / 2), both.saturating_sub(NARROWEST));
        list_w = both - read_w;
    }
    app.panes_right = main.right();
    app.side_by_side = list_w > 0 && read_w > 0;
    let [side, list, read] = Layout::horizontal([
        Constraint::Length(side_w),
        Constraint::Length(list_w),
        Constraint::Length(read_w),
    ])
    .areas(main);
    search(f, app, top);
    if side_w > 0 {
        sidebar(f, app, side);
    }
    if list_w > 0 {
        if app.map.mode == crate::map::Mode::Map {
            map_pane(f, app, list);
        } else {
            messages(f, app, list);
        }
    }
    if read_w > 0 {
        if app.map.mode == crate::map::Mode::Map && app.review.as_ref().is_some_and(|r| !r.reading)
        {
            review_pane(f, app, read);
        } else {
            reader(f, app, read);
        }
    }
    // The line before the message, to drag: it and a column either side,
    // as one alone is hard to hit.
    if list_w > 0 && read_w > 0 {
        let grip = Rect {
            x: read.x.saturating_sub(1),
            width: 3,
            ..read
        };
        app.targets.push((grip, Target::Divider));
    }
    app.read_width = read_w;
    app.read_height = read.height;
    keys(f, app, foot_area, foot_lines, spots);
    if app.compose.is_some() {
        compose_popup(f, app, main);
    }
    let popup = match &app.dialog {
        Dialog::Closed => None,
        &Dialog::Help(scroll) => {
            let (area, scroll) = help(f, &t, scroll);
            app.dialog = Dialog::Help(scroll);
            Some((area, Vec::new()))
        }
        Dialog::Form(form) => Some(form_dialog(f, &t, form)),
        Dialog::Files(files) => Some(files_dialog(f, &t, files)),
        Dialog::Attach(picker) => Some(attach_dialog(f, &t, picker)),
        Dialog::SaveTo(s) => Some(attach_dialog(f, &t, &s.picker)),
        Dialog::View => Some(view_dialog(f, &t, &app.map)),
        Dialog::Debrief(d) => Some(debrief_dialog(f, &t, d)),
        Dialog::Papers(p) => Some(papers_dialog(f, &t, p)),
        Dialog::StartMap => Some((
            question(
                f,
                &t,
                "Begin the map?",
                "y begins it · n not now",
                "The map lays out every account's mail by what it is about and who it is from. \
                 To do so letterbox reads each message's headers and first lines into an index \
                 beside the vault, encrypted with a key kept where your passwords are, and keeps \
                 it up to date in the background, newest first. Nothing leaves this machine.",
                |_| Line::raw(""),
            )
            .0,
            Vec::new(),
        )),
        Dialog::Trash(pick) => Some(trash_dialog(f, &t, pick)),
        &Dialog::Remove(a) => Some(removing(f, &t, &app.accounts[a].name)),
        Dialog::Apply(plan) => Some(applying(f, &t, &plan.said)),
        Dialog::Applied(applied) => Some(applied_dialog(f, &t, applied)),
        Dialog::Rename(a, typed) => Some(renaming(f, &t, &app.accounts[*a].name, typed)),
        Dialog::Label(typed) => Some(asking(
            f,
            &t,
            "New label",
            "⏎ files it · esc stops",
            "What is the new label to be called? What is moving is given it, and taken out of the Inbox.",
            typed,
        )),
        Dialog::RenameLabel(old, typed) => Some(asking(
            f,
            &t,
            "Rename label",
            "⏎ renames · esc keeps it",
            &format!(
                "What is {old} to be called? On every account, and on its server; called as another label is, the two are one."
            ),
            typed,
        )),
    };
    if let Some((area, targets)) = popup {
        app.targets.push((area, Target::Popup));
        app.targets.extend(targets);
        let spots = border_keys(f.buffer_mut(), area);
        app.targets.extend(spots);
    }
}

/// What a click on a hint naming `keys` presses: a key, or two in turn.
fn press(keys: &[KeyEvent]) -> Option<Target> {
    match *keys {
        [k] => Some(Target::Key(k)),
        [a, b] => Some(Target::Chord(a, b)),
        _ => None,
    }
}

/// Where each key of the hint `label`, drawn from column `x` of row `y`,
/// is clicked: the whole of `width` where the hint names one key or chord,
/// else each key named where it is drawn.
fn hint_spots(label: &str, x: u16, y: u16, width: u16) -> Targets {
    let parts = keymap::pressed(label);
    let whole = parts.len() == 1 && parts[0].1 == label.chars().count();
    parts
        .into_iter()
        .filter_map(|(at, w, keys)| {
            let rect = if whole {
                Rect::new(x, y, width, 1)
            } else {
                Rect::new(x + at as u16, y, w as u16, 1)
            };
            Some((rect, press(&keys)?))
        })
        .collect()
}

/// `items`, each a key and what it does, as spans from column `x` of row
/// `y`, each one clickable as its key: the key a block of its own colour,
/// to stand out from the text about it.
fn keyed(
    t: &Theme,
    items: &[(&str, &str)],
    x: u16,
    y: u16,
    spots: &mut Targets,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = x;
    for (key, what) in items {
        let block = format!(" {key} ");
        let what = format!("{what}   ");
        let width = (block.width() + what.width()) as u16;
        spots.extend(hint_spots(key, at + 1, y, width - 1));
        spans.push(Span::styled(
            block,
            Style::new().fg(t.title).bg(t.focus).bold(),
        ));
        spans.push(Span::styled(what, Style::new().fg(t.soft)));
        at += width;
    }
    spans
}

/// The keys a dialog's bottom border says, as it is drawn, each part
/// between ` · ` clicked as the key it begins with.
fn border_keys(buf: &mut ratatui::buffer::Buffer, area: Rect) -> Targets {
    if area.height < 2 || area.width < 4 {
        return Vec::new();
    }
    let y = area.bottom() - 1;
    let cells: Vec<(u16, String)> = (area.x + 1..area.right() - 1)
        .map(|x| (x, buf[(x, y)].symbol().to_string()))
        .filter(|(_, s)| !s.is_empty())
        .collect();
    let text: String = cells.iter().map(|(_, s)| s.as_str()).collect();
    let Some(text) = text.strip_prefix(' ') else {
        return Vec::new();
    };
    let text = text.trim_end_matches('─').trim_end();
    let mut spots = Vec::new();
    let mut at = 1;
    for part in text.split(" · ") {
        let width = part.chars().count();
        if let Some(key) = part.split(' ').next()
            && let Some((x, _)) = cells.get(at)
        {
            spots.extend(hint_spots(key, *x, y, width as u16));
        }
        at += width + 3;
    }
    spots
}

type Targets = Vec<(Rect, Target)>;

pub(crate) fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b.div_ceil(1024)),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Where a key's hint is at the foot: its line, how far along, how wide.
type Spot = (usize, u16, u16, Target);

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// When a message was sent, as a list shows it: the time today, else
/// `Yesterday`, else the day this week, else the date.
pub fn short_date(seconds: i64, now: &Zoned) -> String {
    let Ok(at) = Timestamp::from_second(seconds) else {
        return String::new();
    };
    let at = at.to_zoned(now.time_zone().clone());
    let (day, today) = (at.date(), now.date());
    if day == today {
        at.strftime("%-I:%M %p").to_string()
    } else if today.yesterday().ok() == Some(day) {
        "Yesterday".to_string()
    } else if day < today && today.checked_sub(6.days()).is_ok_and(|week| day >= week) {
        at.strftime("%A").to_string()
    } else {
        at.strftime("%Y-%m-%d").to_string()
    }
}

/// When a message was sent, in full, in this machine's time zone.
pub fn long_date(seconds: i64) -> String {
    match Timestamp::from_second(seconds) {
        Ok(at) => at
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%A %-d %B %Y, %-I:%M %p")
            .to_string(),
        Err(_) => String::new(),
    }
}

/// `text` as lines no wider than `width`, broken at spaces, or within a
/// word longer than a line, as an address.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for line in text.split('\n') {
        if line.width() <= width {
            out.push(line.to_string());
            continue;
        }
        let mut cur = String::new();
        let mut w = 0;
        let mut start = true;
        for word in line.split(' ') {
            if start && word.is_empty() {
                cur.push(' ');
                w += 1;
                continue;
            }
            start = false;
            let ww = word.width();
            let need = if cur.trim().is_empty() {
                w + ww
            } else {
                w + 1 + ww
            };
            if need <= width {
                if !cur.trim().is_empty() {
                    cur.push(' ');
                    w += 1;
                }
                cur.push_str(word);
                w += ww;
                continue;
            }
            if !cur.trim().is_empty() {
                out.push(std::mem::take(&mut cur));
                w = 0;
            }
            // Longer than a line, as an address: broken after a `/`, `.`
            // or the like, where that leaves most of the line used.
            let mut rest = word;
            while w + rest.width() > width {
                let room = width - w;
                let (mut fit, mut used) = (0, 0);
                let (mut slash, mut mark) = (None, None);
                for (i, c) in rest.char_indices() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw > room {
                        break;
                    }
                    used += cw;
                    fit = i + c.len_utf8();
                    if c == '/' {
                        slash = Some(fit);
                    } else if ".@-?&=_,;:".contains(c) {
                        mark = Some(fit);
                    }
                }
                let cut = [slash, mark]
                    .into_iter()
                    .flatten()
                    .find(|&b| b * 2 >= fit)
                    .unwrap_or(fit);
                if cut == 0 && !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    w = 0;
                    continue;
                }
                let cut = cut.max(rest.chars().next().map_or(0, char::len_utf8));
                cur.push_str(&rest[..cut]);
                out.push(std::mem::take(&mut cur));
                w = 0;
                rest = &rest[cut..];
            }
            cur.push_str(rest);
            w += rest.width();
        }
        out.push(cur);
    }
    out
}

/// `s`, cut to `width` columns with `…` if it is wider.
fn clip(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// The end of `s`, as much as `width` columns show, `…` before it if it is
/// cut: what is typed, its end in sight.
fn tail(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut w = 1;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if w + cw > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    std::iter::once('…').chain(out.into_iter().rev()).collect()
}

fn tilde(path: &std::path::Path) -> String {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}
