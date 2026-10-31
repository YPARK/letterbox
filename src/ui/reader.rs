//! The message read: its header, its text wrapped, or its pages; on the
//! map, where it lies and what is near it.
use super::*;

/// The message read as its pages, if it is shown so: the page shown, as
/// wide as the pane, and which it is below it. Whether it was.
#[cfg(feature = "pages")]
pub(super) fn paged(f: &mut Frame, app: &mut App, area: Rect) -> bool {
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

/// The message read, its line to drag on the `edge` it has.
pub(super) fn reader(f: &mut Frame, app: &mut App, area: Rect, edge: Borders) {
    let t = app.theme;
    let block = Block::new()
        .borders(edge)
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
    app.page_room = (inner.width, inner.height.saturating_sub(1));
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
pub(super) fn mapped(app: &App, width: usize) -> Option<(Vec<Line<'static>>, Vec<Line<'static>>)> {
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
pub(super) fn message(m: &Message, width: usize, t: &Theme) -> Vec<Line<'static>> {
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
