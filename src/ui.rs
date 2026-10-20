//! The screen, laid out as a mail app's: the accounts and their folders on
//! a tinted sidebar, the folder's messages, and the message read, side by
//! side where there is room. In a narrow window, as an Emacs buffer split
//! in two, one of them at a time.

use crate::app::{App, Body, FIELDS, Focus, Form, Row, State, Target};
use crate::mail::Message;
use crate::theme::Theme;
use jiff::{Timestamp, ToSpan, Zoned};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph,
};
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
const KEY: usize = 9;

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

pub fn draw(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    app.targets.clear();
    f.render_widget(
        Block::new().style(Style::new().bg(t.page).fg(t.text)),
        f.area(),
    );
    let foot_height = foot(app, f.area().width).len() as u16;
    let [top, main, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(foot_height),
    ])
    .areas(f.area());
    let [side_w, list_w, read_w] = panes(main.width, app.focus);
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
        messages(f, app, list);
    }
    if read_w > 0 {
        reader(f, app, read);
    }
    keys(f, app, foot);
    let popup = if app.help {
        Some(help(f, &t))
    } else if let Some(form) = &app.form {
        Some(form_dialog(f, &t, form))
    } else {
        app.removing.map(|a| removing(f, &t, &app.accounts[a].name))
    };
    if let Some(area) = popup {
        app.targets.push((area, Target::Popup));
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

/// The form an account is added with: each field on a line, the one typed
/// in lit; the password shown as dots; what to know, or what went wrong,
/// below.
fn form_dialog(f: &mut Frame, t: &Theme, form: &Form) -> Rect {
    let width = f.area().width.min(72);
    let room = (width as usize).saturating_sub(4);
    let value_w = room.saturating_sub(KEY + 1);
    let mut lines = vec![Line::raw("")];
    for (i, name) in FIELDS.iter().enumerate() {
        let here = i == form.at && !form.checking;
        let value = if *name == "Password" {
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
            Span::styled(
                format!("{name:<KEY$}"),
                if here {
                    Style::new().fg(t.title).bold()
                } else {
                    Style::new().fg(t.dim)
                },
            ),
            Span::styled(
                format!(" {value}{}", " ".repeat(pad)),
                Style::new()
                    .fg(t.text)
                    .bg(if here { t.field } else { t.page }),
            ),
        ]));
    }
    lines.push(Line::raw(""));
    let note_style = if form.failed {
        Style::new().fg(t.error)
    } else {
        Style::new().fg(t.soft)
    };
    if let Some(note) = &form.note {
        lines.extend(
            wrap(note, room)
                .into_iter()
                .map(|l| Line::styled(l, note_style)),
        );
    }
    let keeper = if cfg!(target_os = "macos") {
        "the keychain"
    } else {
        "the system's keyring"
    };
    lines.extend(
        wrap(
            &format!("It signs in before it is added. The password is kept in {keeper}, not in the accounts file."),
            room,
        )
        .into_iter()
        .map(|l| Line::styled(l, Style::new().fg(t.faint))),
    );
    let foot = if form.checking {
        "signing in… · esc closes"
    } else {
        "tab next · ⏎ on the password signs in and adds · esc cancel"
    };
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, "Add account", foot)),
        area,
    );
    area
}

