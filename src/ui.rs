//! The screen, laid out as a mail app's: the accounts and their folders on
//! a tinted sidebar, the folder's messages, and the message read, side by
//! side where there is room. In a narrow window, as an Emacs buffer split
//! in two, one of them at a time.

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

fn dialog<'a>(t: &Theme, title: &'a str, foot: &'a str) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.dim))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(t.title).bold(),
        ))
        .title_bottom(Line::styled(format!(" {foot} "), Style::new().fg(t.dim)))
        .padding(Padding::horizontal(1))
        .style(Style::new().bg(t.page).fg(t.text))
}

/// A line of buttons, the first lit; and where each is along it.
fn buttons(t: &Theme, labels: &[(&str, Button)]) -> (Line<'static>, Vec<(u16, u16, Button)>) {
    let mut spans = Vec::new();
    let mut at = Vec::new();
    let mut x = 0u16;
    for (i, (label, button)) in labels.iter().enumerate() {
        let text = format!(" {label} ");
        let w = text.width() as u16;
        let style = if i == 0 {
            Style::new().fg(t.on_accent).bg(t.accent).bold()
        } else {
            Style::new().fg(t.text).bg(t.focus)
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw("  "));
        at.push((x, w, *button));
        x += w + 2;
    }
    (Line::from(spans), at)
}

/// Where what is clicked on in a dialog drawn in `area` is: `line` of
/// its text, `x` along it, `width` wide.
fn spot(area: Rect, line: usize, x: u16, width: u16) -> Rect {
    Rect {
        x: area.x + 2 + x,
        y: area.y + 1 + line as u16,
        width: width.min(area.width.saturating_sub(4 + x)),
        height: 1,
    }
}

type Targets = Vec<(Rect, Target)>;

/// The form an account is added with: each field on a line, the one typed
/// in lit; the password shown as dots; where it is kept, picked; what to
/// know, or what went wrong, below; and its buttons.
fn form_dialog(f: &mut Frame, t: &Theme, form: &Form) -> (Rect, Targets) {
    let width = f.area().width.min(72);
    let room = (width as usize).saturating_sub(4);
    let value_w = room.saturating_sub(KEY + 1);
    let mut lines = vec![Line::raw("")];
    let shown = form.shown();
    for &i in &shown {
        let name = FIELDS[i];
        let here = i == form.at && !form.checking;
        let label = Span::styled(
            format!("{name:<KEY$}"),
            if here {
                Style::new().fg(t.title).bold()
            } else {
                Style::new().fg(t.dim)
            },
        );
        let pick = |picked: bool| {
            if picked {
                Style::new()
                    .fg(t.title)
                    .bg(if here { t.focus } else { t.row })
                    .bold()
            } else {
                Style::new().fg(t.dim)
            }
        };
        if i == SIGN_IN {
            let mut spans = vec![label, Span::raw(" ")];
            for (auth, name) in [(Auth::Password, "password"), (Auth::Microsoft, "microsoft")] {
                spans.push(Span::styled(format!(" {name} "), pick(form.auth == auth)));
                spans.push(Span::raw(" "));
            }
            lines.push(Line::from(spans));
            continue;
        }
        if i == KEEP {
            let mut spans = vec![label, Span::raw(" ")];
            for (k, keeper) in form.keepers.iter().enumerate() {
                let style = if k == form.keeper {
                    Style::new()
                        .fg(t.title)
                        .bg(if here { t.focus } else { t.row })
                        .bold()
                } else {
                    Style::new().fg(t.dim)
                };
                spans.push(Span::styled(format!(" {} ", keeper.name()), style));
                spans.push(Span::raw(" "));
            }
            if form.keepers.is_empty() {
                spans.push(Span::styled("nowhere here", Style::new().fg(t.error)));
            }
            lines.push(Line::from(spans));
            continue;
        }
        let value = if [PASSWORD, PASSPHRASE, AGAIN].contains(&i) {
            "•".repeat(form.fields[i].chars().count())
        } else {
            form.fields[i].clone()
        };
        let mut value = tail(&value, value_w.saturating_sub(1));
        if here {
            value.push('▏');
        }
        let pad = value_w.saturating_sub(value.width());
        lines.push(Line::from(vec![
            label,
            Span::styled(
                format!(" {value}{}", " ".repeat(pad)),
                Style::new()
                    .fg(t.text)
                    .bg(if here { t.field } else { t.page }),
            ),
        ]));
    }
    lines.push(Line::raw(""));
    let note_style = if form.failed.is_some() {
        Style::new().fg(t.error)
    } else {
        Style::new().fg(t.soft)
    };
    // How to make an app password, where the provider takes one: said
    // with the key that opens its page.
    let help = config::app_password_help(&form.fields[EMAIL], &form.fields[SERVER])
        .filter(|_| form.auth == Auth::Password)
        .map(|(note, _)| format!("{note} ^o opens the page."));
    let guiding = form.failed.is_none() && !form.checking;
    if let Some(note) = help.as_ref().filter(|_| guiding).or(form.note.as_ref()) {
        lines.extend(
            wrap(note, room)
                .into_iter()
                .map(|l| Line::styled(l, note_style)),
        );
    }
    if let Some(w) = &form.waiting {
        lines.push(Line::styled(
            "Sign in on Microsoft's page, opened in your browser.",
            Style::new().fg(t.text),
        ));
        if let Some((page, code)) = &w.device {
            lines.extend(
                wrap(&format!("Or, on any device, open {page} and enter"), room)
                    .into_iter()
                    .map(|l| Line::styled(l, Style::new().fg(t.text))),
            );
            lines.push(Line::styled(
                format!("    {code}"),
                Style::new().fg(t.accent).bold(),
            ));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "If the browser did not open, open this there:",
            Style::new().fg(t.faint),
        ));
        lines.extend(
            wrap(&w.page, room)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(t.faint))),
        );
    } else if let Some(help) = help.filter(|_| form.failed == Some(Failure::Refused)) {
        // Refused, why takes the help's place above: it is said again here,
        // where what is kept is said otherwise.
        lines.extend(
            wrap(&help, room)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(t.soft))),
        );
    } else {
        let secret = match form.auth {
            Auth::Password => "The password",
            Auth::Microsoft => "Microsoft's token",
        };
        let mut about = format!(
            "It signs in before it is added. {secret} is kept {}, never in the accounts file.",
            form.kept().map_or("nowhere", Keeper::about)
        );
        if shown.contains(&AGAIN) {
            about.push_str(&format!(
                " The vault is new, at {}: its passphrase is asked for each time letterbox \
                 starts.",
                tilde(&vault::path())
            ));
        }
        lines.extend(
            wrap(&about, room)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(t.faint))),
        );
    }
    lines.push(Line::raw(""));
    let (row, places) = if form.checking {
        buttons(t, &[("Cancel", Button::Cancel)])
    } else {
        buttons(
            t,
            &[("Sign in and add", Button::Add), ("Cancel", Button::Cancel)],
        )
    };
    let buttons_at = lines.len();
    lines.push(row);
    let foot = if form.waiting.is_some() {
        "waiting for Microsoft… · esc stops"
    } else if form.checking {
        "signing in…"
    } else {
        "tab next field · ←→ pick · ⏎ on the last adds · esc cancel"
    };
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, "Add account", foot)),
        area,
    );
    let mut targets: Targets = shown
        .iter()
        .enumerate()
        .map(|(line, &i)| (spot(area, 1 + line, 0, room as u16), Target::Field(i)))
        .collect();
    targets.extend(
        places
            .into_iter()
            .map(|(x, w, b)| (spot(area, buttons_at, x, w), Target::Button(b))),
    );
    (area, targets)
}

/// The message's attachments and links: each on a line, the one picked
/// lit, with what ⏎ does with it.
fn files_dialog(f: &mut Frame, t: &Theme, files: &Files) -> (Rect, Targets) {
    let width = f.area().width.min(96);
    let room = (width as usize).saturating_sub(4);
    let side = 22.min(room / 3);
    let mut lines = vec![Line::raw("")];
    for (i, item) in files.items.iter().enumerate() {
        let (icon, name, what) = match item {
            Item::Page => ("◳", "this message".to_string(), "browser".to_string()),
            Item::Present => ("◳", "this message".to_string(), "ohp".to_string()),
            Item::File {
                name,
                mime,
                size: bytes,
                ..
            } => {
                let kind = mime.rsplit('/').next().unwrap_or(mime).to_uppercase();
                let how = if ohp_reads(name) { "ohp" } else { "open" };
                (
                    "▤",
                    name.clone(),
                    format!("{kind} · {} · {how}", size(*bytes)),
                )
            }
            Item::Link(url) => {
                let what = match crate::net::pdf_of(url) {
                    Some(_) => "PDF · ohp",
                    None => "browser",
                };
                ("↗", url.clone(), what.to_string())
            }
        };
        let here = i == files.at;
        let name_room = room.saturating_sub(side + 4);
        let name = clip(&name, name_room);
        let pad = name_room.saturating_sub(name.width());
        let style = if here {
            Style::new().bg(t.focus)
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::styled(format!(" {icon} "), Style::new().fg(t.dim)),
                Span::styled(name, Style::new().fg(t.text)),
                Span::raw(" ".repeat(pad + 1)),
                Span::styled(
                    format!("{:>side$}", clip(&what, side)),
                    Style::new().fg(t.dim),
                ),
            ])
            .style(style),
        );
    }
    if let Some(note) = &files.note {
        lines.push(Line::raw(""));
        lines.extend(
            wrap(note, room)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(t.soft))),
        );
    }
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(
            t,
            "Open",
            "⏎ open · s save · S save all · y copy address · esc close",
        )),
        area,
    );
    let targets = (0..files.items.len())
        .map(|i| (spot(area, 1 + i, 0, room as u16), Target::Item(i)))
        .collect();
    (area, targets)
}

/// The papers of the message read, two lines each: its title and where it
/// stands with paperstack, then who wrote it where. A paper clicked is put
/// in paperstack.
fn papers_dialog(f: &mut Frame, t: &Theme, p: &crate::app::PapersView) -> (Rect, Targets) {
    use crate::paperstack::Standing;
    let width = f.area().width.min(100);
    let room = (width as usize).saturating_sub(4);
    let side = 30.min(room / 3);
    let mut lines = vec![Line::raw("")];
    let first = lines.len();
    for (i, paper) in p.papers.iter().enumerate() {
        let ground = if i == p.at { t.focus } else { t.page };
        let (said, style) = match p.standing.get(i).and_then(Option::as_ref) {
            None => ("…".to_string(), Style::new().fg(t.dim)),
            Some(Standing::Have { starred: true, .. }) => (
                "★ in your library".to_string(),
                Style::new().fg(t.accent).bold(),
            ),
            Some(Standing::Have { .. }) => {
                ("in your library".to_string(), Style::new().fg(t.accent))
            }
            Some(Standing::Like(title)) => (
                format!("like: {}", clip(title, side.saturating_sub(6))),
                Style::new().fg(t.title),
            ),
            Some(Standing::New) => ("new".to_string(), Style::new().fg(t.soft)),
        };
        let title_room = room.saturating_sub(side + 2);
        let title = clip(&paper.title, title_room);
        let pad = title_room.saturating_sub(title.width());
        lines.push(
            Line::from(vec![
                Span::styled(format!(" {title}"), Style::new().fg(t.text)),
                Span::raw(" ".repeat(pad + 1)),
                Span::styled(format!("{said:>side$}"), style),
            ])
            .style(Style::new().bg(ground)),
        );
        lines.push(
            Line::styled(
                clip(&format!("   {}", paper.about), room),
                Style::new().fg(t.dim),
            )
            .style(Style::new().bg(ground)),
        );
    }
    if let Some(note) = &p.note {
        lines.push(Line::raw(""));
        lines.extend(
            wrap(note, room)
                .into_iter()
                .map(|l| Line::styled(format!(" {l}"), Style::new().fg(t.soft))),
        );
    }
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(
            t,
            "Papers in it",
            "⏎ put in paperstack, starred · o open · esc close",
        )),
        area,
    );
    let targets = (0..p.papers.len())
        .map(|i| {
            let mut r = spot(area, first + 2 * i, 0, room as u16);
            r.height = 2;
            (r, Target::Item(i))
        })
        .collect();
    (area, targets)
}

/// What was thrown away on its own, two lines each: how sure, who, what
/// about, and why; then what was learnt since, and how it does now. A
/// message clicked is put back.
fn debrief_dialog(f: &mut Frame, t: &Theme, d: &crate::filter::Debrief) -> (Rect, Targets) {
    let width = f.area().width.min(96);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![
        Line::raw(""),
        Line::styled(
            format!(" Thrown away on its own: {}", d.thrown.len()),
            Style::new().fg(t.title).bold(),
        ),
        Line::raw(""),
    ];
    let first = lines.len();
    for (i, th) in d.thrown.iter().enumerate() {
        let here = i == d.at;
        let ground = if here { t.focus } else { t.page };
        let who = clip(&th.from, 24);
        let head = format!(" {:>3.0}%  {who:<24}  ", th.odds * 100.0);
        let said = room.saturating_sub(head.width() + 10);
        let mut first_line = vec![
            Span::styled(head, Style::new().fg(t.soft)),
            Span::styled(clip(&th.subject, said), Style::new().fg(t.text)),
        ];
        if th.back {
            first_line.push(Span::styled("  put back", Style::new().fg(t.accent).bold()));
        }
        lines.push(Line::from(first_line).style(Style::new().bg(ground)));
        lines.push(
            Line::styled(
                clip(&format!("        {}", th.why.join(" · ")), room),
                Style::new().fg(t.dim),
            )
            .style(Style::new().bg(ground)),
        );
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            " Learnt since: {} of your decisions ({} kept, {} thrown away)",
            d.decided, d.kept, d.thrown_by_you
        ),
        Style::new().fg(t.soft),
    ));
    if let Some(g) = d.grade {
        let pct = |a: usize, b: usize| {
            if b == 0 {
                0.0
            } else {
                100.0 * a as f32 / b as f32
            }
        };
        lines.push(Line::styled(
            format!(
                " At 95% sure, it catches {:.0}% of what you throw away, and would throw away {:.1}% of what you keep",
                pct(g.caught, g.caught + g.missed),
                pct(g.wrong, g.wrong + g.right)
            ),
            Style::new().fg(t.soft),
        ));
    }
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, &d.title, "⏎ put back · u put all back · esc ok")),
        area,
    );
    let targets = (0..d.thrown.len())
        .map(|i| {
            let mut r = spot(area, first + 2 * i, 0, room as u16);
            r.height = 2;
            (r, Target::Item(i))
        })
        .collect();
    (area, targets)
}

