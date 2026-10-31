//! The Inbox looked through, beside the map: each message, what may be
//! done with it, and the labels dropped down under one.
use super::*;

/// The Inbox looked through, beside the map: each message, newest first,
/// with what may be done with it, what will be done lit; and a button that
/// does it all.
pub(super) fn review_pane(f: &mut Frame, app: &mut App, area: Rect) {
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
    let button = " Apply  C-c C-c ";
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
        // As the foot's hint of it is: the keys the table names.
        if let Some((_, _, keys)) = keymap::pressed("C-c C-c").into_iter().next()
            && let [a, b] = keys[..]
        {
            app.targets.push((
                Rect::new(bx, area.y, button.width() as u16, 1),
                Target::Chord(a, b),
            ));
        }
    }
    // Only those chosen are done; the rest are left as they are.
    let counts = if filed + discarded + kept == 0 {
        " none chosen: look through as many as you like; C-c C-c does only those".to_string()
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
    // What is selected with the mouse, as the one picked is when
    // the pane is not looked at.
    let selected: std::collections::HashSet<usize> = app
        .review
        .iter()
        .flat_map(|r| r.selected.iter().filter_map(|k| s.find(k)))
        .collect();
    for (row, &i) in rows.iter().enumerate().skip(top).take(room) {
        if Some(i) == menu_for {
            menu_at = Some(y + 1);
        }
        let d = &s.dots[i];
        let is_picked = Some(i) == picked;
        let ground = if is_picked && focused {
            t.focus
        } else if is_picked || selected.contains(&i) {
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
        let failed = app.batch.failed().any(|j| j.key.as_ref() == Some(&d.key));
        let mark = if failed {
            "✗ "
        } else if d.flags.seen {
            "  "
        } else {
            "● "
        };
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
            // Another account's folder, by its name and the account's.
            Choice::Bring(to, path) => {
                let account = &app.accounts[*to];
                let name = account
                    .folders
                    .iter()
                    .find(|f| &f.path == path)
                    .map_or(path.as_str(), |f| f.name.as_str());
                Some(format!("{name} · {}", account.name))
            }
            _ => s.suggestion(i).map(str::to_string),
        };
        let options = [
            (
                match &label {
                    Some(l) => format!("→ {}", clip(l, 18)),
                    None => "move to…".to_string(),
                },
                matches!(chosen, Some(Choice::Move(_) | Choice::Bring(..))),
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
pub(super) fn label_menu(f: &mut Frame, app: &mut App, area: Rect, under: u16) {
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
            Pick::Elsewhere(to, _, name) => (
                format!(" {name} · {}", app.accounts[*to].name),
                "↦ ".to_string(),
            ),
            Pick::OnMap => (" ◇ pick on the map…".to_string(), String::new()),
        };
        let said = clip(&said, room.saturating_sub(far.width()));
        let gap = room.saturating_sub(said.width() + far.width());
        let text = format!("{said}{}{far}", " ".repeat(gap));
        let style = if k == menu.at {
            Style::new().fg(t.title).bg(t.focus).bold()
        } else if matches!(item, Pick::Label(..) | Pick::Elsewhere(..)) {
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
