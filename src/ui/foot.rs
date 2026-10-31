//! The foot's keys, as many as fit, and the help, every key.
use super::*;

/// The foot: what happened, at the right of the keys where there is room,
/// else on a line of its own above them; the keys; and where each key's
/// hint is, by its line, how far along and how wide, to be clicked.
pub(super) fn foot(app: &App, width: u16) -> (Vec<Line<'static>>, Vec<Spot>) {
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
            // The key in the accent's colour, bold, on a block of the
            // page's, to stand out from the foot's tint.
            spans.push(Span::styled(k, Style::new().fg(t.accent).bg(t.page).bold()));
            spans.push(Span::styled(what, Style::new().fg(t.soft)));
        }
        lines.push(Line::from(spans));
    }
    // While what was asked of the servers is waited for, how far it has
    // come, over anything else said.
    let (said, style) = if app.batch.busy() {
        let said = match app.status.is_empty() {
            true => app.batch.said(),
            false => format!("{} · {}", app.batch.said(), app.status),
        };
        (said, Style::new().fg(t.flag).bold())
    } else {
        // What is marked said while it is, as d and e act on all of it,
        // in the flag's colour when it is a lot; and what looks like junk,
        // while any does, for J to show.
        let marked = app.map.region.as_ref().map_or(0, |r| r.keys.len());
        let junk = app.junk_held();
        let mut parts = Vec::new();
        if junk > 0 {
            parts.push(format!("◆ {junk} look like junk · J"));
        }
        if marked > 0 {
            parts.push(format!("▪ {marked} marked"));
        }
        if !app.status.is_empty() {
            parts.push(app.status.clone());
        }
        let colour = if marked > app.mark_big || junk > 0 {
            t.flag
        } else {
            t.text
        };
        (parts.join(" · "), Style::new().fg(colour).bold())
    };
    if said.is_empty() {
        return (lines, spots);
    }
    let status = Span::styled(format!("{said} "), style);
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
pub(super) fn keys(
    f: &mut Frame,
    app: &mut App,
    area: Rect,
    lines: Vec<Line<'static>>,
    spots: Vec<Spot>,
) {
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
pub(super) fn hints(app: &App, width: u16) -> Vec<Vec<(String, String)>> {
    let pane = match app.focus {
        _ if matches!(app.dialog, Dialog::Trash(_)) => Pane::Picking,
        _ if matches!(app.dialog, Dialog::Attach(_)) => Pane::Attaching,
        _ if matches!(&app.dialog, Dialog::SaveTo(s) if s.naming.is_some()) => Pane::Naming,
        _ if matches!(app.dialog, Dialog::SaveTo(_)) => Pane::Saving,
        _ if matches!(&app.dialog, Dialog::Signed(s) if s.saving.is_some()) => Pane::Saving,
        _ if matches!(app.dialog, Dialog::View) => Pane::Viewing,
        _ if app.compose.is_some() => Pane::Writing,
        _ if app.searching => Pane::Searching,
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
    in_context(app, pane, &mut list);
    let mut lines = lay(&list, width);
    while lines.len() > FOOT
        && let Some(last) = list.iter().rposition(|h| !h.kept)
    {
        list.remove(last);
        lines = lay(&list, width);
    }
    lines
}

/// The hints as each key does here: < and > left out where they size
/// nothing, d's said as it puts back in Trash, ⏎'s as the search goes
/// on the map or to the server; esc, the way back from the message read,
/// kept with help and quit.
fn in_context(app: &App, pane: Pane, list: &mut Vec<keymap::Hint>) {
    let map = app.map.mode == crate::map::Mode::Map;
    let trash = app.picked_in_trash();
    list.retain(|h| !matches!(h.act, Some(keymap::Act::Wider)) || app.resizable());
    for h in list {
        match (h.act, pane) {
            (Some(keymap::Act::Trash), Pane::List | Pane::Reader) if trash => h.what = "put back",
            (Some(keymap::Act::Open), Pane::Searching) if map => h.what = "light it on the map",
            (Some(keymap::Act::Open), Pane::Searching) if !app.searches_everywhere() => {
                h.what = "search the server"
            }
            (Some(keymap::Act::Back), Pane::Reader) => h.kept = true,
            _ => {}
        }
    }
}

pub(super) fn lay(list: &[keymap::Hint], width: u16) -> Vec<Vec<(String, String)>> {
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

/// Every key, in a pop-up scrolled to `scroll`: those of the message
/// being written first, while one is.
pub(super) fn help(f: &mut Frame, t: &Theme, scroll: usize, writing: bool) -> (Rect, usize) {
    let rows: Vec<(&str, &str)> = if writing {
        keymap::help_writing().collect()
    } else {
        keymap::help().collect()
    };
    let rows: Vec<(&str, &str)> = rows
        .into_iter()
        .chain([("click", "pick; the wheel scrolls")])
        .collect();
    // Each key's line wrapped under itself, past the keys' column: as wide
    // as the widest keys, and a space.
    let key = rows
        .iter()
        .map(|(k, _)| k.width() + 1)
        .max()
        .unwrap_or(KEY)
        .max(KEY);
    let width = f.area().width.min(80);
    let room = (width as usize).saturating_sub(4 + key);
    let mut lines: Vec<Line> = Vec::new();
    for (k, what) in rows {
        for (i, l) in wrap(what, room).into_iter().enumerate() {
            let k = if i == 0 { k } else { "" };
            lines.push(Line::from(vec![
                Span::styled(format!("{k:<key$}"), Style::new().fg(t.title).bold()),
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