/// What the map shows, and how, each by the key that changes it, with what
/// it is now; a row clicked is its key pressed.
fn view_dialog(f: &mut Frame, t: &Theme, map: &crate::map::Map) -> (Rect, Targets) {
    use crate::keymap::Act;
    let on = |b: bool| if b { "on" } else { "off" };
    let shown = |hidden: bool| if hidden { "hidden" } else { "shown" };
    let mut lines = vec![Line::raw("")];
    for &(key, act, what) in keymap::VIEW {
        let now = match act {
            Act::Lens => map.lens.name(),
            Act::Since => map.since.name(),
            Act::ShowRead => shown(map.hide_read),
            Act::ShowSent => shown(map.hide_sent),
            Act::Unfiled => {
                if map.unfiled_only {
                    "only it"
                } else {
                    "with the rest"
                }
            }
            Act::NewOnly => on(map.new_only),
            _ => "",
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {key}  "), Style::new().fg(t.accent).bold()),
            Span::styled(format!("{what:<18}"), Style::new().fg(t.text)),
            Span::styled(now, Style::new().fg(t.dim)),
        ]));
    }
    let width = 52.min(f.area().width);
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, "View", "letters change it · esc closes")),
        area,
    );
    let room = width.saturating_sub(4);
    let targets = (0..keymap::VIEW.len())
        .map(|i| (spot(area, 1 + i, 0, room), Target::Item(i)))
        .collect();
    (area, targets)
}

/// Files to attach, picked a folder at a time: the folder, what is typed,
/// and its entries, the one picked lit, those marked `●`, the newest `★`.
fn attach_dialog(f: &mut Frame, t: &Theme, p: &crate::pick::Picker) -> (Rect, Targets) {
    use crate::pick::Entry;
    let width = f.area().width.min(84);
    let height = f.area().height.saturating_sub(4).clamp(8, 30);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![
        Line::styled(
            clip(&format!("{}/", crate::compose::named(&p.dir)), room),
            Style::new().fg(t.dim),
        ),
        Line::from(vec![
            Span::styled("narrow ", Style::new().fg(t.dim)),
            Span::styled(
                tail(&p.filter, room.saturating_sub(9)),
                Style::new().fg(t.text),
            ),
            Span::styled("▏", Style::new().fg(t.accent)),
        ]),
        Line::styled(
            clip(
                "type to narrow, or a path from /; ␣ marks several to attach",
                room,
            ),
            Style::new().fg(t.faint),
        ),
        Line::raw(""),
    ];
    let top = lines.len();
    let rows = (height as usize).saturating_sub(2 + top + 2).max(1);
    let shown = p.shown();
    let offset = (p.row + 1).saturating_sub(rows);
    for (i, e) in shown.iter().enumerate().skip(offset).take(rows) {
        let (mark, name, about, style) = match e {
            Entry::Up => (
                "  ",
                "..".to_string(),
                String::new(),
                Style::new().fg(t.dim),
            ),
            Entry::Dir(n) => (
                "  ",
                format!("{n}/"),
                String::new(),
                Style::new().fg(t.title),
            ),
            Entry::File { name, size: bytes } => {
                let mark = if p.marked.contains(&p.dir.join(name)) {
                    "● "
                } else if p.newest.as_deref() == Some(name) {
                    "★ "
                } else {
                    "  "
                };
                (
                    mark,
                    name.clone(),
                    size(*bytes as usize),
                    Style::new().fg(t.text),
                )
            }
        };
        let side = about.width();
        let name = clip(&name, room.saturating_sub(side + 4));
        let pad = room.saturating_sub(2 + name.width() + side);
        let line = Line::from(vec![
            Span::styled(mark, Style::new().fg(t.accent)),
            Span::styled(name, style),
            Span::raw(" ".repeat(pad)),
            Span::styled(about, Style::new().fg(t.dim)),
        ]);
        lines.push(if i == p.row {
            line.style(Style::new().bg(t.focus))
        } else {
            line
        });
    }
    while lines.len() < top + rows {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(""));
    let note = match (&p.note, p.marked.len()) {
        (Some(note), _) => Line::styled(clip(note, room), Style::new().fg(t.error)),
        (None, 0) => Line::raw(""),
        (None, n) => Line::styled(
            format!("{n} marked: ⏎ attaches them"),
            Style::new().fg(t.soft),
        ),
    };
    lines.push(note);
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(
            t,
            "Attach files",
            "⏎ attach · ␣ mark · C-a all · ← up · ~ home · esc",
        )),
        area,
    );
    let targets = (offset..shown.len().min(offset + rows))
        .map(|i| {
            (
                spot(area, top + i - offset, 0, room as u16),
                Target::Item(i),
            )
        })
        .collect();
    (area, targets)
}

/// The account's folders, one highlighted, asking which of them is Trash.
fn trash_dialog(f: &mut Frame, t: &Theme, pick: &TrashPick) -> (Rect, Targets) {
    let width = f.area().width.min(72);
    let room = (width as usize).saturating_sub(4);
    let intro = wrap(
        "Which of these folders should hold mail you delete? It stays there for 7 days, and d puts it back.",
        room,
    );
    let shows = (f.area().height as usize)
        .saturating_sub(intro.len() + 4)
        .max(1)
        .min(pick.folders.len());
    let start = pick
        .at
        .saturating_sub(shows / 2)
        .min(pick.folders.len().saturating_sub(shows));
    let mut lines: Vec<Line> = intro.into_iter().map(Line::raw).collect();
    lines.push(Line::raw(""));
    let first = lines.len();
    for (i, (_, name)) in pick.folders.iter().enumerate().skip(start).take(shows) {
        let style = if i == pick.at {
            Style::new().bg(t.focus)
        } else {
            Style::new()
        };
        lines.push(Line::styled(
            format!(" {} ", clip(name, room.saturating_sub(2))),
            style,
        ));
    }
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, "Trash", "⏎ this folder · esc cancel")),
        area,
    );
    let targets = (start..start + shows)
        .map(|i| {
            (
                spot(area, first + i - start, 0, room as u16),
                Target::Item(i),
            )
        })
        .collect();
    (area, targets)
}

/// A question about an account, in a box of its own: what is asked, and
/// `last`, a line to answer on, under it.
fn question(
    f: &mut Frame,
    t: &Theme,
    title: &str,
    foot: &str,
    asked: &str,
    last: impl FnOnce(usize) -> Line<'static>,
) -> (Rect, usize) {
    let width = f.area().width.min(64);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![Line::raw("")];
    lines.extend(wrap(asked, room).into_iter().map(Line::raw));
    lines.push(Line::raw(""));
    let last_at = lines.len();
    lines.push(last(room));
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(dialog(t, title, foot)), area);
    (area, last_at)
}

/// A question answered by a name typed: what is asked, and the field.
fn asking(
    f: &mut Frame,
    t: &Theme,
    title: &str,
    foot: &str,
    asked: &str,
    typed: &str,
) -> (Rect, Targets) {
    let field = Style::new().fg(t.text).bg(t.field);
    let (area, _) = question(f, t, title, foot, asked, |room| {
        let value = tail(typed, room.saturating_sub(1));
        let pad = " ".repeat(room.saturating_sub(value.width() + 1));
        Line::from(Span::styled(format!("{value}▏{pad}"), field))
    });
    (area, Vec::new())
}

fn renaming(f: &mut Frame, t: &Theme, name: &str, typed: &str) -> (Rect, Targets) {
    let field = Style::new().fg(t.text).bg(t.field);
    let (area, _) = question(
        f,
        t,
        "Rename account",
        "⏎ renames · esc keeps it",
        &format!("What is {name} to be called? Only letterbox's name for it changes."),
        |room| {
            let value = tail(typed, room.saturating_sub(1));
            let pad = " ".repeat(room.saturating_sub(value.width() + 1));
            Line::from(Span::styled(format!("{value}▏{pad}"), field))
        },
    );
    (area, Vec::new())
}

fn removing(f: &mut Frame, t: &Theme, name: &str) -> (Rect, Targets) {
    let (row, places) = buttons(t, &[("Remove", Button::Remove), ("Keep", Button::Keep)]);
    let (area, buttons_at) = question(
        f,
        t,
        "Remove account",
        "y removes · any other key keeps it",
        &format!(
            "Remove {name} from letterbox? Its mail stays on its server; the password \
             letterbox kept for it is forgotten."
        ),
        |_| row,
    );
    let targets = places
        .into_iter()
        .map(|(x, w, b)| (spot(area, buttons_at, x, w), Target::Button(b)))
        .collect();
    (area, targets)
}

/// Whether to do what was chosen in the Inbox looked through, `said`.
fn applying(f: &mut Frame, t: &Theme, said: &str) -> (Rect, Targets) {
    let (row, places) = buttons(t, &[("Do it", Button::Apply), ("Not yet", Button::NotYet)]);
    let (area, buttons_at) = question(
        f,
        t,
        "Do what was chosen?",
        "y does it · n not yet",
        said,
        |_| row,
    );
    let targets = places
        .into_iter()
        .map(|(x, w, b)| (spot(area, buttons_at, x, w), Target::Button(b)))
        .collect();
    (area, targets)
}

/// The message being written, over the screen, faded beneath it: as wide
/// as two thirds of it, a letter's lines at least, and as tall as it.
fn compose_popup(f: &mut Frame, app: &mut App, main: Rect) {
    let t = app.theme;
    let buf = f.buffer_mut();
    for y in main.top()..main.bottom() {
        for x in main.left()..main.right() {
            buf[(x, y)].set_fg(t.faint);
        }
    }
    // Narrow, it is the screen's, with no border to take its room.
    if main.width < MEDIUM + 4 {
        f.render_widget(Clear, main);
        return compose_pane(f, app, main);
    }
    let area = {
        let width = (main.width * 2 / 3).max(84).min(main.width - 4);
        let height = main.height.saturating_sub(2).max(3);
        Rect {
            x: main.x + (main.width - width) / 2,
            y: main.y + (main.height - height) / 2,
            width,
            height,
        }
    };
    let from = app
        .compose
        .as_ref()
        .and_then(|c| app.accounts.get(c.account))
        .map(|a| a.email.clone())
        .unwrap_or_default();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.dim))
        .title(Span::styled(
            format!(" From {from} "),
            Style::new().fg(t.title).bold(),
        ))
        .style(Style::new().bg(t.page).fg(t.text));
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);
    app.targets.push((area, Target::Popup));
    // What ends it, as buttons on its top edge, each in a colour of its
    // own, at the right: clicked, or pressed by its keys.
    let buttons = [
        ("Send", "C-c C-c", t.accent, t.on_accent),
        ("Attach", "C-c C-a", t.focus, t.title),
        ("Discard", "C-c C-k", t.error, t.on_accent),
    ];
    let said: Vec<String> = buttons
        .iter()
        .map(|(what, key, ..)| format!(" {what}  {key} "))
        .collect();
    let width: u16 = said.iter().map(|s| s.width() as u16 + 1).sum();
    let title_end = area.x + 2 + format!(" From {from} ").width() as u16;
    if area.right() >= title_end + width + 3 {
        let mut x = area.right() - 2 - width;
        let buf = f.buffer_mut();
        for ((_, key, bg, fg), said) in buttons.iter().zip(&said) {
            let w = said.width() as u16;
            buf.set_stringn(
                x,
                area.y,
                said,
                usize::from(w),
                Style::new().fg(*fg).bg(*bg).bold(),
            );
            app.targets.extend(hint_spots(key, x, area.y, w));
            x += w + 1;
        }
    }
    compose_pane(f, app, inner);
}

/// What an `Attach` line's file is: its size, or why it cannot be sent.
fn attached(value: &str, forwarded: &[crate::compose::File]) -> Result<String, &'static str> {
    if let Some(name) = value.strip_suffix(crate::compose::FORWARDED) {
        return forwarded
            .iter()
            .find(|f| f.name == name)
            .map(|f| size(f.bytes.len()))
            .ok_or("not among those forwarded");
    }
    match std::fs::metadata(crate::compose::path_of(value)) {
        Ok(m) if m.is_file() => Ok(size(m.len() as usize)),
        Ok(_) => Err("not a file"),
        Err(_) => Err("not found"),
    }
}

