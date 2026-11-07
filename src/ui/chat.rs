//! Slack drawn: its rows in the sidebar, the conversation read in the list's
//! place, its newest last as Slack has it, and a thread in the message's.

use super::*;
use crate::app::chat::{ChatRow, Open, Part};
use crate::slack::reading::Message as Said;

/// Slack's column, at the right: each workspace's heading, its channels and
/// its people, and the row that shows every one.
pub(super) fn column(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(t.rule))
        .style(Style::new().bg(t.side));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let items: Vec<ListItem> = app
        .chat
        .rows
        .iter()
        .map(|row| match *row {
            ChatRow::Head(s) => head(app, s, width),
            ChatRow::Section(_, part) => {
                let name = match part {
                    Part::Channels => " CHANNELS",
                    Part::People => " PEOPLE",
                };
                ListItem::new(Line::styled(name, Style::new().fg(t.faint).bold()))
            }
            ChatRow::Talk(s, i) => talk(app, s, i, width),
            ChatRow::More(s) => more(app, s),
        })
        .collect();
    let picked = if app.focus == Focus::Chat {
        Style::new().bg(t.focus)
    } else {
        Style::new().bg(t.row)
    };
    let mut state = ListState::default().with_selected(Some(app.chat.row));
    f.render_stateful_widget(List::new(items).highlight_style(picked), inner, &mut state);
    let heights: Vec<u16> = app
        .chat
        .rows
        .iter()
        .map(|r| if matches!(r, ChatRow::Head(_)) { 2 } else { 1 })
        .collect();
    for (i, row) in super::list::rows(inner, state.offset(), &heights) {
        app.targets.push((row, Target::ChatRow(i)));
    }
}

/// A workspace's heading in Slack's column.
pub(super) fn head(app: &App, space: usize, width: usize) -> ListItem<'static> {
    let t = app.theme;
    let s = &app.chat.spaces[space];
    let mut after = Vec::new();
    if s.loading {
        after.push(Span::styled(" reading…", Style::new().fg(t.faint)));
    } else if s.error.is_some() {
        after.push(Span::styled(" cannot read", Style::new().fg(t.error)));
    }
    let used: usize = after.iter().map(Span::width).sum();
    let live = if s.live { " · live" } else { "" };
    let name = clip(
        &format!("SLACK · {}{live}", s.name.to_uppercase()),
        width.saturating_sub(used + 2).max(4),
    );
    let mut spans = vec![Span::styled(
        format!(" {name}"),
        Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
    )];
    spans.extend(after);
    ListItem::new(vec![Line::raw(""), Line::from(spans)])
}

/// A conversation's row: its kind, its name, and what it has unread.
pub(super) fn talk(app: &App, space: usize, i: usize, width: usize) -> ListItem<'static> {
    let t = app.theme;
    let talk = &app.chat.spaces[space].talks[i];
    let mark = talk.channel.kind.mark();
    let unread = talk.unread.unwrap_or(0);
    let count = match unread {
        0 => String::new(),
        n if n >= crate::slack::UNREAD_MOST => format!("{n}+"),
        n => n.to_string(),
    };
    let room = width.saturating_sub(2 + 2 + count.len() + 2);
    let name = clip(&talk.channel.name, room);
    let pad = room.saturating_sub(name.width());
    let open = app
        .chat
        .open
        .as_ref()
        .is_some_and(|o| o.space == space && o.id == talk.channel.id);
    let style = if open {
        Style::new().fg(t.title).bold()
    } else if unread > 0 {
        Style::new().fg(t.text).bold()
    } else {
        Style::new().fg(t.soft)
    };
    ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{mark} "), Style::new().fg(t.faint)),
        Span::styled(name, style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(count, Style::new().fg(t.dim).bold()),
    ]))
}

/// The row that shows every conversation, or folds them again.
pub(super) fn more(app: &App, space: usize) -> ListItem<'static> {
    let t = app.theme;
    let shown = app.chat.shown(space).len();
    let s = &app.chat.spaces[space];
    let said = if s.unfolded {
        "  fewer".to_string()
    } else {
        format!("  {} more", s.talks.len() - shown)
    };
    ListItem::new(Line::styled(said, Style::new().fg(t.faint)))
}

/// When a message was, as the list says when mail came.
fn when(ts: &str) -> String {
    let Some(at) = crate::slack::seconds(ts).and_then(|s| Timestamp::from_second(s).ok()) else {
        return String::new();
    };
    let here = at.to_zoned(jiff::tz::TimeZone::system());
    let today = Zoned::now().date();
    if here.date() == today {
        here.strftime("%H:%M").to_string()
    } else {
        here.strftime("%b %-d %H:%M").to_string()
    }
}