fn removing(f: &mut Frame, t: &Theme, name: &str) -> Rect {
    let width = f.area().width.min(64);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![Line::raw("")];
    lines.extend(
        wrap(
            &format!(
                "Remove {name} from letterbox? Its mail stays on its server; the password \
                 letterbox kept for it is forgotten."
            ),
            room,
        )
        .into_iter()
        .map(Line::raw),
    );
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(
            t,
            "Remove account",
            "y removes · any other key keeps it",
        )),
        area,
    );
    area
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
    if app.query.is_empty() && !app.searching {
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
    let count = match (&app.listing, app.folder()) {
        (Some(l), _) if l.loading && l.messages.is_empty() => "fetching… ".to_string(),
        (Some(l), Some(fo)) => {
            let unseen = fo
                .unseen
                .filter(|&n| n > 0)
                .map(|n| format!(", {n} unread"))
                .unwrap_or_default();
            let shown = if app.query.is_empty() {
                String::new()
            } else {
                format!("{} of ", app.shown.len())
            };
            format!("{shown}{} messages{unseen} ", l.total)
        }
        _ => String::new(),
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
    let listed = app.listing.as_ref().map(|l| (l.account, l.folder.clone()));
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| match *row {
            Row::Account(a) => {
                let account = &app.accounts[a];
                let mut spans = vec![Span::styled(
                    format!(
                        " {}",
                        clip(&account.name.to_uppercase(), width.saturating_sub(16))
                    ),
                    Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
                )];
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
                let indent = "  ".repeat(folder.depth + 1);
                let count = folder
                    .unseen
                    .filter(|&n| n > 0)
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                let room = width.saturating_sub(indent.len() + 2 + count.len() + 2);
                let name = clip(&folder.name, room);
                let pad = room.saturating_sub(name.width());
                let open = listed.as_ref() == Some(&(a, folder.path.clone()));
                let style = if !folder.selectable {
                    Style::new().fg(t.dim)
                } else if open {
                    Style::new().fg(t.title).bold()
                } else {
                    Style::new().fg(t.soft)
                };
                ListItem::new(Line::from(vec![
                    Span::raw(indent),
                    Span::styled(format!("{} ", folder.role.icon()), Style::new().fg(t.faint)),
                    Span::styled(name, style),
                    Span::raw(" ".repeat(pad + 1)),
                    Span::styled(count, Style::new().fg(t.dim).bold()),
                ]))
            }
            Row::Add => ListItem::new(vec![
                Line::raw(""),
                Line::from(vec![
                    Span::styled("  + ", Style::new().fg(t.accent)),
                    Span::styled("Add account", Style::new().fg(t.dim)),
                ]),
            ]),
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
            if matches!(r, Row::Account(_) | Row::Add) {
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
            let right_w: usize = right.iter().map(|s| s.content.width()).sum();
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
        if let Some(e) = &l.error {
            lines.push(Line::styled("  Cannot list this folder:", title));
            lines.extend(e.lines().map(|e| Line::styled(format!("  {e}"), dim)));
        } else if l.loading {
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
    let lines: Vec<Line> = match app.reading.as_ref().map(|r| &r.body) {
        None => vec![
            Line::raw(""),
            Line::styled(
                if app.shown.is_empty() {
                    "No message picked."
                } else {
                    "⏎ reads the message picked."
                },
                dim,
            ),
        ],
        Some(Body::Loading) => vec![Line::raw(""), Line::styled("fetching…", dim)],
        Some(Body::Failed(e)) => {
            let mut lines = vec![
                Line::raw(""),
                Line::styled("Cannot read this message:", Style::new().fg(t.title).bold()),
            ];
            lines.extend(wrap(e, width).into_iter().map(|l| Line::styled(l, dim)));
            lines
        }
        Some(Body::Ready(m)) => message(m, width, &t),
    };
    // A message longer than the pane gives its last line to saying how far
    // in it is.
    let longer = lines.len() > inner.height as usize;
    let height = (inner.height as usize).saturating_sub(usize::from(longer));
    app.height = height;
    app.lines = lines.len();
    app.scroll = app.scroll.min(lines.len().saturating_sub(height));
    let shown: Vec<Line> = lines.into_iter().skip(app.scroll).take(height).collect();
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

/// The foot: what happened, at the right of the keys where there is room,
/// else on a line of its own above them; and the keys.
fn foot(app: &App, width: u16) -> Vec<Line<'static>> {
    let t = app.theme;
    let mut lines = hints(app, width);
    if app.status.is_empty() {
        return lines;
    }
    let status = Span::styled(format!("{} ", app.status), Style::new().fg(t.text).bold());
    let last = lines.last_mut().expect("a line of keys");
    let gap = (width as usize).saturating_sub(last.width() + status.width());
    if gap >= 2 {
        last.spans.push(Span::raw(" ".repeat(gap)));
        last.spans.push(status);
    } else {
        let room = width as usize;
        let text = clip(status.content.trim_end(), room.saturating_sub(1));
        lines.insert(
            0,
            Line::from(Span::styled(format!(" {text}"), status.style)),
        );
    }
    lines
}

fn keys(f: &mut Frame, app: &App, area: Rect) {
    let lines = foot(app, area.width);
    f.render_widget(
        Paragraph::new(lines).style(Style::new().bg(app.theme.side)),
        area,
    );
}

/// The keys of the pane with the focus, as many lines of them as the
/// foot takes on a screen `width` wide: hints left out to fit, but help and
/// quit.
fn hints(app: &App, width: u16) -> Vec<Line<'static>> {
    let t = app.theme;
    let mut list: Vec<(&str, &str)> = match app.focus {
        Focus::Side => vec![
            ("⏎", "open"),
            ("j/k", "move"),
            ("A", "add account"),
            ("x", "remove"),
            ("r", "refresh"),
        ],
        Focus::List => vec![
            ("⏎", "read"),
            ("/", "search"),
            ("u", "read/unread"),
            ("s", "flag"),
            ("r", "refresh"),
        ],
        Focus::Reader => vec![
            ("␣", "page"),
            ("n/p", "next/previous"),
            ("u", "unread"),
            ("s", "flag"),
            ("o", "browser"),
            ("a", "save files"),
            ("esc", "back"),
        ],
    };
    list.extend([("?", "help"), ("q", "quit")]);
    let lay = |list: &[(&str, &str)]| -> Vec<Vec<(String, String)>> {
        let mut lines: Vec<Vec<(String, String)>> = vec![Vec::new()];
        let mut w = 0;
        for (k, what) in list {
            let (k, what) = (format!(" {k} "), format!("{what} "));
            let iw = k.width() + what.width();
            if w + iw > width as usize && w > 0 {
                lines.push(Vec::new());
                w = 0;
            }
            w += iw;
            lines.last_mut().expect("a line").push((k, what));
        }
        lines
    };
    let mut lines = lay(&list);
    while lines.len() > FOOT && list.len() > 2 {
        list.remove(list.len() - 3);
        lines = lay(&list);
    }
    lines
        .into_iter()
        .map(|items| {
            Line::from(
                items
                    .into_iter()
                    .flat_map(|(k, what)| {
                        [
                            Span::styled(k, Style::new().fg(t.soft).bold()),
                            Span::styled(what, Style::new().fg(t.dim)),
                        ]
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn help(f: &mut Frame, t: &Theme) -> Rect {
    let rows = [
        ("j k ↑ ↓", "move; in a message, scroll"),
        (
            "h l tab",
            "move between folders, messages and the message read",
        ),
        ("⏎", "open the folder, or read the message"),
        ("space b", "a page down, or up, in the message"),
        ("n p", "read the next message, or the one before"),
        ("g G", "go to the top, or the end"),
        (
            "/",
            "search the messages listed as you type; ⏎ searches the server",
        ),
        ("esc", "back to the list; clear the search"),
        ("u", "mark read, or unread"),
        ("s", "flag, or unflag"),
        ("o", "open the message in the browser, as its HTML"),
        ("a", "save its attachments to Downloads"),
        (
            "r",
            "look for new mail now; it is looked for every two minutes",
        ),
        ("A", "add an account: its address and password"),
        ("x", "remove the account picked in the sidebar"),
        ("click", "pick; the wheel scrolls"),
        ("q", "quit"),
    ];
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
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.dim))
        .title(Span::styled(" Keys ", Style::new().fg(t.title).bold()))
        .padding(Padding::horizontal(1))
        .style(Style::new().bg(t.page).fg(t.text));
    f.render_widget(Paragraph::new(lines).block(block), area);
    area
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
