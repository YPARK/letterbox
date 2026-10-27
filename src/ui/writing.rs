//! The message being written, over the screen: its header lines, its
//! text as edited, and its keys on its edge.
use super::*;

/// The message being written, over the screen, faded beneath it: as wide
/// as two thirds of it, a letter's lines at least, and as tall as it.
pub(super) fn compose_popup(f: &mut Frame, app: &mut App, main: Rect) {
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
        ("Grammar", "C-c C-s", t.flag, t.on_accent),
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
    grammar_popup(f, app, area);
}

/// Over the message being written, the change a model would make, asked:
/// the sentence it is in, what it is in red, what it would be, and a button
/// for each answer, each pressed by its key; or, gone through to be sent,
/// whether it is to go now.
fn grammar_popup(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let (title, foot, lines, buttons) = if let Some(e) = app.checking.as_ref().and_then(|c| {
        let e = c.edit()?;
        Some((e.clone(), c.at + 1, c.edits.len()))
    }) {
        let (e, at, of) = e;
        let line = app
            .compose
            .as_ref()
            .and_then(|c| c.buffer.lines().get(e.row).cloned())
            .unwrap_or_default();
        let chars: Vec<char> = line.chars().collect();
        let room = usize::from(area.width.min(76)).saturating_sub(6);
        let said_w = e.said.chars().count().max(1);
        let side = room.saturating_sub(said_w) / 2;
        let from = e.start.saturating_sub(side).min(e.start);
        let to = (e.end + side).min(chars.len());
        // An ellipsis only where the line is cut.
        let before = format!(
            "{}{}",
            if from > 0 { "…" } else { "" },
            chars[from..e.start.min(chars.len())]
                .iter()
                .collect::<String>()
        );
        let after = format!(
            "{}{}",
            chars[e.end.min(chars.len())..to].iter().collect::<String>(),
            if to < chars.len() { "…" } else { "" }
        );
        let red = Style::new()
            .fg(t.title)
            .bg(crate::theme::mix(t.error, t.page, 0.55))
            .bold();
        let marked = if e.said.is_empty() {
            "▴".to_string()
        } else {
            e.said.clone()
        };
        let (said, with) = (e.said.trim(), e.with.trim());
        let what = match (said.is_empty(), with.is_empty()) {
            (true, _) => format!("put in “{with}”"),
            (_, true) => format!("take out “{said}”"),
            _ => format!("“{said}” → “{with}”"),
        };
        let lines = vec![
            Line::raw(""),
            Line::from(vec![
                Span::styled(before, Style::new().fg(t.text)),
                Span::styled(marked, red),
                Span::styled(after, Style::new().fg(t.text)),
            ]),
            Line::raw(""),
            Line::styled(what, Style::new().fg(t.title).bold()),
            Line::raw(""),
        ];
        let buttons = vec![
            ("Yes", 'y', t.accent, t.on_accent),
            ("Skip", 'n', t.focus, t.title),
            ("Accept all", 'a', t.flag, t.on_accent),
            ("Ignore all", 'i', t.error, t.on_accent),
        ];
        (format!("Grammar · {at} of {of}"), "y n a i", lines, buttons)
    } else if app.sending_asked {
        let lines = vec![
            Line::raw(""),
            Line::styled("Its grammar is gone through.", Style::new().fg(t.title)),
            Line::raw(""),
        ];
        let buttons = vec![
            ("Send", 'y', t.accent, t.on_accent),
            ("Keep writing", 'n', t.focus, t.title),
        ];
        (
            "Send it now?".to_string(),
            "y sends · any other key goes on writing",
            lines,
            buttons,
        )
    } else {
        return;
    };
    // The buttons, a line of their own, each its own colour and key.
    let said: Vec<String> = buttons
        .iter()
        .map(|(what, key, ..)| format!(" {what}  {key} "))
        .collect();
    let row_w: u16 = said.iter().map(|s| s.width() as u16 + 1).sum();
    let width = area.width.min(76).max(row_w + 4).min(f.area().width);
    let height = lines.len() as u16 + 3;
    // Low in the message, beneath what is written at its top.
    let y = area.bottom().saturating_sub(height + 4).max(area.y + 1);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let popup = Rect::new(x, y, width, height.min(f.area().height));
    f.render_widget(Clear, popup);
    let title = title.as_str();
    f.render_widget(
        Paragraph::new(lines.clone()).block(dialog(&t, title, foot)),
        popup,
    );
    let row = popup.y + lines.len() as u16 + 1;
    let mut bx = popup.x + 2;
    let buf = f.buffer_mut();
    for ((_, key, bg, fg), said) in buttons.iter().zip(&said) {
        let w = said.width() as u16;
        if row >= popup.bottom() || bx + w > popup.right() {
            break;
        }
        buf.set_stringn(
            bx,
            row,
            said,
            usize::from(w),
            Style::new().fg(*fg).bg(*bg).bold(),
        );
        let pressed = KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char(*key),
            ratatui::crossterm::event::KeyModifiers::NONE,
        );
        app.targets
            .push((Rect::new(bx, row, w, 1), Target::Key(pressed)));
        bx += w + 1;
    }
}

/// What an `Attach` line's file is: its size, or why it cannot be sent.
pub(super) fn attached(
    value: &str,
    forwarded: &[crate::compose::File],
) -> Result<String, &'static str> {
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
pub(super) fn compose_pane(f: &mut Frame, app: &mut App, area: Rect) {
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
            // The change a model would make, looked at, marked as the region
            // is: what it would take out, or where it would put in.
            let checked = app
                .checking
                .as_ref()
                .and_then(|c| c.edit())
                .filter(|e| e.row == i)
                .map(|e| e.start..e.end.max(e.start + 1));
            let (region, mark) = match checked {
                Some(range) => (
                    range,
                    Style::new()
                        .fg(t.title)
                        .bg(crate::theme::mix(t.error, t.page, 0.55))
                        .bold(),
                ),
                None => (
                    compose.buffer.marked(i),
                    Style::new().fg(t.title).bg(t.focus),
                ),
            };
            let mut shown = compose_line(line, i == cursor_row, cursor_col, width, t, region, mark);
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
/// columns in `region` marked in `mark`, its end too when it goes on.
pub(super) fn compose_line(
    line: &str,
    cursor: bool,
    col: usize,
    width: usize,
    t: Theme,
    region: std::ops::Range<usize>,
    mark: Style,
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
            mark
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
        spans.push(Span::styled(" ", mark));
    }
    Line::from(spans)
}