/// The message being written: its text, the cursor reversed, and a mode
/// line, as an Emacs buffer has one. Each file attached says its size.
fn compose_pane(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(compose) = &mut app.compose else {
        return;
    };
    let t = app.theme;
    let mode_h = 1u16.min(area.height);
    // Two lines of how it goes, where there is room for them.
    let guide_h = if area.height >= 14 { 2 } else { 0 };
    let body = Rect {
        height: area.height.saturating_sub(mode_h + guide_h),
        ..area
    };
    let guide = Rect {
        y: area.y + body.height,
        height: guide_h,
        ..area
    };
    let mode = Rect {
        y: area.y + body.height + guide_h,
        height: area.height - body.height - guide_h,
        ..area
    };
    // Each key on it is clicked as it is pressed.
    let mut spots: Targets = Vec::new();
    if guide_h > 0 {
        let say = Style::new().fg(t.dim);
        let mut first = vec![Span::raw("  ")];
        first.extend(keyed(
            &t,
            &[
                ("C-c C-c", "send"),
                ("C-c C-a", "attach"),
                ("C-c C-k", "discard"),
                ("C-g", "stops, never losing it"),
            ],
            guide.x + 2,
            guide.y,
            &mut spots,
        ));
        let second = "  C-c is Ctrl+c, so C-c C-c is Ctrl+c twice. The arrows move. F1: every key.";
        if let (Some(at), Some(f1)) = (second.find("F1"), keymap::key_named("F1")) {
            let x = guide.x + second[..at].width() as u16;
            spots.push((Rect::new(x, guide.y + 1, 14, 1), Target::Key(f1)));
        }
        let lines = vec![Line::from(first), Line::styled(second, say)];
        f.render_widget(Paragraph::new(lines).style(Style::new().bg(t.side)), guide);
    }
    let rows = (body.height as usize).max(1);
    if compose.buffer.row() < compose.scroll {
        compose.scroll = compose.buffer.row();
    } else if compose.buffer.row() >= compose.scroll + rows {
        compose.scroll = compose.buffer.row() + 1 - rows;
    }
    let width = (body.width as usize).max(1);
    let scroll = compose.scroll;
    let head = compose
        .buffer
        .lines()
        .iter()
        .position(String::is_empty)
        .unwrap_or(0);
    let cursor_row = compose.buffer.row();
    let cursor_col = compose.buffer.col();
    let mut lines: Vec<Line> = compose
        .buffer
        .lines()
        .iter()
        .enumerate()
        .skip(scroll)
        .take(rows)
        .map(|(i, line)| {
            let region = compose.buffer.marked(i);
            let mut shown = compose_line(line, i == cursor_row, cursor_col, width, t, region);
            if i < head
                && let Some(value) = line.strip_prefix("Attach:").map(str::trim)
                && !value.is_empty()
            {
                shown.spans.push(match attached(value, &compose.files) {
                    Ok(size) => Span::styled(format!("  {size}"), Style::new().fg(t.dim)),
                    Err(why) => Span::styled(format!("  {why}"), Style::new().fg(t.error)),
                });
            }
            shown
        })
        .collect();
    while (lines.len() as u16) < body.height {
        lines.push(Line::raw(""));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::new().bg(t.page).fg(t.text)),
        body,
    );
    // Searching, the mode line says what is searched for, as Emacs's
    // echo area does.
    let label = " Compose ";
    let mut spans = vec![Span::styled(
        label,
        Style::new().fg(t.side).bg(t.title).bold(),
    )];
    let x = mode.x + label.width() as u16 + 2;
    spans.push(Span::raw("  "));
    match compose.buffer.searching() {
        _ if compose.asking => {
            let asked = "Message modified; kill anyway? (y or n)   ";
            spans.push(Span::styled(asked, Style::new().fg(t.title).bold()));
            spans.extend(keyed(
                &t,
                &[("y", "discard it"), ("n", "go on writing")],
                x + asked.width() as u16,
                mode.y,
                &mut spots,
            ));
        }
        Some(s) => spans.push(Span::styled(s, Style::new().fg(t.title).bold())),
        None => spans.extend(keyed(
            &t,
            &[
                ("C-SPC", "mark"),
                ("C-w", "cut"),
                ("C-y", "yank"),
                ("C-s", "search"),
                ("F1", "all keys"),
            ],
            x,
            mode.y,
            &mut spots,
        )),
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(t.side)),
        mode,
    );
    app.targets.extend(spots);
}

/// A line of the message being written: the cursor reversed, and the
/// columns in `region` marked as the region, its end too when it goes on.
fn compose_line(
    line: &str,
    cursor: bool,
    col: usize,
    width: usize,
    t: Theme,
    region: std::ops::Range<usize>,
) -> Line<'static> {
    let chars: Vec<char> = line.chars().collect();
    let widths: Vec<usize> = chars
        .iter()
        .map(|c| c.width().unwrap_or(0).max(1))
        .collect();
    let col = col.min(chars.len());
    let cursor_x: usize = widths.iter().take(col).sum();
    let mut start = 0;
    let mut start_x = 0;
    if cursor && width > 0 && cursor_x >= width {
        while cursor_x.saturating_sub(start_x) >= width && start < chars.len() {
            start_x += widths[start];
            start += 1;
        }
    }
    let quoted = line.trim_start().starts_with('>');
    let mut spans = Vec::new();
    let mut x = 0;
    let mut i = start;
    while i < chars.len() && x + widths[i] <= width {
        let style = if cursor && i == col {
            Style::new().add_modifier(Modifier::REVERSED)
        } else if region.contains(&i) {
            Style::new().fg(t.title).bg(t.focus)
        } else if quoted {
            Style::new().fg(t.dim)
        } else {
            Style::new().fg(t.text)
        };
        spans.push(Span::styled(chars[i].to_string(), style));
        x += widths[i];
        i += 1;
    }
    if cursor && col >= chars.len() && x < width {
        spans.push(Span::styled(
            " ",
            Style::new().add_modifier(Modifier::REVERSED),
        ));
    } else if region.end > chars.len() && x < width {
        // The line's end is in the region, as what it cuts takes it too.
        spans.push(Span::styled(" ", Style::new().bg(t.focus)));
    }
    Line::from(spans)
}

fn search(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    f.render_widget(Block::new().style(Style::new().bg(t.side)), area);
    let (name, brand_w) = match area.width {
        w if w >= WIDE => (" ✉ letterbox", SIDEBAR),
        w if w >= 50 => (" ✉ letterbox", 13),
        _ => (" ✉", 3),
    };
    let [brand, field, right] = Layout::horizontal([
        Constraint::Length(brand_w),
        Constraint::Min(10),
        Constraint::Length(if area.width >= MEDIUM { 42 } else { 0 }),
    ])
    .areas(area);
    f.render_widget(
        Paragraph::new(Span::styled(name, Style::new().fg(t.title).bold())),
        brand,
    );
    let folder = app.folder_name();
    let width = field.width.saturating_sub(2).min(64);
    let room = (width as usize).saturating_sub(5);
    let mut spans = vec![Span::raw(" ⌕ ")];
    if app.compose.is_some() {
        spans = vec![Span::styled(" Compose", Style::new().fg(t.title).bold())];
    } else if app.query.is_empty() && !app.searching {
        let what = if app.map.mode == crate::map::Mode::Map && room >= 16 {
            "Search the map".to_string()
        } else if folder.is_empty() || room < 16 {
            "Search".to_string()
        } else {
            format!("Search {folder}")
        };
        spans.push(Span::styled(clip(&what, room), Style::new().fg(t.faint)));
    } else {
        spans.push(Span::styled(
            tail(&app.query, room),
            Style::new().fg(t.text),
        ));
    }
    if app.searching {
        spans.push(Span::styled("▏", Style::new().fg(t.accent)));
    }
    let field = Rect { width, ..field };
    app.targets.push((field, Target::Search));
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(t.field).fg(t.dim)),
        field,
    );
    let count = match &app.listing {
        Some(l) if l.fetching() => "fetching… ".to_string(),
        Some(l) => {
            let unseen = match l.view {
                View::All(role) => app.unseen_of(role),
                View::Folder(..) => app.folder().and_then(|f| f.unseen),
                _ => Some(app.shown.iter().filter(|m| !m.flags.seen).count() as u32),
            };
            let unseen = unseen
                .filter(|&n| n > 0)
                .map(|n| format!(", {n} unread"))
                .unwrap_or_default();
            let shown = if app.query.is_empty() {
                String::new()
            } else {
                format!("{} of ", app.shown.len())
            };
            format!("{shown}{} messages{unseen} ", l.total())
        }
        None => String::new(),
    };
    // The two views, the one shown lit, each to be clicked as its key is
    // pressed.
    let mut x = right.x;
    for (name, key, on) in [
        (" List ", "M-l", app.map.mode == crate::map::Mode::List),
        (" Map ", "M-g", app.map.mode == crate::map::Mode::Map),
    ] {
        let width = (name.width() as u16).min(right.right().saturating_sub(x));
        let tab = Rect { x, width, ..right };
        let style = if on {
            Style::new().fg(t.title).bg(t.focus).bold()
        } else {
            Style::new().fg(t.dim)
        };
        f.render_widget(Paragraph::new(Span::styled(name, style)), tab);
        if width > 0
            && let Some(k) = keymap::key_named(key)
        {
            app.targets.push((tab, Target::Key(k)));
        }
        x += width;
    }
    let rest = Rect {
        x,
        width: right.right().saturating_sub(x),
        ..right
    };
    f.render_widget(
        Paragraph::new(Span::styled(count, Style::new().fg(t.dim))).alignment(Alignment::Right),
        rest,
    );
}

/// Where the items of a list drawn in `area`, from `offset`, each as high
/// as `heights` says, are on the screen, by their place in the list.
fn rows(area: Rect, offset: usize, heights: &[u16]) -> Vec<(usize, Rect)> {
    let mut y = area.y;
    let mut rows = Vec::new();
    for (i, &h) in heights.iter().enumerate().skip(offset) {
        if y >= area.bottom() {
            break;
        }
        let height = h.min(area.bottom() - y);
        rows.push((i, Rect { y, height, ..area }));
        y += height;
    }
    rows
}

fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let width = area.width as usize;
    let listed = app.listing.as_ref().map(|l| &l.view);
    let together = app.rows.contains(&Row::Everyone);
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| match *row {
            Row::Everyone => ListItem::new(vec![
                Line::raw(""),
                Line::styled(
                    " ALL ACCOUNTS",
                    Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
                ),
            ]),
            Row::All(role) => folder_row(
                t,
                width,
                FolderRow {
                    depth: 1,
                    role,
                    name: role.name(),
                    unseen: app.unseen_of(role),
                    selectable: true,
                    open: listed == Some(&View::All(role)),
                },
            ),
            Row::Account(a) => {
                let account = &app.accounts[a];
                // Its colour, to tell its messages by among others'.
                let mut spans = if together {
                    vec![Span::styled(" ●", Style::new().fg(t.account(a)))]
                } else {
                    Vec::new()
                };
                spans.push(Span::styled(
                    format!(
                        " {}",
                        clip(&account.name.to_uppercase(), width.saturating_sub(16))
                    ),
                    Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
                ));
                match &account.state {
                    State::Connecting => {
                        spans.push(Span::styled(" connecting…", Style::new().fg(t.faint)))
                    }
                    State::Failed(_) => {
                        spans.push(Span::styled(" cannot connect", Style::new().fg(t.error)))
                    }
                    State::Ready => {}
                }
                ListItem::new(vec![Line::raw(""), Line::from(spans)])
            }
            Row::Folder(a, i) => {
                let folder = &app.accounts[a].folders[i];
                folder_row(
                    t,
                    width,
                    FolderRow {
                        depth: folder.depth + 1,
                        role: folder.role,
                        name: &folder.name,
                        unseen: folder.unseen,
                        selectable: folder.selectable,
                        open: matches!(listed, Some(View::Folder(x, path))
                            if *x == a && *path == folder.path),
                    },
                )
            }
            Row::Add => ListItem::new(vec![
                Line::raw(""),
                Line::from(vec![
                    Span::styled("  + ", Style::new().fg(t.accent)),
                    Span::styled("Add account", Style::new().fg(t.dim)),
                ]),
            ]),
            Row::MapHead => {
                let mut spans = vec![Span::styled(
                    " MAP",
                    Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
                )];
                if app.map.computing {
                    spans.push(Span::styled(" drawing…", Style::new().fg(t.faint)));
                }
                ListItem::new(vec![Line::raw(""), Line::from(spans)])
            }
            Row::Map => {
                let unread = app
                    .map
                    .snapshot
                    .as_ref()
                    .map_or(0, |s| s.groups.iter().map(|g| g.unread).sum());
                map_row(
                    t,
                    width,
                    Span::styled("▦ ", Style::new().fg(t.faint)),
                    "Every cluster",
                    unread,
                    matches!(listed, Some(View::Map)),
                )
            }
            Row::Cluster(id) => match app.map.snapshot.as_ref().and_then(|s| s.group(id)) {
                Some(g) => map_row(
                    t,
                    width,
                    Span::styled(
                        if g.label.is_some() { "◆ " } else { "◇ " },
                        Style::new().fg(t.hue(g.hue)),
                    ),
                    &g.name(),
                    g.unread,
                    matches!(listed, Some(View::Cluster(x)) if *x == id)
                        || app.map.mode == crate::map::Mode::Map
                            && app.map.focused == crate::map::Focused::Cluster(id),
                ),
                None => ListItem::new(Line::raw("")),
            },
            Row::Loners => {
                let n = app.map.snapshot.as_ref().map_or(0, |s| {
                    s.loners()
                        .iter()
                        .filter(|&&i| !s.dots[i].flags.seen)
                        .count()
                });
                map_row(
                    t,
                    width,
                    Span::styled("○ ", Style::new().fg(t.faint)),
                    "Fits nowhere",
                    n,
                    matches!(listed, Some(View::Loners)),
                )
            }
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.row));
    let picked = if app.focus == Focus::Side {
        Style::new().bg(t.focus)
    } else {
        Style::new().bg(t.row)
    };
    f.render_stateful_widget(
        List::new(items)
            .style(Style::new().bg(t.side))
            .highlight_style(picked),
        area,
        &mut state,
    );
    app.targets.push((area, Target::Side));
    let heights: Vec<u16> = app
        .rows
        .iter()
        .map(|r| {
            if matches!(r, Row::Everyone | Row::Account(_) | Row::Add | Row::MapHead) {
                2
            } else {
                1
            }
        })
        .collect();
    for (i, row) in rows(area, state.offset(), &heights) {
        app.targets.push((row, Target::Row(i)));
    }
}

