//! The dialogs over the screen: each a box of its own, its buttons and
//! what a click on each does.
use super::*;

/// A box `width` wide, its title on top and the keys it takes, `foot`, on
/// its bottom edge, as many as fit.
pub(super) fn dialog<'a>(t: &Theme, width: u16, title: &'a str, foot: &str) -> Block<'a> {
    let foot = fit_foot(foot, (width as usize).saturating_sub(4));
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

/// `foot`, its hints dropped whole from the end till it is `room` wide,
/// but the one for esc, kept last: how to leave is never lost.
pub(super) fn fit_foot(foot: &str, room: usize) -> String {
    if foot.width() <= room {
        return foot.to_string();
    }
    let hints: Vec<&str> = foot.split(" · ").collect();
    let esc = hints.iter().rposition(|h| h.starts_with("esc"));
    let mut used = esc.map_or(0, |e| hints[e].width() + 3);
    let mut kept = Vec::new();
    for (i, hint) in hints.iter().enumerate() {
        if Some(i) == esc {
            continue;
        }
        if used + hint.width() + 3 > room + 3 {
            break;
        }
        used += hint.width() + 3;
        kept.push(*hint);
    }
    kept.extend(esc.map(|e| hints[e]));
    match kept.as_slice() {
        [] => clip(foot, room),
        _ => clip(&kept.join(" · "), room),
    }
}

/// A line of buttons, the first lit; and where each is along it.
pub(super) fn buttons(
    t: &Theme,
    labels: &[(&str, Button)],
) -> (Line<'static>, Vec<(u16, u16, Button)>) {
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
pub(super) fn spot(area: Rect, line: usize, x: u16, width: u16) -> Rect {
    Rect {
        x: area.x + 2 + x,
        y: area.y + 1 + line as u16,
        width: width.min(area.width.saturating_sub(4 + x)),
        height: 1,
    }
}

/// The papers of the message read, two lines each: its title and where it
/// stands with paperstack, then who wrote it where. A paper clicked is put
/// in paperstack.
pub(super) fn papers_dialog(
    f: &mut Frame,
    t: &Theme,
    p: &crate::app::PapersView,
) -> (Rect, Targets) {
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
                Span::styled(format!("{:>side$}", clip(&said, side)), style),
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
            area.width,
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
pub(super) fn debrief_dialog(
    f: &mut Frame,
    t: &Theme,
    d: &crate::filter::Debrief,
) -> (Rect, Targets) {
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
        Paragraph::new(lines).block(dialog(
            t,
            area.width,
            &d.title,
            "⏎ put back · u put all back · tab looks like junk · esc ok",
        )),
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
pub(super) fn view_dialog(f: &mut Frame, t: &Theme, map: &crate::map::Map) -> (Rect, Targets) {
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
        Paragraph::new(lines).block(dialog(
            t,
            area.width,
            "View",
            "letters change it · esc closes",
        )),
        area,
    );
    let room = width.saturating_sub(4);
    let targets = (0..keymap::VIEW.len())
        .map(|i| (spot(area, 1 + i, 0, room), Target::Item(i)))
        .collect();
    (area, targets)
}

/// The account's folders, one highlighted, asking which of them is Trash.
pub(super) fn trash_dialog(f: &mut Frame, t: &Theme, pick: &TrashPick) -> (Rect, Targets) {
    let width = f.area().width.min(72);
    let room = (width as usize).saturating_sub(4);
    let intro = wrap(
        "Which of these folders should hold mail you delete? It stays there as long as its server keeps it, and d puts it back.",
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
        Paragraph::new(lines).block(dialog(t, area.width, "Trash", "⏎ this folder · esc cancel")),
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
pub(super) fn question(
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
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, area.width, title, foot)),
        area,
    );
    (area, last_at)
}

/// A question answered by a name typed: what is asked, and the field.
pub(super) fn asking(
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

pub(super) fn renaming(f: &mut Frame, t: &Theme, name: &str, typed: &str) -> (Rect, Targets) {
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

pub(super) fn removing(f: &mut Frame, t: &Theme, name: &str) -> (Rect, Targets) {
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

/// Whether to reply with the copy of a PDF attached signed in ohp, or
/// save it in a folder picked; and how saving it went.
pub(super) fn signed_dialog(f: &mut Frame, t: &Theme, signed: &Signed) -> (Rect, Targets) {
    let (row, places) = buttons(
        t,
        &[
            ("Reply", Button::Reply),
            ("Reply all", Button::ReplyAll),
            ("Save…", Button::SaveCopy),
            ("Not now", Button::NotNow),
        ],
    );
    let name = signed
        .path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut said =
        format!("Reply with {name} attached? It is kept with the message, to attach later too.");
    if let Some(note) = &signed.note {
        said = format!("{said} {note}.");
    }
    let (area, buttons_at) = question(
        f,
        t,
        "Signed",
        "⏎ r replies with it · a to all · s saves it in a folder · any other key keeps it",
        &said,
        |_| row,
    );
    let targets = places
        .into_iter()
        .map(|(x, w, b)| (spot(area, buttons_at, x, w), Target::Button(b)))
        .collect();
    (area, targets)
}

/// Whether to do what was chosen in the Inbox looked through, `said`.
pub(super) fn applying(f: &mut Frame, t: &Theme, said: &str) -> (Rect, Targets) {
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

/// How what was asked of the servers went, once it all settled: how many
/// were confirmed, and each that failed, with why.
pub(super) fn applied_dialog(
    f: &mut Frame,
    t: &Theme,
    applied: &crate::app::Applied,
) -> (Rect, Targets) {
    let width = f.area().width.min(72);
    let room = (width as usize).saturating_sub(4);
    let (title, mark, colour) = if applied.failed.is_empty() {
        ("Done", "✓ ", t.title)
    } else {
        ("Not all done", "✗ ", t.flag)
    };
    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(mark, Style::new().fg(colour).bold()),
            Span::styled(applied.said.clone(), Style::new().fg(t.title).bold()),
        ]),
    ];
    if !applied.failed.is_empty() {
        lines.push(Line::raw(""));
        for failed in &applied.failed {
            for (i, l) in wrap(failed, room.saturating_sub(2)).into_iter().enumerate() {
                let mark = if i == 0 { "✗ " } else { "  " };
                lines.push(Line::from(vec![
                    Span::styled(mark, Style::new().fg(t.flag)),
                    Span::raw(l),
                ]));
            }
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "What failed is left where it was; in the Inbox looked through, as it was chosen, for $ again.",
            Style::new().fg(t.dim),
        ));
    }
    let lines: Vec<Line> = lines
        .into_iter()
        .flat_map(|l| {
            // A long last line wrapped, as the rest are.
            if l.width() > room && l.spans.len() == 1 {
                let style = l.spans[0].style;
                wrap(&l.spans[0].content, room)
                    .into_iter()
                    .map(|w| Line::styled(w, style))
                    .collect()
            } else {
                vec![l]
            }
        })
        .collect();
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, area.width, title, "any key closes")),
        area,
    );
    (area, Vec::new())
}

