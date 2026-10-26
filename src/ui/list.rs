//! The search, the sidebar of accounts and folders, and the list of
//! messages, or what an empty one says.
use super::*;

pub(super) fn search(f: &mut Frame, app: &mut App, area: Rect) {
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
pub(super) fn rows(area: Rect, offset: usize, heights: &[u16]) -> Vec<(usize, Rect)> {
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

pub(super) fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
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
pub(super) fn map_row(
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
pub(super) struct FolderRow<'a> {
    pub(super) depth: usize,
    pub(super) role: Role,
    pub(super) name: &'a str,
    pub(super) unseen: Option<u32>,
    pub(super) selectable: bool,
    pub(super) open: bool,
}

pub(super) fn folder_row(t: Theme, width: usize, row: FolderRow) -> ListItem<'static> {
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

pub(super) fn messages(f: &mut Frame, app: &mut App, area: Rect) {
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

/// What an empty list says: how to set up an account, that the account is
/// being reached or could not be, or that there is nothing to show.
pub(super) fn empty(f: &mut Frame, app: &App, area: Rect) {
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