/// A row of the map's in the sidebar: its icon, its name, and how many of
/// its messages are not read.
fn map_row(
    t: Theme,
    width: usize,
    icon: Span<'static>,
    name: &str,
    unread: usize,
    open: bool,
) -> ListItem<'static> {
    let count = if unread > 0 {
        unread.to_string()
    } else {
        String::new()
    };
    let room = width.saturating_sub(2 + 2 + count.len() + 2);
    let name = clip(name, room);
    let pad = room.saturating_sub(name.width());
    let style = if open {
        Style::new().fg(t.title).bold()
    } else {
        Style::new().fg(t.soft)
    };
    ListItem::new(Line::from(vec![
        Span::raw("  "),
        icon,
        Span::styled(name, style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(count, Style::new().fg(t.dim).bold()),
    ]))
}

/// A folder's row in the sidebar: indented `depth` steps, its role's icon,
/// its name, and how many of its messages are not read.
struct FolderRow<'a> {
    depth: usize,
    role: Role,
    name: &'a str,
    unseen: Option<u32>,
    selectable: bool,
    open: bool,
}

fn folder_row(t: Theme, width: usize, row: FolderRow) -> ListItem<'static> {
    let indent = "  ".repeat(row.depth);
    let count = row
        .unseen
        .filter(|&n| n > 0)
        .map(|n| n.to_string())
        .unwrap_or_default();
    let room = width.saturating_sub(indent.len() + 2 + count.len() + 2);
    let name = clip(row.name, room);
    let pad = room.saturating_sub(name.width());
    let style = if !row.selectable {
        Style::new().fg(t.dim)
    } else if row.open {
        Style::new().fg(t.title).bold()
    } else {
        Style::new().fg(t.soft)
    };
    ListItem::new(Line::from(vec![
        Span::raw(indent),
        Span::styled(format!("{} ", row.role.icon()), Style::new().fg(t.faint)),
        Span::styled(name, style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(count, Style::new().fg(t.dim).bold()),
    ]))
}

