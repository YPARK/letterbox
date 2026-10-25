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

/// The panes' widths with the map in the middle: as `panes`, but the map
/// wider than a list, as wide as the message read where there is room.
pub fn map_panes(width: u16, focus: Focus) -> [u16; 3] {
    match panes(width, focus) {
        [side, list, read] if list > 0 && read > 0 => {
            let map = ((list + read) / 2).max(list);
            [side, map, list + read - map]
        }
        other => other,
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
    let composing = app.compose.is_some();
    let [side_w, list_w, read_w] = if composing {
        if main.width >= MEDIUM {
            [SIDEBAR, 0, main.width - SIDEBAR]
        } else {
            [0, 0, main.width]
        }
    } else if app.map.mode == crate::map::Mode::Map {
        map_panes(main.width, app.focus)
    } else {
        panes(main.width, app.focus)
    };
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
        if app.compose.is_some() {
            compose_pane(f, app, read);
        } else {
            reader(f, app, read);
        }
    }
    keys(f, app, foot_area, foot_lines, spots);
    let popup = match &app.dialog {
        Dialog::Closed => None,
        &Dialog::Help(scroll) => {
            let (area, scroll) = help(f, &t, scroll);
            app.dialog = Dialog::Help(scroll);
            Some((area, Vec::new()))
        }
        Dialog::Form(form) => Some(form_dialog(f, &t, form)),
        Dialog::Files(files) => Some(files_dialog(f, &t, files)),
        Dialog::Trash(pick) => Some(trash_dialog(f, &t, pick)),
        &Dialog::Remove(a) => Some(removing(f, &t, &app.accounts[a].name)),
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
    }
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
            "Files and links",
            "⏎ open · s save · y copy address · esc close",
        )),
        area,
    );
    let targets = (0..files.items.len())
        .map(|i| (spot(area, 1 + i, 0, room as u16), Target::Item(i)))
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