/// A message's lines: who and when, then what they wrote, wrapped.
fn said(t: &Theme, message: &Said, width: usize, picked: bool) -> Vec<Line<'static>> {
    let mark = if picked { "▌" } else { " " };
    let mut lines = vec![Line::from(vec![
        Span::styled(mark, Style::new().fg(t.accent)),
        Span::styled(message.who.clone(), Style::new().fg(t.title).bold()),
        Span::styled(format!("  {}", when(&message.ts)), Style::new().fg(t.faint)),
    ])];
    for line in wrap(&message.text, width.saturating_sub(2)) {
        lines.push(Line::from(vec![
            Span::styled(mark, Style::new().fg(t.accent)),
            Span::styled(line, Style::new().fg(t.text)),
        ]));
    }
    for file in &message.files {
        let line = clip(
            &format!("📎 {} ({})", file.name, size(file.size as usize)),
            width.saturating_sub(2),
        );
        lines.push(Line::from(vec![
            Span::styled(mark, Style::new().fg(t.accent)),
            Span::styled(line, Style::new().fg(t.soft)),
        ]));
    }
    if message.replies > 0 {
        let s = if message.replies == 1 { "y" } else { "ies" };
        lines.push(Line::from(vec![
            Span::styled(mark, Style::new().fg(t.accent)),
            Span::styled(
                format!("↳ {} repl{s}", message.replies),
                Style::new().fg(t.accent),
            ),
        ]));
    }
    lines.push(Line::raw(""));
    lines
}

/// What is said where there are no messages to show.
fn waiting(open_loading: bool, error: Option<&str>) -> Option<String> {
    match (error, open_loading) {
        (Some(why), _) => Some(format!("cannot read it: {why}")),
        (None, true) => Some("reading…".to_string()),
        _ => None,
    }
}

/// The conversation read, in the list's place.
pub(super) fn conversation(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let Some(open) = app.chat.open.as_ref() else {
        return;
    };
    let block = Block::new()
        .title(Line::styled(
            format!(" {}", open.name),
            Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(t.page));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.targets.push((area, Target::List));
    let inner = writing_under(f, app, inner, false);
    let Some(open) = app.chat.open.as_ref() else {
        return;
    };
    let lines = transcript(&t, open, inner.width as usize);
    let lines = match lines {
        Ok((lines, picked_end)) => {
            // The message picked in sight, as near the foot as it can be.
            let height = inner.height as usize;
            let top = picked_end.saturating_sub(height);
            lines.into_iter().skip(top).take(height).collect()
        }
        Err(said) => vec![Line::styled(format!(" {said}"), Style::new().fg(t.dim))],
    };
    f.render_widget(Paragraph::new(lines), inner);
}

/// The conversation's lines, and where the message picked ends; or what is
/// said in their place.
fn transcript(t: &Theme, open: &Open, width: usize) -> Result<(Vec<Line<'static>>, usize), String> {
    if open.messages.is_empty() {
        return Err(waiting(open.loading, open.error.as_deref())
            .unwrap_or_else(|| "nothing said here yet".to_string()));
    }
    let mut lines = Vec::new();
    let mut picked_end = 0;
    for (i, message) in open.messages.iter().enumerate() {
        let picked = open.picked == Some(i);
        lines.extend(said(t, message, width, picked));
        if picked {
            picked_end = lines.len();
        }
    }
    if let Some(why) = &open.error {
        lines.push(Line::styled(
            format!(" not read again: {why}"),
            Style::new().fg(t.error),
        ));
        picked_end = lines.len();
    }
    Ok((lines, picked_end.max(1)))
}

/// The message being written, under the conversation, or under the thread
/// for a reply in it, if one is: what it says, where it goes, and how it is
/// sent; the room left above it.
fn writing_under(f: &mut Frame, app: &App, area: Rect, in_thread: bool) -> Rect {
    let t = app.theme;
    let Some(w) = app
        .chat
        .writing
        .as_ref()
        .filter(|w| w.thread.is_some() == in_thread)
    else {
        return area;
    };
    let width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (row, line) in w.buffer.lines().iter().enumerate() {
        let mut line = line.clone();
        if row == w.buffer.row() && !w.sending {
            let at = line
                .char_indices()
                .nth(w.buffer.col())
                .map_or(line.len(), |(i, _)| i);
            line.insert(at, '▏');
        }
        for part in wrap(&line, width) {
            lines.push(Line::styled(part, Style::new().fg(t.text)));
        }
    }
    if let Some(why) = &w.error {
        for part in wrap(&format!("not sent: {why}"), width) {
            lines.push(Line::styled(part, Style::new().fg(t.error)));
        }
    }
    let tall = (lines.len() as u16 + 2).clamp(3, (area.height / 2).max(3));
    let [above, under] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(tall)]).areas(area);
    let place = if w.thread.is_some() {
        " in its thread"
    } else {
        ""
    };
    let (title, colour) = if w.sending {
        (format!(" Sending to {}{place}… ", w.name), t.dim)
    } else {
        (
            format!(" To {}{place} · C-⏎ sends it · esc lets it go ", w.name),
            t.accent,
        )
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(colour))
        .title(Line::styled(title, Style::new().fg(colour).bold()))
        .style(Style::new().bg(t.page));
    let inside = block.inner(under);
    f.render_widget(Clear, under);
    f.render_widget(block, under);
    let skip = lines.len().saturating_sub(inside.height as usize);
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()),
        inside,
    );
    above
}