fn messages(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let focused = app.focus == Focus::List;
    let block = Block::new()
        .title(Line::styled(
            format!(" {}", app.folder_name()),
            Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(t.page));
    let inner = block.inner(area);
    app.targets.push((area, Target::List));
    if app.shown.is_empty() {
        f.render_widget(block, area);
        empty(f, app, inner);
        return;
    }
    // A column for the mark of the message picked; four lines a message.
    let width = (inner.width as usize).saturating_sub(1);
    let fits = (inner.height / 4).max(1) as usize;
    let picked = app.list.selected().unwrap_or(0);
    let mut top = app.list.offset().min(app.shown.len().saturating_sub(1));
    if picked < top {
        top = picked;
    } else if picked >= top + fits {
        top = picked + 1 - fits;
    }
    let end = (top + fits + 1).min(app.shown.len());
    let now = Zoned::now();
    let rule = Line::styled("─".repeat(width), Style::new().fg(t.rule));
    // Among other accounts' messages, each says its account's name, in
    // its colour.
    let together = app.listing.as_ref().is_some_and(|l| l.mixed());
    let names: Vec<String> = app.accounts.iter().map(|a| clip(&a.name, 12)).collect();
    // What is marked, by where each is: its account, folder and UID.
    let marked: std::collections::HashSet<(String, String, u32)> =
        match (&app.map.region, &app.map.snapshot) {
            (Some(r), Some(s)) => r
                .keys
                .iter()
                .filter_map(|k| s.find(k))
                .flat_map(|i| {
                    let d = &s.dots[i];
                    d.places
                        .iter()
                        .map(|p| (d.key.account.clone(), p.folder.clone(), p.uid))
                        .collect::<Vec<_>>()
                })
                .collect(),
            _ => Default::default(),
        };
    let items: Vec<ListItem> = app.shown[top..end]
        .iter()
        .map(|m| {
            let unseen = !m.flags.seen;
            let is_marked = app.accounts.get(m.account).is_some_and(|a| {
                marked.contains(&(a.ident.clone(), m.folder.clone(), m.summary.uid))
            });
            let date = m.date.map(|d| short_date(d, &now)).unwrap_or_default();
            let mut right = Vec::new();
            if m.flags.flagged {
                right.push(Span::styled("⚑ ", Style::new().fg(t.flag)));
            }
            if m.flags.answered {
                right.push(Span::styled("↩ ", Style::new().fg(t.faint)));
            }
            right.push(Span::styled(format!("{date} "), Style::new().fg(t.dim)));
            let tag = if together {
                Span::styled(
                    format!("{} ", names[m.account]),
                    Style::new().fg(t.account(m.account)),
                )
            } else {
                Span::raw("")
            };
            let right_w: usize =
                right.iter().map(|s| s.content.width()).sum::<usize>() + tag.content.width();
            let from = clip(
                if m.from.is_empty() {
                    "(no sender)"
                } else {
                    &m.from
                },
                width.saturating_sub(right_w + 4),
            );
            let pad = width.saturating_sub(3 + from.width() + right_w);
            let mut first = vec![
                if is_marked {
                    Span::styled(" ▪ ", Style::new().fg(t.on_accent).bg(t.accent).bold())
                } else if unseen {
                    Span::styled(" ● ", Style::new().fg(t.accent))
                } else {
                    Span::raw("   ")
                },
                tag,
                Span::styled(from, Style::new().fg(t.title).bold()),
                Span::raw(" ".repeat(pad)),
            ];
            first.extend(right);
            let subject = if m.subject.is_empty() {
                "(no subject)"
            } else {
                &m.subject
            };
            let subject_style = if unseen {
                Style::new().fg(t.text).bold()
            } else {
                Style::new().fg(t.soft)
            };
            // A subject too long for its line goes on in the preview's.
            let room = width.saturating_sub(4);
            let mut subject = wrap(subject, room).into_iter();
            let head = subject.next().unwrap_or_default();
            let rest: Vec<String> = subject.collect();
            let second = if rest.is_empty() {
                Span::styled(clip(&m.preview, room), Style::new().fg(t.dim))
            } else {
                Span::styled(clip(&rest.join(" "), room), subject_style)
            };
            ListItem::new(vec![
                Line::from(first),
                Line::from(vec![
                    Span::raw("   "),
                    Span::styled(clip(&head, room), subject_style),
                ]),
                Line::from(vec![Span::raw("   "), second]),
                rule.clone(),
            ])
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_symbol(if focused { "▎" } else { " " })
        .highlight_style(Style::new().bg(if focused { t.focus } else { t.row }))
        .style(Style::new().bg(t.page));
    let mut state = ListState::default().with_selected(app.list.selected().map(|s| s - top));
    f.render_stateful_widget(list, area, &mut state);
    *app.list.offset_mut() = top;
    let heights = vec![4; end - top];
    let mut picked_at = None;
    for (at, row) in rows(inner, 0, &heights) {
        if app.list.selected() == Some(top + at) {
            picked_at = Some(row.y + 2);
        }
        app.targets.push((row, Target::Message(top + at)));
    }
    // The labels it may be moved to, dropped down under it.
    if app.menu.is_some()
        && let Some(under) = picked_at
    {
        label_menu(f, app, area, under);
    }
}

/// The map, in the list's place: a legend above, the clusters as a
/// mosaic, or one gone into, its parts as one or its threads as a tree,
/// and how faithful it is below.
fn map_pane(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::map::Focused;
    let t = app.theme;
    f.render_widget(Block::new().style(Style::new().bg(t.page)), area);
    app.targets.push((area, Target::List));
    if area.height == 0 || area.width == 0 {
        return;
    }
    let dim = Style::new().fg(t.dim);
    // Below the map, the message picked, and how faithful it is: a line
    // each, or the second alone where there is little room.
    let foot_h = u16::from(area.height >= 4) + u16::from(area.height >= 10);
    let [legend, canvas, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(foot_h),
    ])
    .areas(area);
    let say = |f: &mut Frame, what: &str| {
        f.render_widget(
            Paragraph::new(vec![Line::raw(""), Line::styled(format!("  {what}"), dim)])
                .wrap(ratatui::widgets::Wrap { trim: false }),
            canvas,
        );
    };
    let now = Timestamp::now().as_second();
    let Some(s) = app.map.snapshot.clone() else {
        f.render_widget(
            Paragraph::new(Span::styled(" Map", dim.add_modifier(Modifier::BOLD))),
            legend,
        );
        let why = match &app.map_off {
            Some(why) => format!("No map: {why}."),
            None if !app.mapping.is_empty() => app.mapping.clone(),
            None => "mapping…".to_string(),
        };
        return say(f, &why);
    };
    let shown = app.map.shown(now);
    // The Inbox looked through: where it is heading lit on the map.
    let heading = app.heading();
    let new: usize = shown
        .iter()
        .filter(|&&i| app.map.is_new(&s.dots[i], now))
        .count();
    let unread = shown.iter().filter(|&&i| !s.dots[i].flags.seen).count();
    let mut head = vec![Span::styled(" Map", dim.add_modifier(Modifier::BOLD))];
    // What is searched for, first, as it is what is looked at.
    let query = app.query.clone();
    if let Some(h) = app.map.hits(&query) {
        head.push(Span::styled(
            format!(" · {} found for “{}”", h.count, query.trim()),
            Style::new().fg(t.title).bold(),
        ));
    }
    let mut what = format!(
        " · {} · {} · {} {}",
        s.lens.name(),
        app.map.since.name(),
        shown.len(),
        if app.map.unfiled_only {
            "not yet filed"
        } else {
            "messages"
        }
    );
    let level = app.map.level(now);
    if let Some((root, _, level)) = &level {
        let (crate::forest::Level::Mosaic(way) | crate::forest::Level::Tree(way)) = level;
        what = format!(" · {}{what}", root.names(way).join(" › "));
    } else {
        what = format!(" · its clusters{what}");
    }
    if unread > 0 {
        what.push_str(&format!(", {unread} unread"));
    }
    if new > 0 {
        what.push_str(&format!(", {new} new"));
    }
    match app.map.focused {
        Focused::Cluster(_) if app.map.tree.is_some() => {}
        Focused::Cluster(id) => {
            if let Some(g) = s.group(id) {
                what.push_str(&format!(" · in {}", g.name()));
            }
        }
        Focused::Loners => what.push_str(" · fitting nowhere"),
        Focused::All => {}
    }
    head.push(Span::styled(what, dim));
    let key = if let Some((_, _, crate::forest::Level::Mosaic(_))) = &level {
        "   ⏎ in  esc out  ▀ unread"
    } else if app.map.tree.is_some() {
        "   h out  l in  j/k along  ⏎ read  esc out  • one  ● more"
    } else if heading.is_some() {
        "   orange where the Inbox goes, the more the brighter"
    } else {
        "   ▦ a block each, as big as it holds, by those alike  ⏎ in  ▀ unread  ◆ label  ◇ cluster"
    };
    let used: usize = head.iter().map(|h| h.content.width()).sum();
    if used + key.width() <= legend.width as usize {
        head.push(Span::styled(key, Style::new().fg(t.faint)));
    }
    f.render_widget(Paragraph::new(Line::from(head)), legend);
    let explained = match s.faithfulness {
        _ if heading.is_some() => heading_said(heading.as_ref().unwrap(), &s),
        Some(_) if app.map.preview.is_some() => {
            " laid out again, the blocks where they would be: ⏎ keeps it, esc drops it".to_string()
        }
        Some(faith) if faith < 0.7 => format!(
            " {:.0}% faithful: M-r lays it out again, so the blocks lie by those alike",
            faith * 100.0
        ),
        Some(faith) => format!(
            " {:.0}% faithful: the blocks lie by those alike",
            faith * 100.0
        ),
        None => String::new(),
    };
    let room = foot.width as usize;
    let mut lines = Vec::new();
    // The message picked, what the keys act on.
    if foot_h > 1 {
        let picked = app.map.picked().map(|i| &s.dots[i]).map(|d| {
            let who = if d.from.is_empty() {
                &d.address
            } else {
                &d.from
            };
            format!(" ▸ {who} — {}", d.subject)
        });
        lines.push(Line::styled(
            clip(&picked.unwrap_or_default(), room),
            Style::new().fg(t.title),
        ));
    }
    if foot_h > 0 {
        lines.push(Line::styled(
            clip(&explained, room),
            Style::new().fg(t.faint),
        ));
        f.render_widget(Paragraph::new(lines), foot);
    }
    if s.at.is_none() {
        return say(f, "laying the map out…");
    }
    if shown.is_empty() {
        return say(
            f,
            if app.map.unfiled_only {
                "Everything here is filed: , f shows the filed mail too."
            } else {
                "Nothing to map here: r shows read mail, t goes further back."
            },
        );
    }
    if canvas.height == 0 || canvas.width < 3 {
        return;
    }
    app.targets.push((canvas, Target::Map));
    if app.map.tree.is_some() {
        return tree_pane(f, app, &s, canvas, now);
    }
    cluster_mosaic(f, app, &s, &shown, canvas, now);
}

/// Where each message lies on the map: as laid out again, while that is
/// shown to be kept or dropped; else as it is.
fn laid_out(app: &App, s: &crate::map::Snapshot) -> Option<Vec<crate::model::Point>> {
    let at = s.at.as_ref()?;
    Some(
        app.map
            .preview
            .clone()
            .filter(|p| p.len() == at.len())
            .unwrap_or_else(|| at.clone()),
    )
}

/// From grey to orange, as `heat` goes from none to the most.
fn heated(t: &Theme, heat: f32) -> ratatui::style::Color {
    crate::theme::mix(t.faint, t.flag, 0.3 + 0.7 * heat)
}

/// Where the Inbox looked through goes, said: to each label, the most
/// first, to the Trash, and kept.
fn heading_said(h: &crate::app::Heading, s: &crate::map::Snapshot) -> String {
    let n = |w: &crate::app::Going| match w.chosen {
        0 => format!("{}", w.all()),
        c => format!("{} ({c} chosen)", w.all()),
    };
    let mut to: Vec<(&u32, &crate::app::Going)> = h.to.iter().collect();
    to.sort_by_key(|(id, w)| (std::cmp::Reverse(w.all()), **id));
    let mut said: Vec<String> = to
        .into_iter()
        .filter_map(|(id, w)| Some(format!("{} {}", s.group(*id)?.name(), n(w))))
        .collect();
    for (what, w) in [
        ("a new label", &h.elsewhere),
        ("the Trash", &h.trash),
        ("kept", &h.kept),
    ] {
        if w.all() > 0 {
            said.push(format!("{what} {}", n(w)));
        }
    }
    format!(" where the Inbox goes: {}", said.join(", "))
}

/// A cluster opened as its tree, from the left: the cluster, its parts,
/// their threads and their messages, each branch a row with its own
/// branches beside it and down. Open are the branch picked and those it is
/// under; the rest are closed, as big as how many they hold.
fn tree_pane(f: &mut Frame, app: &mut App, s: &crate::map::Snapshot, canvas: Rect, now: i64) {
    use crate::forest::Branch;
    let t = app.theme;
    app.map.branches.clear();
    let Some((whole, picked, level)) = app.map.level(now) else {
        return;
    };
    // Its parts, a mosaic of them; below the finest, the threads of the
    // part they are in, as a tree from it.
    let base = match level {
        crate::forest::Level::Mosaic(above) => {
            return parts_mosaic(f, app, s, &whole, &picked, &above, canvas, now);
        }
        crate::forest::Level::Tree(part) => part,
    };
    let Some(root) = whole.at(&base) else {
        return;
    };
    let picked = picked[base.len()..].to_vec();
    let query = app.query.clone();
    let found = app.map.hits(&query).map(|h| h.matched.clone());
    struct Row {
        path: Vec<usize>,
        row: usize,
        open: bool,
    }
    // Each closed branch a row of its own, in turn; an open one on the row
    // of its first.
    fn lay(
        b: &Branch,
        path: Vec<usize>,
        picked: &[usize],
        rows: &mut Vec<Row>,
        next: &mut usize,
    ) -> usize {
        let open = !b.is_leaf() && picked.starts_with(&path);
        let row = if open {
            let mut first = None;
            for (k, c) in b.branches.iter().enumerate() {
                let mut p = path.clone();
                p.push(k);
                let r = lay(c, p, picked, rows, next);
                first.get_or_insert(r);
            }
            first.unwrap_or(*next)
        } else {
            *next += 1;
            *next - 1
        };
        rows.push(Row { path, row, open });
        row
    }
    let mut rows = Vec::new();
    let mut count = 0;
    lay(root, Vec::new(), &picked, &mut rows, &mut count);
    let row_of: std::collections::HashMap<Vec<usize>, usize> =
        rows.iter().map(|r| (r.path.clone(), r.row)).collect();
    // What is seen moves only to keep the branch picked in sight.
    let h = usize::from(canvas.height);
    let at = row_of.get(&picked).copied().unwrap_or(0);
    let mut top = app.map.tree.as_ref().map_or(0, |t| t.top);
    if at < top {
        top = at;
    } else if at >= top + h {
        top = at + 1 - h;
    }
    top = top.min(count.saturating_sub(h));
    if let Some(tree) = &mut app.map.tree {
        tree.top = top;
    }
    // What each says: its mark, its name, how many it holds.
    let said_of = |b: &Branch| {
        let newest = b.newest().unwrap_or(0);
        let d = &s.dots[newest];
        let unread = b
            .messages
            .iter()
            .filter(|&&i| !s.dots[i].flags.seen)
            .count();
        let new = b
            .messages
            .iter()
            .filter(|&&i| app.map.is_new(&s.dots[i], now))
            .count();
        let hits = found
            .as_ref()
            .map(|m| b.messages.iter().filter(|&&i| m[i]).count());
        // No ⬤ here: some terminals draw it wider than a cell, over the name.
        let glyph = match b.messages.len() {
            1 if d.flags.flagged => '⚑',
            1 if d.mine => '▪',
            1 if d.system.sent => '▹',
            1 => '•',
            _ => '●',
        };
        let mut said = format!("{glyph} {}", b.name);
        if !b.is_leaf() {
            said.push_str(&format!(" {}", b.messages.len()));
            match hits {
                Some(n) if n > 0 => said.push_str(&format!(" · {n} found")),
                _ if new > 0 => said.push_str(&format!(" · {new} new")),
                _ if unread > 0 => said.push_str(&format!(" · {unread} unread")),
                _ => {}
            }
        } else if let Some(at) = d.date {
            said.push_str(&format!(" · {}", short_date(at, &Zoned::now())));
        }
        (said, newest, unread, hits)
    };
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    let said: Vec<_> = rows.iter().map(|r| root.at(&r.path).map(said_of)).collect();
    // Each level as wide as what its open branch says, to a point, and a
    // line's room after it.
    let levels = root.levels();
    let mut lefts = vec![u32::from(canvas.x) + 1; levels + 1];
    for d in 0..levels {
        let widest = rows
            .iter()
            .zip(&said)
            .filter(|(r, _)| r.open && r.path.len() == d)
            .filter_map(|(_, w)| w.as_ref().map(|w| w.0.width()))
            .max()
            .unwrap_or(8)
            .clamp(8, 30) as u32;
        lefts[d + 1] = lefts[d] + widest + 4;
    }
    let right = canvas.right();
    let buf = f.buffer_mut();
    let mut spots: Vec<(Rect, usize)> = Vec::new();
    for (r, said) in rows.iter().zip(said) {
        let (Some(b), Some((said, newest, unread, hits))) = (root.at(&r.path), said) else {
            continue;
        };
        if r.row < top || r.row >= top + h {
            continue;
        }
        let y = canvas.y + (r.row - top) as u16;
        let x = lefts[r.path.len()];
        if x >= u32::from(right) {
            continue;
        }
        let x = x as u16;
        let column = (lefts[r.path.len() + 1] - u32::from(x)) as u16;
        let room = if r.open {
            column.saturating_sub(3)
        } else {
            right.saturating_sub(x)
        };
        let said = clip(&said, usize::from(room));
        let width = said.width() as u16;
        let mut colour = t.hue(s.hues[newest]);
        if hits == Some(0) {
            colour = t.rule;
        } else if unread == 0 {
            colour = crate::theme::mix(colour, t.page, 0.4);
        }
        let style = if r.path == picked {
            Style::new().fg(t.title).bg(t.focus).bold()
        } else if hits == Some(0) {
            Style::new().fg(t.dim).bg(t.page)
        } else if picked.starts_with(&r.path) || unread > 0 {
            Style::new().fg(t.title).bg(t.page).bold()
        } else {
            Style::new().fg(t.soft).bg(t.page)
        };
        let mut chars = said.chars();
        if let Some(g) = chars.next() {
            let glyph_style = if r.path == picked {
                style
            } else {
                Style::new().fg(colour).bg(t.page).bold()
            };
            buf[(x, y)].set_char(g).set_style(glyph_style);
        }
        buf.set_stringn(x + 1, y, chars.as_str(), usize::from(width), style);
        spots.push((Rect::new(x, y, width.max(1), 1), app.map.branches.len()));
        app.map.branches.push([base.as_slice(), &r.path].concat());
        if !r.open {
            continue;
        }
        // Its branches, beside it and down: a line from it, and one down
        // to each.
        let line = Style::new().fg(t.rule).bg(t.page);
        let down = x + column - 2;
        if down + 1 >= right {
            continue;
        }
        for x in x + width + 1..down {
            buf[(x, y)].set_char('─').set_style(line);
        }
        let kids: Vec<usize> = (0..b.branches.len())
            .filter_map(|k| {
                let mut p = r.path.clone();
                p.push(k);
                row_of.get(&p).copied()
            })
            .collect();
        let (first, last) = (r.row, kids.last().copied().unwrap_or(r.row));
        for row in first..=last {
            if row < top || row >= top + h {
                continue;
            }
            let y = canvas.y + (row - top) as u16;
            let (bend, on) = match (row == first, kids.contains(&row), row == last) {
                (true, _, true) => ('─', true),
                (true, _, false) => ('┬', true),
                (false, true, true) => ('└', true),
                (false, true, false) => ('├', true),
                (false, false, _) => ('│', false),
            };
            buf[(down, y)].set_char(bend).set_style(line);
            if on {
                buf[(down + 1, y)].set_char('─').set_style(line);
            }
        }
    }
    for (spot, row) in spots {
        app.targets.push((spot, Target::Branch(row)));
    }
}

/// A block of a mosaic: what a click on it is, what it is called and
/// holds, where it lies on the map, and how it is lit.
struct Tile {
    /// Its number among the blocks drawn: a cluster's, or a part's place.
    id: u32,
    target: Target,
    chip: &'static str,
    name: String,
    members: Vec<usize>,
    centre: [f32; 2],
    hue: f32,
    /// Its colour, where not its hue: as much of the Inbox goes there.
    colour: Option<ratatui::style::Color>,
    /// Whether it is in what the map is narrowed to.
    focus: bool,
    /// How much of the Inbox looked through goes there, and whether the
    /// one picked does.
    going: usize,
    bound: bool,
    /// What it says, only what is not yet filed shown.
    unfiled: Option<&'static str>,
}

/// Farther than all of it, the clusters as a mosaic, as paperstack shows
/// its themes: each a block as big as how many of its messages are shown,
/// beside those it lies by on the map.
fn cluster_mosaic(
    f: &mut Frame,
    app: &mut App,
    s: &crate::map::Snapshot,
    shown: &[usize],
    canvas: Rect,
    now: i64,
) {
    use crate::map::Focused;
    let Some(at) = laid_out(app, s) else {
        return;
    };
    let heading = app.heading();
    let mut tiles = Vec::new();
    for g in &s.groups {
        let members: Vec<usize> = g
            .members
            .iter()
            .copied()
            .filter(|i| shown.binary_search(i).is_ok())
            .collect();
        if members.is_empty() {
            continue;
        }
        // Where it lies: the middle of its largest part, a label's mail
        // lying, it may be, in several.
        let core: Vec<usize> = g
            .core
            .iter()
            .copied()
            .filter(|i| shown.binary_search(i).is_ok())
            .collect();
        let middle = if core.is_empty() { &members } else { &core };
        tiles.push(Tile {
            id: g.id,
            target: Target::Anchor(g.id),
            chip: if g.label.is_some() { "◆ " } else { "◇ " },
            name: g.name(),
            centre: middle_of(&at, middle),
            hue: g.hue,
            colour: heading.as_ref().map(|h| match h.heat(g.id) {
                heat if heat > 0.0 => heated(&app.theme, heat),
                _ => app.theme.rule,
            }),
            focus: matches!(app.map.focused, Focused::All)
                || app.map.focused == Focused::Cluster(g.id),
            going: heading
                .as_ref()
                .and_then(|h| h.to.get(&g.id))
                .map_or(0, crate::app::Going::all),
            bound: heading
                .as_ref()
                .is_some_and(|h| h.picked == Some(crate::app::Bound::To(g.id))),
            unfiled: app.map.unfiled_only.then_some(if g.label.is_some() {
                "would go here"
            } else {
                "no label yet"
            }),
            members,
        });
    }
    let picked = app.map.picked().and_then(|i| s.group_of(i)).map(|g| g.id);
    mosaic(f, app, s, canvas, &tiles, picked, None, now);
}

/// A cluster gone into, its parts the branch `above` is split in, as a
/// mosaic, the one the way `picked` goes to picked.
#[allow(clippy::too_many_arguments)]
fn parts_mosaic(
    f: &mut Frame,
    app: &mut App,
    s: &crate::map::Snapshot,
    root: &crate::forest::Branch,
    picked: &[usize],
    above: &[usize],
    canvas: Rect,
    now: i64,
) {
    let (Some(at), Some(parent)) = (laid_out(app, s), root.at(above)) else {
        return;
    };
    let mut tiles = Vec::new();
    for (k, b) in parent.branches.iter().enumerate() {
        let row = app.map.branches.len();
        app.map.branches.push([above, &[k]].concat());
        // Its hue, the mean of its messages' by angle.
        let (sin, cos) = b.messages.iter().fold((0.0, 0.0), |(x, y), &i| {
            let h = s.hues[i].to_radians();
            (x + h.sin(), y + h.cos())
        });
        tiles.push(Tile {
            id: k as u32,
            target: Target::Branch(row),
            chip: "",
            name: b.name.clone(),
            members: b.messages.clone(),
            centre: middle_of(&at, &b.messages),
            hue: sin.atan2(cos).to_degrees().rem_euclid(360.0),
            colour: None,
            focus: true,
            going: 0,
            bound: false,
            unfiled: None,
        });
    }
    let id = app.map.tree.as_ref().map_or(0, |t| t.id);
    let picked = picked.last().map(|&k| k as u32);
    mosaic(
        f,
        app,
        s,
        canvas,
        &tiles,
        picked,
        Some((id, above.to_vec())),
        now,
    );
}

/// The middle of the messages `members` as laid out at `at`.
fn middle_of(at: &[crate::model::Point], members: &[usize]) -> [f32; 2] {
    let k = members.len().max(1) as f32;
    members.iter().fold([0.0, 0.0], |m, &i| {
        [m[0] + at[i][0] / k, m[1] + at[i][1] / k]
    })
}

/// `tiles` as a mosaic, as paperstack shows its themes: each a block,
/// boxed, as big as how many messages it holds and beside those it lies
/// by on the map; each message a half of a cell of its block, bright where
/// it is unread, or found; on each, its name, how many it holds and what
/// is new in it. `picked` is the block picked; `level`, the cluster and
/// the way to the parts shown, if they are a cluster's.
#[allow(clippy::too_many_arguments)]
fn mosaic(
    f: &mut Frame,
    app: &mut App,
    s: &crate::map::Snapshot,
    canvas: Rect,
    tiles: &[Tile],
    picked: Option<u32>,
    level: Option<(u32, Vec<usize>)>,
    now: i64,
) {
    use crate::mosaic::Mosaic;
    use crate::theme::mix;
    let t = app.theme;
    if tiles.is_empty() || canvas.width == 0 || canvas.height == 0 {
        return;
    }
    let query = app.query.clone();
    let found = app.map.hits(&query).map(|h| h.matched.clone());
    let heading = app.heading();
    // What each holds that is unread, new, and found.
    let count = |m: &[usize], keep: &dyn Fn(usize) -> bool| m.iter().filter(|&&i| keep(i)).count();
    let unread: Vec<usize> = tiles
        .iter()
        .map(|b| count(&b.members, &|i| !s.dots[i].flags.seen))
        .collect();
    let new: Vec<usize> = tiles
        .iter()
        .map(|b| count(&b.members, &|i| app.map.is_new(&s.dots[i], now)))
        .collect();
    let hits: Vec<Option<usize>> = tiles
        .iter()
        .map(|b| found.as_ref().map(|m| count(&b.members, &|i| m[i])))
        .collect();
    // What each holds that is marked, to act on all of it.
    let region: std::collections::HashSet<usize> = app
        .map
        .region
        .iter()
        .flat_map(|r| r.keys.iter().filter_map(|k| s.find(k)))
        .collect();
    let marks: Vec<usize> = tiles
        .iter()
        .map(|b| count(&b.members, &|i| region.contains(&i)))
        .collect();
    let lit: Vec<bool> = (0..tiles.len())
        .map(|k| tiles[k].focus && hits[k] != Some(0) && !(app.map.new_only && new[k] == 0))
        .collect();
    // Tiled again only when what is shown, or the canvas, has changed: a
    // message to a half of a cell, as square as a terminal's cells halved.
    let Some(at) = laid_out(app, s) else {
        return;
    };
    let items: Vec<(usize, usize, [f32; 2])> = tiles
        .iter()
        .enumerate()
        .flat_map(|(k, b)| {
            let at = &at;
            b.members.iter().map(move |&i| (i, k, at[i]))
        })
        .collect();
    let tiled = (
        s as *const crate::map::Snapshot as usize,
        items.iter().map(|it| it.0).collect::<Vec<_>>(),
        canvas.width,
        canvas.height,
        level,
        app.map.preview.as_ref().map(|p| p.as_ptr() as usize),
    );
    let mosaic = match &app.map.mosaic {
        Some((was, m)) if *was == tiled => m.clone(),
        _ => {
            let aspect = f32::from(canvas.width) / (2.0 * f32::from(canvas.height));
            let centres: Vec<[f32; 2]> = tiles.iter().map(|b| b.centre).collect();
            let m = Arc::new(Mosaic::new(&items, &centres, aspect));
            app.map.mosaic = Some((tiled, m.clone()));
            m
        }
    };
    // Each half cell's block, a row of them to each half row.
    let (w, h) = (usize::from(canvas.width), 2 * usize::from(canvas.height));
    let (cols, rows) = (mosaic.cols as f32, mosaic.rows as f32);
    let block_of: Vec<Option<usize>> = (0..h)
        .flat_map(|half| (0..w).map(move |col| (col, half)))
        .map(|(col, half)| {
            mosaic.at([
                (col as f32 + 0.5) / w as f32 * cols,
                (half as f32 + 0.5) / h as f32 * rows,
            ])
        })
        .collect();
    let block_at = |col: i64, half: i64| {
        ((0..w as i64).contains(&col) && (0..h as i64).contains(&half))
            .then(|| block_of[half as usize * w + col as usize])
            .flatten()
    };
    // Each message's own tile, read or not: where a cell of the grid
    // spans a few of the screen's, with a gutter at its right and foot;
    // where it spans less, each message unread, or, searching, found, at
    // least the half cell its middle is in.
    let lit_up = |i: usize| found.as_ref().map_or(!s.dots[i].flags.seen, |m| m[i]);
    let mut marked = vec![false; w * h];
    for &(i, p) in &mosaic.cells {
        let col = ((p[0] / cols * w as f32) as usize).min(w - 1);
        let half = ((p[1] / rows * h as f32) as usize).min(h - 1);
        if lit_up(i) {
            marked[half * w + col] = true;
        }
    }
    let grid = |col: i64, half: i64| {
        [
            ((col as f32 + 0.5) / w as f32 * cols).floor(),
            ((half as f32 + 0.5) / h as f32 * rows).floor(),
        ]
    };
    let gutters = w as f32 / cols >= 3.0 && h as f32 / rows >= 3.0;
    // Of a half cell: none, a message read, one lit up, or the one picked.
    let picked_one = app.map.picked();
    let tone = |col: i64, half: i64| -> u8 {
        if marked[half as usize * w + col as usize] {
            return 2;
        }
        let at = grid(col, half);
        let Some(i) = mosaic.held([at[0] + 0.5, at[1] + 0.5]) else {
            return 0;
        };
        if gutters && (grid(col + 1, half)[0] != at[0] || grid(col, half + 1)[1] != at[1]) {
            return 0;
        }
        match () {
            _ if Some(i) == picked_one => 3,
            _ if lit_up(i) => 2,
            _ => 1,
        }
    };
    // Each block's colour: its own, or its hue; grey where it is not lit.
    // Its cells in a faint pastel of it, less faint picked, its edge a box
    // in a paler one, and what is unread, or found, in the colour itself.
    let paper = t.ground();
    let strong: Vec<_> = tiles
        .iter()
        .zip(&lit)
        .map(|(b, &lit)| match b.colour {
            _ if !lit => t.rule,
            Some(c) => c,
            None => t.hue(b.hue),
        })
        .collect();
    let fills: Vec<_> = tiles
        .iter()
        .zip(&strong)
        .map(|(b, &c)| mix(c, paper, if picked == Some(b.id) { 0.6 } else { 0.82 }))
        .collect();
    let reads: Vec<_> = tiles
        .iter()
        .zip(&strong)
        .map(|(b, &c)| mix(c, paper, if picked == Some(b.id) { 0.45 } else { 0.7 }))
        .collect();
    let edges: Vec<_> = strong
        .iter()
        .zip(&marks)
        .map(|(&c, &m)| if m > 0 { t.title } else { mix(c, paper, 0.4) })
        .collect();
    let colour = |col: i64, half: i64| {
        let Some(b) = block_at(col, half) else {
            return t.page;
        };
        let inner = [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .iter()
            .all(|(dx, dy)| block_at(col + dx, half + dy) == Some(b));
        match tone(col, half) {
            _ if !inner => edges[b],
            3 => t.accent,
            2 => strong[b],
            1 => reads[b],
            _ => fills[b],
        }
    };
    // Each block's bounds on the screen, and the runs of it on each row,
    // to be clicked.
    let mut bounds: Vec<Option<[u16; 4]>> = vec![None; tiles.len()];
    let mut runs: Vec<(Rect, usize)> = Vec::new();
    let buf = f.buffer_mut();
    for row in 0..canvas.height {
        let mut run: Option<(u16, u16, usize)> = None;
        for col in 0..canvas.width {
            let (c, half) = (i64::from(col), 2 * i64::from(row));
            let (top, low) = (colour(c, half), colour(c, half + 1));
            let (x, y) = (canvas.x + col, canvas.y + row);
            let cell = &mut buf[(x, y)];
            // A half beyond the blocks as the page, never in the
            // terminal's own colour for text, as a half block would be.
            match (top == t.page, low == t.page) {
                (true, true) => cell.set_symbol(" "),
                (true, false) => cell.set_symbol("▄").set_fg(low),
                _ => cell.set_symbol("▀").set_fg(top),
            };
            cell.set_bg(if top == t.page { t.page } else { low });
            let Some(b) = block_at(c, half + 1).or(block_at(c, half)) else {
                continue;
            };
            let r = bounds[b].get_or_insert([x, x, y, y]);
            *r = [r[0].min(x), r[1].max(x), r[2].min(y), r[3].max(y)];
            match &mut run {
                Some((_, end, rb)) if *rb == b && *end + 1 == x => *end = x,
                _ => {
                    if let Some((a, e, rb)) = run.take() {
                        runs.push((Rect::new(a, y, e - a + 1, 1), rb));
                    }
                    run = Some((x, x, b));
                }
            }
        }
        if let Some((a, e, rb)) = run {
            runs.push((Rect::new(a, canvas.y + row, e - a + 1, 1), rb));
        }
    }
    for (r, b) in runs {
        app.targets.push((r, tiles[b].target));
    }
    app.map.blocks = tiles
        .iter()
        .zip(&bounds)
        .filter_map(|(b, r)| {
            let [x0, x1, y0, y1] = (*r)?;
            let middle = |a: u16, z: u16| (f32::from(a) + f32::from(z) + 1.0) / 2.0;
            Some((
                b.id,
                [middle(x0, x1), middle(y0, y1)],
                f32::from(x1 - x0 + 1) / 2.0,
            ))
        })
        .collect();
    // On each block, inside its box, its name, then how many it holds and
    // what is new in it; on one line where there is not room for two.
    for (k, (b, r)) in tiles.iter().zip(&bounds).enumerate() {
        let Some([x0, x1, y0, y1]) = *r else {
            continue;
        };
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        let width = if bw > 2 { bw - 2 } else { bw };
        if width < 3 {
            continue;
        }
        let (x, y) = (
            if bw > 2 { x0 + 1 } else { x0 },
            if bh > 2 { y0 + 1 } else { y0 },
        );
        let mut said = format!("{}", b.members.len());
        match hits[k] {
            _ if b.going > 0 => said.push_str(&format!(" ← {} of the Inbox", b.going)),
            _ if marks[k] == b.members.len() => said.push_str(" · all marked"),
            _ if marks[k] > 0 => said.push_str(&format!(" · {} marked", marks[k])),
            Some(h) if h > 0 => said.push_str(&format!(" · {h} found")),
            _ if b.unfiled.is_some() => said.push_str(&format!(" · {}", b.unfiled.unwrap_or(""))),
            _ if new[k] > 0 => said.push_str(&format!(" · {} new", new[k])),
            _ if unread[k] > 0 => said.push_str(&format!(" · {} unread", unread[k])),
            _ => {}
        }
        let lines = if y + 1 < y1 {
            vec![format!(" {}{} ", b.chip, b.name), format!(" {said} ")]
        } else {
            vec![format!(" {}{} {said} ", b.chip, b.name)]
        };
        let style = if b.bound {
            // Where the one picked goes.
            Style::new().fg(t.ground()).bg(t.flag).bold()
        } else if picked == Some(b.id) {
            Style::new().fg(t.title).bg(t.focus).bold()
        } else if heading.is_some() && b.going > 0 {
            Style::new().fg(t.flag).bg(fills[k]).bold()
        } else if !lit[k] {
            Style::new().fg(t.dim).bg(fills[k])
        } else if unread[k] > 0 {
            Style::new().fg(t.title).bg(fills[k]).bold()
        } else {
            Style::new().fg(t.soft).bg(fills[k])
        };
        for (row, line) in (y..).zip(&lines) {
            let text = clip(line, usize::from(width));
            let spot = Rect::new(x, row, text.width() as u16, 1);
            f.render_widget(Paragraph::new(Span::styled(text, style)), spot);
            app.targets.push((spot, b.target));
        }
    }
}

/// What an empty list says: how to set up an account, that the account is
/// being reached or could not be, or that there is nothing to show.
fn empty(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let dim = Style::new().fg(t.dim);
    let title = Style::new().fg(t.title).bold();
    let mut lines = vec![Line::raw("")];
    if app.live().next().is_none() {
        lines.push(Line::styled("  No accounts yet.", title));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("  Press ", Style::new().fg(t.text)),
            Span::styled("A", title),
            Span::styled(
                " to add one: your address and password.",
                Style::new().fg(t.text),
            ),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("  They are kept in {}.", tilde(&app.config)),
            dim,
        ));
        lines.push(Line::from(vec![
            Span::styled("  Or try ", dim),
            Span::styled("letterbox --demo", Style::new().fg(t.soft).bold()),
            Span::styled(" first.", dim),
        ]));
    } else if let Some(l) = &app.listing {
        if let Some(e) = l.error() {
            lines.push(Line::styled("  Cannot list this folder:", title));
            lines.extend(e.lines().map(|e| Line::styled(format!("  {e}"), dim)));
        } else if l.loading() {
            lines.push(Line::styled("  fetching…", dim));
        } else if !app.query.is_empty() && app.found.is_none() {
            lines.push(Line::styled("  Nothing listed matches.", title));
            lines.push(Line::styled(
                "  ⏎ searches the whole folder on the server.",
                dim,
            ));
        } else if !app.query.is_empty() {
            lines.push(Line::styled("  Nothing found.", title));
        } else {
            lines.push(Line::styled("  No messages.", dim));
        }
    } else {
        for a in &app.accounts {
            match &a.state {
                State::Connecting => {
                    lines.push(Line::styled(format!("  {}: connecting…", a.name), dim))
                }
                State::Failed(e) => {
                    lines.push(Line::styled(format!("  {} cannot connect:", a.name), title));
                    lines.extend(e.lines().map(|e| Line::styled(format!("  {e}"), dim)));
                    lines.push(Line::raw(""));
                }
                State::Ready => {}
            }
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .style(Style::new().bg(t.page))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

/// The Inbox looked through, beside the map: each message, newest first,
/// with what may be done with it, what will be done lit; and a button that
/// does it all.
fn review_pane(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::app::Choice;
    let t = app.theme;
    let focused = app.focus == Focus::Reader;
    f.render_widget(Block::new().style(Style::new().bg(t.page)), area);
    app.targets.push((area, Target::Reader));
    if area.height < 4 || area.width < 24 {
        return;
    }
    let Some(s) = app.map.snapshot.clone() else {
        return;
    };
    let rows = app.review_rows();
    let picked = app
        .review
        .as_ref()
        .and_then(|r| r.picked.as_ref())
        .and_then(|k| s.find(k));
    let (filed, discarded, kept, _) = app.review_counts();
    let before = rows.iter().filter(|&&i| s.dots[i].looked).count();
    // Its heading: how many, what will be done, and the button that does it.
    let x = area.x + 1;
    let width = area.width.saturating_sub(2);
    let head = if before > 0 {
        format!(
            " Inbox · {} to look through · {before} kept before ",
            rows.len() - before
        )
    } else {
        format!(" Inbox · {} to look through ", rows.len())
    };
    let button = " Apply  $ ";
    let buf = f.buffer_mut();
    buf.set_stringn(
        x,
        area.y,
        &head,
        usize::from(width),
        Style::new().fg(t.title).bold(),
    );
    let right = x + width;
    if right > x + head.width() as u16 + button.width() as u16 {
        let bx = right - button.width() as u16;
        buf.set_string(
            bx,
            area.y,
            button,
            Style::new().fg(t.on_accent).bg(t.accent).bold(),
        );
        if let Some(key) = keymap::key_named("$") {
            app.targets.push((
                Rect::new(bx, area.y, button.width() as u16, 1),
                Target::Key(key),
            ));
        }
    }
    // Only those chosen are done; the rest are left as they are.
    let counts = if filed + discarded + kept == 0 {
        " none chosen: look through as many as you like; $ does only those".to_string()
    } else {
        format!(" {filed} to file · {discarded} to discard · {kept} to keep · the rest left")
    };
    buf.set_stringn(
        x,
        area.y + 1,
        &counts,
        usize::from(width),
        Style::new().fg(t.soft),
    );
    for cx in x..right {
        buf[(cx, area.y + 2)]
            .set_char('─')
            .set_style(Style::new().fg(t.rule));
    }
    if rows.is_empty() {
        buf.set_stringn(
            x,
            area.y + 4,
            " Nothing in an Inbox.",
            usize::from(width),
            Style::new().fg(t.dim),
        );
        return;
    }
    // Two lines a message; what is seen moves to keep the one picked in it.
    let room = usize::from(area.height.saturating_sub(3)) / 2;
    let at = picked
        .and_then(|p| rows.iter().position(|&i| i == p))
        .unwrap_or(0);
    let mut top = app.review.as_ref().map_or(0, |r| r.top);
    if at < top {
        top = at;
    } else if at >= top + room.max(1) {
        top = at + 1 - room.max(1);
    }
    if let Some(r) = &mut app.review {
        r.top = top;
    }
    let now = Zoned::now();
    let mut y = area.y + 3;
    let menu_for = app.menu.as_ref().and_then(|m| s.find(&m.key));
    let mut menu_at = None;
    for (row, &i) in rows.iter().enumerate().skip(top).take(room) {
        if Some(i) == menu_for {
            menu_at = Some(y + 1);
        }
        let d = &s.dots[i];
        let is_picked = Some(i) == picked;
        let ground = if is_picked && focused {
            t.focus
        } else if is_picked {
            t.row
        } else {
            t.page
        };
        for cx in x..right {
            buf[(cx, y)]
                .set_char(' ')
                .set_style(Style::new().bg(ground));
            buf[(cx, y + 1)]
                .set_char(' ')
                .set_style(Style::new().bg(ground));
        }
        let from = if d.from.is_empty() {
            &d.address
        } else {
            &d.from
        };
        let date = d.date.map(|at| short_date(at, &now)).unwrap_or_default();
        let mark = if d.flags.seen { "  " } else { "● " };
        let said = format!("{mark}{} — {}", clip(from, 22), d.subject);
        let room_said = usize::from(width).saturating_sub(date.width() + 2);
        let style = if d.looked {
            Style::new().fg(t.dim).bg(ground)
        } else if d.flags.seen {
            Style::new().fg(t.soft).bg(ground)
        } else {
            Style::new().fg(t.title).bg(ground).bold()
        };
        buf.set_stringn(x, y, clip(&said, room_said), room_said, style);
        buf.set_string(
            right.saturating_sub(date.width() as u16),
            y,
            &date,
            Style::new().fg(t.dim).bg(ground),
        );
        app.targets
            .push((Rect::new(x, y, width, 1), Target::ReviewRow(row)));
        // What may be done with it: what was chosen lit, else what is
        // suggested in its colour.
        let chosen = app.chosen(i);
        let suggested = crate::app::App::suggested(&s, i);
        let shown = chosen.clone().unwrap_or_else(|| suggested.clone());
        let label = match &shown {
            Choice::Move(l) => Some(l.clone()),
            _ => s.suggestion(i).map(str::to_string),
        };
        let options = [
            (
                match &label {
                    Some(l) => format!("→ {}", clip(l, 18)),
                    None => "move to…".to_string(),
                },
                matches!(chosen, Some(Choice::Move(_))),
                t.accent,
                t.on_accent,
                chosen.is_none() && matches!(suggested, Choice::Move(_)),
            ),
            (
                "keep".to_string(),
                chosen == Some(Choice::Keep),
                t.title,
                t.page,
                chosen.is_none() && suggested == Choice::Keep,
            ),
            (
                match s.dots[i].thrown {
                    Some(p) => format!("discard {:.0}%", p * 100.0),
                    None => "discard".to_string(),
                },
                chosen == Some(Choice::Discard),
                t.error,
                t.on_accent,
                chosen.is_none() && suggested == Choice::Discard,
            ),
            ("new…".to_string(), false, t.accent, t.on_accent, false),
        ];
        let mut cx = x + 2;
        for (k, (what, on, bg, fg, hint)) in options.iter().enumerate() {
            let chip = if *on {
                format!(" ✓ {what} ")
            } else if *hint {
                format!(" [?] {what} ")
            } else {
                format!(" [ ] {what} ")
            };
            let w = chip.width() as u16;
            if cx + w > right {
                break;
            }
            let style = if *on {
                Style::new().fg(*fg).bg(*bg).bold()
            } else if *hint {
                Style::new().fg(*bg).bg(ground)
            } else {
                Style::new().fg(t.dim).bg(ground)
            };
            buf.set_string(cx, y + 1, &chip, style);
            app.targets
                .push((Rect::new(cx, y + 1, w, 1), Target::Choose(row, k as u8)));
            cx += w + 1;
        }
        y += 2;
    }
    if let Some(under) = menu_at {
        label_menu(f, app, area, under);
    }
}

/// The labels a message may be moved to, dropped down under it in the
/// list, or under the line of what may be done with it looked through, or over it where there is no room
/// below: what is typed, then each label with how far it is by angle, the
/// one picked lit.
fn label_menu(f: &mut Frame, app: &mut App, area: Rect, under: u16) {
    use crate::app::Pick;
    let t = app.theme;
    let items = app.menu_items();
    let Some(menu) = app.menu.clone() else {
        return;
    };
    let width = area.width.saturating_sub(4).min(40);
    let height = (items.len() as u16 + 3).min(area.height);
    let below = under + 1 + height <= area.bottom();
    let y = if below {
        under + 1
    } else {
        under.saturating_sub(height).max(area.y)
    };
    let x = area.x + 3;
    let rect = Rect::new(x, y, width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.accent))
        .title(Span::styled(" Move to ", Style::new().fg(t.title).bold()))
        .title_bottom(Span::styled(" ⏎ moves · esc ", Style::new().fg(t.dim)))
        .style(Style::new().bg(t.page));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    let room = usize::from(inner.width);
    let mut lines = vec![Line::from(vec![
        Span::styled(" ⌕ ", Style::new().fg(t.dim).bg(t.field)),
        Span::styled(
            format!(
                "{:<w$}",
                format!("{}▏", menu.typed),
                w = room.saturating_sub(3)
            ),
            Style::new().fg(t.text).bg(t.field),
        ),
    ])];
    for (k, item) in items.iter().enumerate() {
        let (said, far) = match item {
            Pick::Label(name, deg) => (format!(" {name}"), format!("{deg:>3.0}° ")),
            Pick::New(name) => (format!(" + new label “{name}”"), String::new()),
            Pick::OnMap => (" ◇ pick on the map…".to_string(), String::new()),
        };
        let said = clip(&said, room.saturating_sub(far.width()));
        let gap = room.saturating_sub(said.width() + far.width());
        let text = format!("{said}{}{far}", " ".repeat(gap));
        let style = if k == menu.at {
            Style::new().fg(t.title).bg(t.focus).bold()
        } else if matches!(item, Pick::Label(..)) {
            Style::new().fg(t.text).bg(t.page)
        } else {
            Style::new().fg(t.soft).bg(t.page)
        };
        lines.push(Line::styled(text, style));
        if 1 + (k as u16) < inner.height {
            app.targets.push((
                Rect::new(inner.x, inner.y + 1 + k as u16, inner.width, 1),
                Target::MenuItem(k),
            ));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// The message read as its pages, if it is shown so: the page shown, as
/// wide as the pane, and which it is below it. Whether it was.
#[cfg(feature = "pages")]
fn paged(f: &mut Frame, app: &mut App, area: Rect) -> bool {
    // Another message's pages, as another is read: let go.
    let place = app.read_place();
    if app
        .paged
        .as_ref()
        .is_some_and(|p| Some(&p.place) != place.as_ref())
    {
        app.paged = None;
    }
    let (Some(paged), Some(picker)) = (
        app.paged.as_mut(),
        app.picker.as_ref().and_then(Option::as_ref),
    ) else {
        return false;
    };
    let t = app.theme;
    let foot = Rect {
        y: area.bottom().saturating_sub(1),
        height: 1.min(area.height),
        ..area
    };
    let page = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let said = format!(
        "page {} of {} · j k turn · V its text",
        paged.at + 1,
        paged.len()
    );
    if let Some(drawn) = paged.drawn(picker) {
        f.render_stateful_widget(ratatui_image::StatefulImage::default(), page, drawn);
    }
    f.render_widget(
        Paragraph::new(Line::styled(said, Style::new().fg(t.dim))),
        foot,
    );
    true
}

fn reader(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(if app.resizing {
            t.accent
        } else if app.focus == Focus::Reader {
            t.dim
        } else {
            t.rule
        }))
        .padding(Padding::horizontal(1))
        .style(Style::new().bg(t.page));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.targets.push((area, Target::Reader));
    #[cfg(feature = "pages")]
    if paged(f, app, inner) {
        return;
    }
    let width = inner.width as usize;
    let dim = Style::new().fg(t.dim);
    let said: Vec<Line<'static>>;
    let lines: &[Line<'static>] = match app.reading.as_ref().map(|r| &r.body) {
        None => {
            said = vec![
                Line::raw(""),
                Line::styled(
                    if app.shown.is_empty() {
                        "No message picked."
                    } else {
                        "⏎ reads the message picked."
                    },
                    dim,
                ),
            ];
            &said
        }
        Some(Body::Loading) => {
            said = vec![Line::raw(""), Line::styled("fetching…", dim)];
            &said
        }
        Some(Body::Failed(e)) => {
            let mut lines = vec![
                Line::raw(""),
                Line::styled("Cannot read this message:", Style::new().fg(t.title).bold()),
            ];
            lines.extend(wrap(e, width).into_iter().map(|l| Line::styled(l, dim)));
            said = lines;
            &said
        }
        Some(Body::Ready(m)) => {
            let drawn = app
                .drawn
                .as_ref()
                .is_some_and(|(d, w, theme, _)| Arc::ptr_eq(d, m) && *w == width && *theme == t);
            if !drawn {
                app.drawn = Some((m.clone(), width, t, message(m, width, &t)));
            }
            let base = &app.drawn.as_ref().expect("drawn above").3;
            match mapped(app, width) {
                Some((head, tail)) => {
                    // The map's rows after the headers, before the rule.
                    let rule = base
                        .iter()
                        .position(|l| l.spans.first().is_some_and(|s| s.content.starts_with('─')))
                        .unwrap_or(base.len());
                    let mut all = base[..rule].to_vec();
                    all.extend(head);
                    all.extend(base[rule..].iter().cloned());
                    all.extend(tail);
                    said = all;
                    &said
                }
                None => base,
            }
        }
    };
    // A message longer than the pane gives its last line to saying how far
    // in it is.
    let longer = lines.len() > inner.height as usize;
    let height = (inner.height as usize).saturating_sub(usize::from(longer));
    app.height = height;
    app.lines = lines.len();
    app.scroll = app.scroll.min(lines.len().saturating_sub(height));
    let shown: Vec<Line> = lines
        .iter()
        .skip(app.scroll)
        .take(height)
        .cloned()
        .collect();
    f.render_widget(
        Paragraph::new(shown),
        Rect {
            height: height as u16,
            ..inner
        },
    );
    if longer && inner.height > 0 {
        let end = app.scroll + height >= app.lines;
        let percent = (app.scroll + height) * 100 / app.lines.max(1);
        let mut at = if end {
            "── end ".to_string()
        } else {
            format!("── {percent}% · space for more ")
        };
        if at.width() > inner.width as usize {
            at = if end {
                "end".into()
            } else {
                format!("{percent}%")
            };
        }
        let rule = "─".repeat((inner.width as usize).saturating_sub(at.width()));
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(at, dim),
                Span::styled(rule, Style::new().fg(t.rule)),
            ])),
            Rect {
                y: inner.bottom() - 1,
                height: 1,
                ..inner
            },
        );
    }
}

/// What the map says of the message read: its labels and where it is, to
/// go with its headers; and the messages nearest it by angle, to go after
/// its text. None when the map has it not.
fn mapped(app: &App, width: usize) -> Option<(Vec<Line<'static>>, Vec<Line<'static>>)> {
    let r = app.reading.as_ref()?;
    let s = app.map.snapshot.as_ref()?;
    let ident = &app.accounts.get(r.account)?.ident;
    let i = s.dots.iter().position(|d| {
        d.key.account == *ident
            && d.places
                .iter()
                .any(|p| p.folder == r.folder && p.uid == r.uid)
    })?;
    let t = app.theme;
    let dim = Style::new().fg(t.dim);
    let d = &s.dots[i];
    let soft = Style::new().fg(t.soft);
    let mut head = Vec::new();
    // A field as the headers are: its name, and its value wrapped under
    // itself, past the names' column, after an icon if it has one.
    let mut field = |name: &str, icon: Option<Span<'static>>, value: &str| {
        let room = width.saturating_sub(LABEL + if icon.is_some() { 2 } else { 0 });
        for (k, l) in wrap(value, room).into_iter().enumerate() {
            let mut spans = vec![Span::styled(
                format!("{:<LABEL$}", if k == 0 { name } else { "" }),
                dim,
            )];
            match (&icon, k) {
                (Some(icon), 0) => spans.push(icon.clone()),
                (Some(_), _) => spans.push(Span::raw("  ")),
                _ => {}
            }
            spans.push(Span::styled(l, soft));
            head.push(Line::from(spans));
        }
    };
    if !d.labels.is_empty() {
        field("Labels", None, &d.labels.join(", "));
    }
    let mut here = match s.group_of(i) {
        _ if s.clustering.loner[i] => "fits nowhere".to_string(),
        Some(g) => format!(
            "{}, with {} others",
            g.name(),
            g.members.len().saturating_sub(1)
        ),
        None => String::new(),
    };
    if d.twin {
        here.push_str("; in another account too");
    }
    let icon = s.group_of(i).filter(|_| !s.clustering.loner[i]).map(|g| {
        Span::styled(
            if g.label.is_some() { "◆ " } else { "◇ " },
            Style::new().fg(t.hue(g.hue)),
        )
    });
    field("Here", icon, &here);
    let now = Zoned::now();
    let mut tail = vec![
        Line::raw(""),
        Line::styled("─".repeat(width), Style::new().fg(t.rule)),
    ];
    // After filing, what would go to the label too; else what is near it.
    let (near, home): (Vec<(usize, f32)>, Option<usize>) = match &app.map.suggesting {
        Some(label) => {
            let said = format!("Would go to {label} too · y files the first, Y all, - not it");
            for l in wrap(&said, width) {
                tail.push(Line::styled(l, Style::new().fg(t.title).bold()));
            }
            let folded = crate::index::fold(label);
            let home = s
                .groups
                .iter()
                .position(|g| g.label.as_deref().map(crate::index::fold) == Some(folded.clone()));
            let mut near = app.suggestions();
            near.truncate(8);
            if near.is_empty() {
                tail.push(Line::styled("nothing more, by angle", dim));
            }
            (near, home)
        }
        None => {
            // Filed under no label yet: where it would go.
            if let Some(label) = s.suggestion(i) {
                let said =
                    format!("Would go to {label} · y files it there, Y all like it, - not it");
                for l in wrap(&said, width) {
                    tail.push(Line::styled(l, Style::new().fg(t.title).bold()));
                }
            }
            tail.push(Line::styled(
                "Near it, by angle",
                Style::new().fg(t.title).bold(),
            ));
            (s.model.nearest(i, 5), None)
        }
    };
    for (j, deg) in near {
        let n = &s.dots[j];
        let across = home.is_some_and(|h| s.clustering.of.get(j) != Some(&h));
        let date = n.date.map(|x| short_date(x, &now)).unwrap_or_default();
        let from = if n.from.is_empty() {
            &n.address
        } else {
            &n.from
        };
        let what = if across {
            format!("{from} — {} · {date} · across the map", n.subject)
        } else {
            format!("{from} — {} · {date}", n.subject)
        };
        for (k, l) in wrap(&what, width.saturating_sub(6)).into_iter().enumerate() {
            let degrees = if k == 0 {
                format!("{deg:>3.0}°  ")
            } else {
                " ".repeat(6)
            };
            tail.push(Line::from(vec![
                Span::styled(degrees, dim),
                Span::styled(l, soft),
            ]));
        }
    }
    Some((head, tail))
}