/// The message being written: its text, the cursor reversed, and a mode
/// line, as an Emacs buffer has one.
fn compose_pane(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(compose) = &mut app.compose else {
        return;
    };
    let t = app.theme;
    let mode_h = 1u16.min(area.height);
    let body = Rect {
        height: area.height.saturating_sub(mode_h),
        ..area
    };
    let mode = Rect {
        y: area.y + body.height,
        height: area.height - body.height,
        ..area
    };
    let rows = (body.height as usize).max(1);
    if compose.buffer.row() < compose.scroll {
        compose.scroll = compose.buffer.row();
    } else if compose.buffer.row() >= compose.scroll + rows {
        compose.scroll = compose.buffer.row() + 1 - rows;
    }
    let width = (body.width as usize).max(1);
    let scroll = compose.scroll;
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
            compose_line(line, i == cursor_row, cursor_col, width, t, region)
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
    let said = match compose.buffer.searching() {
        Some(s) => Span::styled(format!("  {s}"), Style::new().fg(t.title).bold()),
        None => Span::styled(
            "  C-c C-c send   C-c C-k or C-g cancel   C-SPC mark   C-s search",
            Style::new().fg(t.dim),
        ),
    };
    let mode_line = Line::from(vec![
        Span::styled(" Compose ", Style::new().fg(t.side).bg(t.title).bold()),
        said,
    ]);
    f.render_widget(
        Paragraph::new(mode_line).style(Style::new().bg(t.side)),
        mode,
    );
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
        Constraint::Length(if area.width >= MEDIUM { 34 } else { 0 }),
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
        let what = if folder.is_empty() || room < 16 {
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
    f.render_widget(
        Paragraph::new(Span::styled(count, Style::new().fg(t.dim))).alignment(Alignment::Right),
        right,
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
    let items: Vec<ListItem> = app.shown[top..end]
        .iter()
        .map(|m| {
            let unseen = !m.flags.seen;
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
                if unseen {
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
    for (at, row) in rows(inner, 0, &heights) {
        app.targets.push((row, Target::Message(top + at)));
    }
}

/// The map, in the list's place: a legend above, each message a cell, the
/// clusters' anchors, the message picked named beside it, and what the
/// colour says below.
fn map_pane(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::map::{Colour, Focused};
    let t = app.theme;
    f.render_widget(Block::new().style(Style::new().bg(t.page)), area);
    app.targets.push((area, Target::List));
    app.map.cells.clear();
    app.map.drawn.clear();
    app.map.anchors.clear();
    if area.height == 0 || area.width == 0 {
        return;
    }
    let dim = Style::new().fg(t.dim);
    let foot_h = u16::from(area.height >= 4);
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
    let new: usize = shown
        .iter()
        .filter(|&&i| app.map.is_new(&s.dots[i], now))
        .count();
    let unread = shown.iter().filter(|&&i| !s.dots[i].flags.seen).count();
    let mut head = vec![Span::styled(" Map", dim.add_modifier(Modifier::BOLD))];
    let mut what = format!(
        " · {} · {} · {} messages",
        s.lens.name(),
        app.map.since.name(),
        shown.len()
    );
    if unread > 0 {
        what.push_str(&format!(", {unread} unread"));
    }
    if new > 0 {
        what.push_str(&format!(", {new} new"));
    }
    match app.map.focused {
        Focused::Cluster(id) => {
            if let Some(g) = s.group(id) {
                what.push_str(&format!(" · in {}", g.name()));
            }
        }
        Focused::Loners => what.push_str(" · fitting nowhere"),
        Focused::All => {}
    }
    head.push(Span::styled(what, dim));
    let key = "   ● unread  • read  ⚑ flag  ▪ yours  ◆ label  ◇ cluster";
    let used: usize = head.iter().map(|h| h.content.width()).sum();
    if used + key.width() <= legend.width as usize {
        head.push(Span::styled(key, Style::new().fg(t.faint)));
    }
    f.render_widget(Paragraph::new(Line::from(head)), legend);
    let explained = match s.faithfulness {
        Some(_) if app.map.preview.is_some() => {
            " laid out again, a trail from where each was: ⏎ keeps it, esc drops it".to_string()
        }
        Some(faith) if faith < 0.7 => format!(
            " {:.0}% faithful: M-r lays it out again · {}",
            faith * 100.0,
            app.map.colour.explain()
        ),
        Some(faith) => format!(
            " {:.0}% faithful · {}",
            faith * 100.0,
            app.map.colour.explain()
        ),
        None => format!(" {}", app.map.colour.explain()),
    };
    if foot_h > 0 {
        f.render_widget(
            Paragraph::new(Span::styled(
                clip(&explained, foot.width as usize),
                Style::new().fg(t.faint),
            )),
            foot,
        );
    }
    let Some(old) = &s.at else {
        return say(f, "laying the map out…");
    };
    // Laid out again and not yet kept, it is drawn where it would be, with
    // a trail from where each message was.
    let preview = app.map.preview.clone().filter(|p| p.len() == old.len());
    let at = preview.as_ref().unwrap_or(old);
    if shown.is_empty() {
        return say(
            f,
            "Nothing to map here: r shows read mail, t goes further back.",
        );
    }
    if canvas.height == 0 || canvas.width < 3 {
        return;
    }
    app.targets.push((canvas, Target::Map));
    // Where each message goes: the bounds of what is shown, laid onto the
    // canvas, a column clear at each side.
    let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for &i in &shown {
        for p in std::iter::once(at[i]).chain(preview.as_ref().map(|_| old[i])) {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            y0 = y0.min(p[1]);
            y1 = y1.max(p[1]);
        }
    }
    let (w, h) = (canvas.width.saturating_sub(2).max(1), canvas.height.max(1));
    let cell = |p: [f32; 2]| -> (u16, u16) {
        let fx = if x1 > x0 {
            (p[0] - x0) / (x1 - x0)
        } else {
            0.5
        };
        let fy = if y1 > y0 {
            (p[1] - y0) / (y1 - y0)
        } else {
            0.5
        };
        let cx = (fx * f32::from(w - 1)).round() as u16;
        let cy = (fy * f32::from(h - 1)).round() as u16;
        (canvas.x + 1 + cx.min(w - 1), canvas.y + cy.min(h - 1))
    };
    for &i in &shown {
        let c = cell(at[i]);
        app.map.cells.insert(i, c);
        app.map.drawn.entry(c).or_default().push(i);
    }
    let picked = app.map.picked();
    // What has anything new, by cluster: the rest dimmed, when asked.
    let lively: std::collections::BTreeSet<usize> = shown
        .iter()
        .filter(|&&i| app.map.is_new(&s.dots[i], now))
        .filter_map(|&i| s.clustering.of.get(i).copied())
        .collect();
    let ground = t.ground();
    let marked: std::collections::BTreeSet<usize> = app
        .map
        .region
        .iter()
        .flat_map(|r| r.keys.iter().filter_map(|k| s.find(k)))
        .collect();
    let buf = f.buffer_mut();
    if preview.is_some() {
        let trail = Style::new().fg(t.rule).bg(t.page);
        for &i in &shown {
            let (from, to) = (cell(old[i]), cell(at[i]));
            let steps = from.0.abs_diff(to.0).max(from.1.abs_diff(to.1));
            for k in 1..steps {
                let along = f32::from(k) / f32::from(steps);
                let between = |a: u16, b: u16| {
                    (f32::from(a) + along * (f32::from(b) - f32::from(a))).round() as u16
                };
                let (x, y) = (between(from.0, to.0), between(from.1, to.1));
                if !app.map.drawn.contains_key(&(x, y)) {
                    buf[(x, y)].set_char('·').set_style(trail);
                }
            }
        }
    }
    for (&(x, y), members) in &app.map.drawn {
        let first = picked.filter(|p| members.contains(p)).unwrap_or(members[0]);
        let d = &s.dots[first];
        let glyph = if members.len() > 1 {
            char::from_digit(members.len().min(9) as u32, 10).unwrap_or('9')
        } else if d.flags.flagged {
            '⚑'
        } else if d.mine {
            '▪'
        } else if !d.flags.seen {
            '●'
        } else {
            '•'
        };
        let mut colour = match app.map.colour {
            Colour::Angle => t.hue(s.hues[first]),
            Colour::Cursor => match picked {
                Some(p) => {
                    let deg = crate::model::degrees(&s.model.vectors[first], &s.model.vectors[p]);
                    crate::theme::mix(t.accent, t.rule, deg / 90.0)
                }
                None => t.dim,
            },
            Colour::State if !d.flags.seen => t.accent,
            Colour::State if d.flags.flagged => t.flag,
            Colour::State => t.dim,
        };
        let in_focus = members.iter().any(|&i| app.map.in_focus(i));
        let quiet = app.map.new_only
            && !members
                .iter()
                .any(|i| s.clustering.of.get(*i).is_some_and(|c| lively.contains(c)));
        if !in_focus || quiet {
            colour = t.rule;
        } else if members
            .iter()
            .all(|&i| crate::map::Map::is_old(&s.dots[i], now))
        {
            colour = t.faint;
        }
        let new_here = in_focus && members.iter().any(|&i| app.map.is_new(&s.dots[i], now));
        let mut style = if new_here {
            Style::new().fg(ground).bg(colour)
        } else {
            Style::new().fg(colour).bg(t.page)
        };
        if !new_here && members.iter().any(|i| marked.contains(i)) {
            style = style.bg(t.row);
        }
        if picked.is_some_and(|p| members.contains(&p)) {
            style = Style::new().fg(t.title).bg(t.focus).bold();
        }
        buf[(x, y)].set_char(glyph).set_style(style);
    }
    // A box being drawn, or what is marked, drawn round.
    let corners = |(a, b): ((u16, u16), (u16, u16))| {
        ((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)))
    };
    let round = app
        .map
        .drag
        .filter(|(a, b)| a != b)
        .map(corners)
        .or_else(|| app.map.region.as_ref().and_then(|r| r.boxed))
        .or_else(|| {
            let cells: Vec<(u16, u16)> = marked
                .iter()
                .filter_map(|i| app.map.cells.get(i).copied())
                .collect();
            let x0 = cells.iter().map(|c| c.0).min()?;
            let x1 = cells.iter().map(|c| c.0).max()?;
            let y0 = cells.iter().map(|c| c.1).min()?;
            let y1 = cells.iter().map(|c| c.1).max()?;
            Some((
                (
                    x0.saturating_sub(1).max(canvas.x),
                    y0.saturating_sub(1).max(canvas.y),
                ),
                (
                    (x1 + 1).min(canvas.right() - 1),
                    (y1 + 1).min(canvas.bottom() - 1),
                ),
            ))
        });
    if let Some(((x0, y0), (x1, y1))) = round {
        let edge = Style::new().fg(t.accent).bg(t.page);
        let inside = |x: u16, y: u16| {
            x >= canvas.x && x < canvas.right() && y >= canvas.y && y < canvas.bottom()
        };
        for x in x0..=x1 {
            for (y, glyph) in [(y0, '─'), (y1, '─')] {
                if inside(x, y) && !app.map.drawn.contains_key(&(x, y)) {
                    buf[(x, y)].set_char(glyph).set_style(edge);
                }
            }
        }
        for y in y0..=y1 {
            for x in [x0, x1] {
                if inside(x, y) && !app.map.drawn.contains_key(&(x, y)) {
                    buf[(x, y)].set_char('│').set_style(edge);
                }
            }
        }
        for (x, y, glyph) in [(x0, y0, '┌'), (x1, y0, '┐'), (x0, y1, '└'), (x1, y1, '┘')] {
            if inside(x, y) {
                buf[(x, y)].set_char(glyph).set_style(edge);
            }
        }
    }
    // Each cluster's anchor, at the middle of what of it is drawn: a label's
    // in its hue, one not yet a label tinted, with its words.
    let mut taken: Vec<(u16, u16, u16)> = Vec::new();
    for g in &s.groups {
        let cells: Vec<(u16, u16)> = g
            .members
            .iter()
            .filter_map(|i| app.map.cells.get(i).copied())
            .collect();
        if cells.is_empty() {
            continue;
        }
        let n = cells.len() as f32;
        let mx = cells.iter().map(|c| f32::from(c.0)).sum::<f32>() / n;
        let my = cells.iter().map(|c| f32::from(c.1)).sum::<f32>() / n;
        let name = clip(&g.name(), 22);
        let width = (3 + 1 + name.width()) as u16;
        let left = (mx.round() as u16)
            .saturating_sub(1)
            .max(canvas.x)
            .min(canvas.right().saturating_sub(width.min(canvas.width)));
        let mut row = None;
        for dy in [0i32, 1, -1, 2, -2, 3, -3] {
            let y = my.round() as i32 + dy;
            if y < i32::from(canvas.y) || y >= i32::from(canvas.bottom()) {
                continue;
            }
            let y = y as u16;
            let clear = taken
                .iter()
                .all(|&(tx, ty, tw)| ty != y || left + width <= tx || tx + tw <= left);
            if clear {
                row = Some(y);
                break;
            }
        }
        let Some(y) = row else {
            continue;
        };
        taken.push((left, y, width));
        let dimmed = !matches!(app.map.focused, Focused::All | Focused::Cluster(_))
            || matches!(app.map.focused, Focused::Cluster(id) if id != g.id);
        let (chip, chip_style, name_style) = match &g.label {
            Some(_) => (
                " ◆ ",
                Style::new().fg(ground).bg(t.hue(g.hue)).bold(),
                Style::new().fg(t.title).bg(t.page).bold(),
            ),
            None => (
                " ◇ ",
                Style::new().fg(t.hue(g.hue)).bg(t.row),
                Style::new().fg(t.soft).bg(t.page),
            ),
        };
        let name_style = if dimmed {
            Style::new().fg(t.dim).bg(t.page)
        } else {
            name_style
        };
        let line = Line::from(vec![
            Span::styled(chip, chip_style),
            Span::styled(format!(" {name}"), name_style),
        ]);
        let spot = Rect {
            x: left,
            y,
            width: width.min(canvas.right().saturating_sub(left)),
            height: 1,
        };
        f.render_widget(Paragraph::new(line), spot);
        app.map.anchors.push((spot.x, spot.y, spot.width, g.id));
        app.targets.push((
            Rect {
                width: 3.min(spot.width),
                ..spot
            },
            Target::Anchor(g.id),
        ));
    }
    // The message picked, named beside it: its sender and its subject.
    if let Some(p) = picked
        && let Some(&(x, y)) = app.map.cells.get(&p)
    {
        let d = &s.dots[p];
        let from = if d.from.is_empty() {
            &d.address
        } else {
            &d.from
        };
        let said = format!(" {from} — {} ", d.subject);
        let room_right = canvas.right().saturating_sub(x + 1) as usize;
        let room_left = x.saturating_sub(canvas.x) as usize;
        let (said, left) = if said.width() <= room_right || room_right >= room_left {
            let said = clip(&said, room_right);
            (said, x + 1)
        } else {
            let said = clip(&said, room_left);
            let w = said.width() as u16;
            (said, x.saturating_sub(w))
        };
        let w = said.width() as u16;
        if w > 0 {
            f.render_widget(
                Paragraph::new(Span::styled(said, Style::new().fg(t.title).bg(t.page))),
                Rect {
                    x: left,
                    y,
                    width: w,
                    height: 1,
                },
            );
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

fn reader(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(if app.focus == Focus::Reader {
            t.dim
        } else {
            t.rule
        }))
        .padding(Padding::horizontal(1))
        .style(Style::new().bg(t.page));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.targets.push((area, Target::Reader));
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

fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b.div_ceil(1024)),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Where a key's hint is at the foot: its line, how far along, how wide.
type Spot = (usize, u16, u16, KeyEvent);

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
            if let Some(code) = keymap::key_named(k.trim()) {
                spots.push((y, x, w, code));
            }
            x += w;
            spans.push(Span::styled(k, Style::new().fg(t.soft).bold()));
            spans.push(Span::styled(what, Style::new().fg(t.dim)));
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
    for (y, x, w, code) in spots {
        let at = Rect {
            x: area.x + x,
            y: area.y + y as u16,
            width: w.min(area.width.saturating_sub(x)),
            height: 1,
        };
        if at.y < area.bottom() {
            app.targets.push((at, Target::Key(code)));
        }
    }
}

/// The keys of the pane with the focus, as many lines of them as the
/// foot takes on a screen `width` wide: hints left out to fit, but help and
/// quit.
fn hints(app: &App, width: u16) -> Vec<Vec<(String, String)>> {
    let pane = match app.focus {
        _ if matches!(app.dialog, Dialog::Trash(_)) => Pane::Picking,
        _ if app.compose.is_some() => Pane::Writing,
        Focus::Side => Pane::Side,
        Focus::List if app.map.mode == crate::map::Mode::Map => Pane::Map,
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