/// The thread read, in the message's place.
pub(super) fn thread(f: &mut Frame, app: &mut App, area: Rect, edge: Borders) {
    let t = app.theme;
    let focused = app.focus == Focus::Reader;
    let block = Block::new()
        .borders(edge)
        .border_style(Style::new().fg(if focused { t.accent } else { t.rule }))
        .title(Line::styled(
            " Thread",
            Style::new().fg(t.dim).add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(t.page));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.targets.push((area, Target::Reader));
    let inner = writing_under(f, app, inner, true);
    let Some(thread) = app.chat.open.as_mut().and_then(|o| o.thread.as_mut()) else {
        let hint = " ⏎ on a message reads its thread here";
        f.render_widget(
            Paragraph::new(Line::styled(hint, Style::new().fg(t.faint))),
            inner,
        );
        return;
    };
    let lines: Vec<Line> = if thread.messages.is_empty() {
        let said = waiting(thread.loading, thread.error.as_deref())
            .unwrap_or_else(|| "nothing in it".to_string());
        vec![Line::styled(format!(" {said}"), Style::new().fg(t.dim))]
    } else {
        thread
            .messages
            .iter()
            .flat_map(|m| said(&t, m, inner.width as usize, false))
            .collect()
    };
    let last = lines.len().saturating_sub(inner.height as usize);
    thread.scroll = thread.scroll.min(last);
    let shown: Vec<Line> = lines
        .into_iter()
        .skip(thread.scroll)
        .take(inner.height as usize)
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
}

/// The box a conversation is gone to by name in: what is typed, and those
/// it finds, the one picked lit.
pub(super) fn jump(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let Some(jump) = &app.chat.jump else {
        return;
    };
    let found = app.jump_found();
    let screen = f.area();
    let width = 52.min(screen.width);
    let height = (found.len().max(1) as u16 + 4).min(screen.height);
    let area = Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height.saturating_sub(height)) / 3,
        width,
        height,
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(t.accent))
        .title(Line::styled(" Go to ", Style::new().fg(t.accent).bold()))
        .title_bottom(Line::styled(
            " ⏎ reads it · ↑↓ pick · esc ",
            Style::new().fg(t.faint),
        ))
        .style(Style::new().bg(t.page));
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);
    let room = inner.width as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!(" {}", jump.query), Style::new().fg(t.text)),
            Span::styled("▏", Style::new().fg(t.accent)),
        ]),
        Line::raw(""),
    ];
    if found.is_empty() {
        lines.push(Line::styled(" nothing fits", Style::new().fg(t.dim)));
    }
    for (n, &(s, i)) in found.iter().enumerate() {
        let talk = &app.chat.spaces[s].talks[i];
        let count = match talk.unread.unwrap_or(0) {
            0 => String::new(),
            n => n.to_string(),
        };
        let name = clip(&talk.channel.name, room.saturating_sub(4 + count.len() + 1));
        let pad = room.saturating_sub(4 + name.width() + count.len());
        let style = if n == jump.at {
            Style::new().bg(t.focus).fg(t.title).bold()
        } else {
            Style::new().fg(t.soft)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", talk.channel.kind.mark()), style),
            Span::styled(name, style),
            Span::styled(format!("{}{count}", " ".repeat(pad)), style),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
    app.targets.push((area, Target::Popup));
}