/// A message as it is read: its subject, who it is from and to, when, its
/// attachments, then its text, quoted lines and the signature dimmed.
fn message(m: &Message, width: usize, t: &Theme) -> Vec<Line<'static>> {
    let dim = Style::new().fg(t.dim);
    let text = Style::new().fg(t.text);
    let mut lines = vec![Line::raw("")];
    let subject = if m.subject.is_empty() {
        "(no subject)"
    } else {
        &m.subject
    };
    lines.extend(
        wrap(subject, width)
            .into_iter()
            .map(|l| Line::styled(l, Style::new().fg(t.title).bold())),
    );
    lines.push(Line::raw(""));
    let mut field = |name: &str, value: &str| {
        for (i, l) in wrap(value, width.saturating_sub(LABEL))
            .into_iter()
            .enumerate()
        {
            let label = if i == 0 { name } else { "" };
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<LABEL$}"), dim),
                Span::styled(l, Style::new().fg(t.soft)),
            ]));
        }
    };
    field("From", &m.from);
    if !m.to.is_empty() {
        field("To", &m.to.join(", "));
    }
    if !m.cc.is_empty() {
        field("Cc", &m.cc.join(", "));
    }
    if let Some(d) = m.date {
        field("Date", &long_date(d));
    }
    if !m.attachments.is_empty() {
        let files: Vec<String> = m
            .attachments
            .iter()
            .map(|a| format!("{} ({})", a.name, size(a.size)))
            .collect();
        field("Files", &files.join(", "));
    }
    lines.push(Line::styled("─".repeat(width), Style::new().fg(t.rule)));
    lines.push(Line::raw(""));
    let mut signature = false;
    for line in m.text.lines() {
        signature |= line == "-- " || line == "--";
        let style = if signature {
            Style::new().fg(t.faint)
        } else if line.trim_start().starts_with('>') {
            dim
        } else {
            text
        };
        lines.extend(
            wrap(line, width)
                .into_iter()
                .map(|l| Line::styled(l, style)),
        );
    }
    if m.text.is_empty() {
        lines.push(Line::styled("(no text)", dim));
    }
    lines
}