/// Whether to trash or archive what is marked, all of it: how many, by
/// account and folder, and what is left out, with the key that takes it.
pub(super) fn bulk_dialog(
    f: &mut Frame,
    t: &Theme,
    said: &crate::app::BulkSaid,
) -> (Rect, Targets) {
    let width = f.area().width.min(72);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![Line::raw("")];
    for l in &said.lines {
        lines.push(Line::raw(clip(l, room)));
    }
    if !said.warnings.is_empty() {
        lines.push(Line::raw(""));
        for w in &said.warnings {
            lines.push(Line::from(vec![
                Span::styled("⚠ ", Style::new().fg(t.flag)),
                Span::styled(clip(w, room.saturating_sub(2)), Style::new().fg(t.flag)),
            ]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "y does it · any other key does nothing, what is marked kept",
        Style::new().fg(t.dim),
    ));
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, area.width, &said.title, "y does it · esc not")),
        area,
    );
    (area, Vec::new())
}

/// New mail letterbox is sure is junk: each, by whom and what, and how
/// sure, the one picked lit, those chosen `[x]`; nothing thrown away unless
/// chosen.
pub(super) fn junk_dialog(
    f: &mut Frame,
    t: &Theme,
    pick: &crate::app::JunkPick,
) -> (Rect, Targets) {
    let width = f.area().width.min(84);
    let room = (width as usize).saturating_sub(4);
    // Two lines each: it, and why it is thought junk.
    let shown = ((f.area().height as usize).saturating_sub(10) / 2).clamp(2, 8);
    let top = pick.at.saturating_sub(shown.saturating_sub(1));
    let mut lines = vec![Line::raw("")];
    for (k, j) in pick.items.iter().enumerate().skip(top).take(shown) {
        let mark = if j.chosen { "[x] " } else { "[ ] " };
        let odds = format!("{:>3.0}%  ", j.odds * 100.0);
        let from = clip(&j.from, 20);
        let rest = room.saturating_sub(mark.width() + odds.width() + from.width() + 2);
        let lit = if k == pick.at {
            Style::new().bg(t.focus)
        } else {
            Style::new()
        };
        lines.push(
            Line::from(vec![
                Span::styled(mark, Style::new().fg(t.flag)),
                Span::styled(odds, Style::new().fg(t.dim)),
                Span::styled(from, Style::new().fg(t.title).bold()),
                Span::raw("  "),
                Span::raw(clip(&j.subject, rest)),
            ])
            .style(lit),
        );
        let why = format!("         {}", j.why.join(" · "));
        lines.push(Line::styled(clip(&why, room), Style::new().fg(t.dim)).style(lit));
    }
    let chosen = pick.items.iter().filter(|j| j.chosen).count();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            "{chosen} chosen of {}. Nothing is thrown away unless you choose it: space chooses the one \
             picked, d throws those chosen away; n says the one picked is not junk, and it is learnt so; \
             i looks through the Inbox; any other key leaves them.",
            pick.items.len()
        ),
        Style::new().fg(t.dim),
    ));
    let lines: Vec<Line> = lines
        .into_iter()
        .flat_map(|l| {
            if l.width() > room && l.spans.len() == 1 {
                let style = l.spans[0].style;
                wrap(&l.spans[0].content, room)
                    .into_iter()
                    .map(|w| Line::styled(w, style))
                    .collect()
            } else {
                vec![l]
            }
        })
        .collect();
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(
            t, area.width,
            &format!("{} new look like junk", pick.items.len()),
            "␣ choose · d throw away chosen · n not junk · i look through · tab thrown away · esc leave",
        )),
        area,
    );
    (area, Vec::new())
}

