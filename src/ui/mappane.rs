//! The map in the list's place: what it shows said above it, the message
//! picked and how faithful it is below, and a cluster gone into, as its
//! parts or, below the finest, as a tree of its threads.
use super::*;

/// The map, in the list's place: a legend above, the clusters as a
/// mosaic, or one gone into, its parts as one or its threads as a tree,
/// and how faithful it is below.
pub(super) fn map_pane(f: &mut Frame, app: &mut App, area: Rect) {
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
    if app.map.hits(&query).is_some() {
        let n = app.map.found(&query, now).len();
        let said = match app.map.searched_in() {
            Some(name) => format!(" · {n} found in {name} for “{}”", query.trim()),
            None => format!(" · {n} found for “{}”", query.trim()),
        };
        head.push(Span::styled(said, Style::new().fg(t.title).bold()));
    }
    // What it is narrowed to, next, before what is cut off first.
    let narrowed = match app.map.focused {
        Focused::Cluster(_) if app.map.tree.is_some() => None,
        Focused::Cluster(id) => s.group(id).map(|g| format!(" · in {}", g.name())),
        Focused::Loners => Some(" · fitting nowhere".to_string()),
        Focused::All => None,
    };
    if let Some(n) = narrowed {
        head.push(Span::styled(n, Style::new().fg(t.title).bold()));
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
        // Laid out again to no better, it is as faithful as it gets.
        Some(faith) if faith < 0.7 && app.map.no_nearer(faith) => {
            format!(
                " {:.0}% faithful: laid out again, no nearer; the blocks lie by those alike",
                faith * 100.0
            )
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
pub(super) fn laid_out(app: &App, s: &crate::map::Snapshot) -> Option<Vec<crate::model::Point>> {
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
pub(super) fn heated(t: &Theme, heat: f32) -> ratatui::style::Color {
    crate::theme::mix(t.faint, t.flag, 0.3 + 0.7 * heat)
}

/// Where the Inbox looked through goes, said: to each label, the most
/// first, to the Trash, and kept.
pub(super) fn heading_said(h: &crate::app::Heading, s: &crate::map::Snapshot) -> String {
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
pub(super) fn tree_pane(
    f: &mut Frame,
    app: &mut App,
    s: &crate::map::Snapshot,
    canvas: Rect,
    now: i64,
) {
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