pub(crate) fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b.div_ceil(1024)),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Where a key's hint is at the foot: its line, how far along, how wide.
type Spot = (usize, u16, u16, Target);

/// The foot: what happened, at the right of the keys where there is room,
/// else on a line of its own above them; the keys; and where each key's
/// hint is, by its line, how far along and how wide, to be clicked.
fn foot(app: &App, width: u16) -> (Vec<Line<'static>>, Vec<Spot>) {
    let t = app.theme;
    let mut lines = Vec::new();
    let mut spots = Vec::new();
    for (y, items) in hints(app, width).into_iter().enumerate() {
        let mut spans = Vec::new();
        let mut x = 0u16;
        for (k, what) in items {
            let w = (k.width() + what.width()) as u16;
            // Where each of its keys is, from the column after its space.
            for (rect, target) in hint_spots(k.trim(), x + 1, 0, w - 1) {
                spots.push((y, rect.x, rect.width, target));
            }
            x += w;
            // The key a block of its own colour, to stand out.
            spans.push(Span::styled(k, Style::new().fg(t.title).bg(t.focus).bold()));
            spans.push(Span::styled(what, Style::new().fg(t.soft)));
        }
        lines.push(Line::from(spans));
    }
    if app.status.is_empty() {
        return (lines, spots);
    }
    let status = Span::styled(format!("{} ", app.status), Style::new().fg(t.text).bold());
    let last = lines.last_mut().expect("a line of keys");
    let gap = (width as usize).saturating_sub(last.width() + status.width());
    if gap >= 2 {
        last.spans.push(Span::raw(" ".repeat(gap)));
        last.spans.push(status);
    } else {
        let text = clip(
            status.content.trim_end(),
            (width as usize).saturating_sub(1),
        );
        lines.insert(
            0,
            Line::from(Span::styled(format!(" {text}"), status.style)),
        );
        for spot in &mut spots {
            spot.0 += 1;
        }
    }
    (lines, spots)
}