/// A label split into subfolders, shown before it is done: each part, as
/// `Label/name`, how many go there, and the words it was named by; the one
/// picked lit, its name typed over if it is being.
pub(super) fn split_dialog(
    f: &mut Frame,
    t: &Theme,
    plan: &crate::app::SplitPlan,
) -> (Rect, Targets) {
    let width = f.area().width.min(84);
    let room = (width as usize).saturating_sub(4);
    let field = Style::new().fg(t.text).bg(t.field);
    let mut lines = vec![Line::raw("")];
    for (k, p) in plan.parts.iter().enumerate() {
        let lit = if k == plan.at {
            Style::new().bg(t.focus)
        } else {
            Style::new()
        };
        let name = match (&plan.typing, k == plan.at) {
            (Some(typed), true) => Span::styled(format!("{typed}▏"), field),
            _ => Span::styled(p.name.clone(), Style::new().fg(t.title).bold()),
        };
        lines.push(
            Line::from(vec![
                Span::raw(format!("{:>5}  ", p.keys.len())),
                Span::styled(format!("{}/", plan.label), Style::new().fg(t.dim)),
                name,
            ])
            .style(lit),
        );
        let words = format!("       {}", p.words.join(" · "));
        lines.push(Line::styled(clip(&words, room), Style::new().fg(t.dim)).style(lit));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            "Each part is moved to a subfolder of {}, made for it, by what it is about. Nothing is \
             done until y; C-x u moves it all back.",
            plan.label
        ),
        Style::new().fg(t.dim),
    ));
    let lines = wrapped(lines, room);
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    let total: usize = plan.parts.iter().map(|p| p.keys.len()).sum();
    f.render_widget(
        Paragraph::new(lines).block(dialog(
            t,
            area.width,
            &format!("Split {} ({total}) into {}?", plan.label, plan.parts.len()),
            "↑ ↓ pick · ⏎ rename · y split · esc not",
        )),
        area,
    );
    (area, Vec::new())
}

/// A label removed, shown before it is done: where its mail would go, as
/// the map would file each, else unfiled, to Archive.
pub(super) fn remove_label_dialog(
    f: &mut Frame,
    t: &Theme,
    plan: &crate::app::RemovePlan,
) -> (Rect, Targets) {
    let width = f.area().width.min(76);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![Line::raw("")];
    for (to, keys) in &plan.to {
        let name = match to {
            Some(label) => format!("→ {label}"),
            None => "→ Archive, unfiled".to_string(),
        };
        lines.push(Line::from(vec![
            Span::raw(format!("{:>5}  ", keys.len())),
            Span::styled(name, Style::new().fg(t.title).bold()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            "No message is thrown away: each is filed where the map would file it, were there no {}, \
             and {} is removed once it is empty. Nothing is done until y; C-x u puts it all back.",
            plan.label, plan.label
        ),
        Style::new().fg(t.dim),
    ));
    let lines = wrapped(lines, room);
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    let total: usize = plan.to.iter().map(|(_, k)| k.len()).sum();
    f.render_widget(
        Paragraph::new(lines).block(dialog(
            t,
            area.width,
            &format!("Remove {} ({total})?", plan.label),
            "y remove · esc not",
        )),
        area,
    );
    (area, Vec::new())
}

/// `lines`, each of one span wider than `room` wrapped to it.
fn wrapped(lines: Vec<Line<'static>>, room: usize) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .flat_map(|l| {
            if l.width() > room && l.spans.len() == 1 {
                let style = l.spans[0].style;
                wrap(&l.spans[0].content, room)
                    .into_iter()
                    .map(|w| Line::styled(w, style))
                    .collect()
            } else {
                vec![l]
            }
        })
        .collect()
}