/// The foot, as `foot` lays it out, drawn in `area`, and its keys' hints
/// to be clicked.
fn keys(f: &mut Frame, app: &mut App, area: Rect, lines: Vec<Line<'static>>, spots: Vec<Spot>) {
    f.render_widget(
        Paragraph::new(lines).style(Style::new().bg(app.theme.side)),
        area,
    );
    for (y, x, w, target) in spots {
        let at = Rect {
            x: area.x + x,
            y: area.y + y as u16,
            width: w.min(area.width.saturating_sub(x)),
            height: 1,
        };
        if at.y < area.bottom() {
            app.targets.push((at, target));
        }
    }
}

/// The keys of the pane with the focus, as many lines of them as the
/// foot takes on a screen `width` wide: hints left out to fit, but help and
/// quit.
fn hints(app: &App, width: u16) -> Vec<Vec<(String, String)>> {
    let pane = match app.focus {
        _ if matches!(app.dialog, Dialog::Trash(_)) => Pane::Picking,
        _ if matches!(app.dialog, Dialog::Attach(_)) => Pane::Attaching,
        _ if matches!(app.dialog, Dialog::View) => Pane::Viewing,
        _ if app.compose.is_some() => Pane::Writing,
        Focus::Side => Pane::Side,
        Focus::List if app.map.mode == crate::map::Mode::Map => Pane::Map,
        Focus::Reader
            if app.map.mode == crate::map::Mode::Map
                && app.review.as_ref().is_some_and(|r| !r.reading) =>
        {
            Pane::Review
        }
        Focus::List => Pane::List,
        Focus::Reader => Pane::Reader,
    };
    let mut list = keymap::foot(pane);
    let mut lines = lay(&list, width);
    while lines.len() > FOOT
        && let Some(last) = list.iter().rposition(|h| !h.kept)
    {
        list.remove(last);
        lines = lay(&list, width);
    }
    lines
}

fn lay(list: &[keymap::Hint], width: u16) -> Vec<Vec<(String, String)>> {
    let mut lines: Vec<Vec<(String, String)>> = vec![Vec::new()];
    let mut w = 0;
    for h in list {
        let (k, what) = (format!(" {} ", h.key), format!("{} ", h.what));
        let iw = k.width() + what.width();
        if w + iw > width as usize && w > 0 {
            lines.push(Vec::new());
            w = 0;
        }
        w += iw;
        lines.last_mut().expect("a line").push((k, what));
    }
    lines
}

fn help(f: &mut Frame, t: &Theme, scroll: usize) -> (Rect, usize) {
    let rows = keymap::help().chain([("click", "pick; the wheel scrolls")]);
    // Each key's line wrapped under itself, past the keys' column.
    let width = f.area().width.min(80);
    let room = (width as usize).saturating_sub(4 + KEY);
    let mut lines: Vec<Line> = Vec::new();
    for (k, what) in rows {
        for (i, l) in wrap(what, room).into_iter().enumerate() {
            let k = if i == 0 { k } else { "" };
            lines.push(Line::from(vec![
                Span::styled(format!("{k:<KEY$}"), Style::new().fg(t.title).bold()),
                Span::raw(l),
            ]));
        }
    }
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    // Taller than the screen, it scrolls.
    let shows = area.height.saturating_sub(2) as usize;
    let scroll = scroll.min(lines.len().saturating_sub(shows));
    let foot = if lines.len() > shows {
        " j/k or the wheel scroll · any other key closes "
    } else {
        " any key closes "
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.dim))
        .title(Span::styled(" Keys ", Style::new().fg(t.title).bold()))
        .title_bottom(Line::styled(foot, Style::new().fg(t.dim)))
        .padding(Padding::horizontal(1))
        .style(Style::new().bg(t.page).fg(t.text));
    let lines: Vec<Line> = lines.into_iter().skip(scroll).collect();
    f.render_widget(Paragraph::new(lines).block(block), area);
    (area, scroll)
}

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
