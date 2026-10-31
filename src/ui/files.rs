//! The dialogs of a message's files: those to open or save, the folder
//! to save them in, or files to attach, and the name to save one under.
use super::*;

/// The message's attachments and links: each on a line, the one picked
/// lit, with what ⏎ does with it.
pub(super) fn files_dialog(f: &mut Frame, t: &Theme, files: &Files) -> (Rect, Targets) {
    let width = f.area().width.min(96);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![Line::raw("")];
    for (i, item) in files.items.iter().enumerate() {
        let (icon, name, what) = match item {
            Item::Page => ("◳", "this message".to_string(), "browser".to_string()),
            Item::Present => ("◳", "this message".to_string(), "ohp".to_string()),
            Item::File {
                name,
                mime,
                size: bytes,
                ..
            } => {
                let kind = mime.rsplit('/').next().unwrap_or(mime).to_uppercase();
                let what = match (files.saving, ohp_reads(name)) {
                    (true, _) => format!("{kind} · {}", size(*bytes)),
                    (false, true) => format!("{kind} · {} · ohp", size(*bytes)),
                    (false, false) => format!("{kind} · {} · open", size(*bytes)),
                };
                ("▤", name.clone(), what)
            }
            Item::Link { url, name } => {
                let what = match crate::net::pdf_of(url) {
                    Some(_) => "PDF · ohp",
                    None => "browser",
                };
                let (said, kind) = link_label(url, name.as_deref(), what);
                ("↗", said, kind)
            }
        };
        let here = i == files.at;
        // The name has what the kind leaves.
        let what = clip(&what, room / 2);
        let name_room = room.saturating_sub(what.width() + 4);
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
                Span::styled(what, Style::new().fg(t.dim)),
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
        Paragraph::new(lines).block(if files.saving {
            dialog(t, area.width, "Save", "⏎ save · a save all · esc close")
        } else {
            dialog(
                t,
                area.width,
                "Open",
                "⏎ open · s save · S save all · y copy address · esc close",
            )
        }),
        area,
    );
    let targets = (0..files.items.len())
        .map(|i| (spot(area, 1 + i, 0, room as u16), Target::Item(i)))
        .collect();
    (area, targets)
}

/// Files to attach, picked a folder at a time: the folder, what is typed,
/// and its entries, the one picked lit, those marked `●`, the newest `★`.
/// Or the folder to save in, picked so: its folders, and `.` to save there.
pub(super) fn attach_dialog(f: &mut Frame, t: &Theme, p: &crate::pick::Picker) -> (Rect, Targets) {
    use crate::pick::Entry;
    let width = f.area().width.min(84);
    let height = f.area().height.saturating_sub(4).clamp(8, 30);
    let room = (width as usize).saturating_sub(4);
    let mut lines = vec![
        Line::styled(
            // Its end in sight: the folder it is in.
            tail(&folder(&p.dir), room),
            Style::new().fg(t.dim),
        ),
        Line::from(vec![
            Span::styled("narrow ", Style::new().fg(t.dim)),
            Span::styled(
                tail(&p.filter, room.saturating_sub(9)),
                Style::new().fg(t.text),
            ),
            Span::styled("▏", Style::new().fg(t.accent)),
        ]),
        Line::styled(
            clip(
                if p.folders {
                    "type to narrow, or a path from /; ⏎ on . saves here"
                } else {
                    "type to narrow, or a path from /; ␣ marks several to attach"
                },
                room,
            ),
            Style::new().fg(t.faint),
        ),
        Line::raw(""),
    ];
    let top = lines.len();
    let rows = (height as usize).saturating_sub(2 + top + 2).max(1);
    let shown = p.shown();
    let offset = (p.row + 1).saturating_sub(rows);
    for (i, e) in shown.iter().enumerate().skip(offset).take(rows) {
        let (mark, name, about, style) = match e {
            Entry::Up => (
                "  ",
                "..".to_string(),
                String::new(),
                Style::new().fg(t.dim),
            ),
            Entry::Here => {
                let here = p.here();
                let about = if here == p.dir {
                    "save here".to_string()
                } else {
                    format!("make {} and save", folder(&here))
                };
                (
                    "  ",
                    "./".to_string(),
                    about,
                    Style::new().fg(t.accent).bold(),
                )
            }
            Entry::Dir(n) => (
                "  ",
                format!("{n}/"),
                String::new(),
                Style::new().fg(t.title),
            ),
            Entry::File { name, size: bytes } => {
                let mark = if p.marked.contains(&p.dir.join(name)) {
                    "● "
                } else if p.newest.as_deref() == Some(name) {
                    "★ "
                } else {
                    "  "
                };
                (
                    mark,
                    name.clone(),
                    size(*bytes as usize),
                    Style::new().fg(t.text),
                )
            }
        };
        let side = about.width();
        let name = clip(&name, room.saturating_sub(side + 4));
        let pad = room.saturating_sub(2 + name.width() + side);
        let line = Line::from(vec![
            Span::styled(mark, Style::new().fg(t.accent)),
            Span::styled(name, style),
            Span::raw(" ".repeat(pad)),
            Span::styled(about, Style::new().fg(t.dim)),
        ]);
        lines.push(if i == p.row {
            line.style(Style::new().bg(t.focus))
        } else {
            line
        });
    }
    while lines.len() < top + rows {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(""));
    let note = match (&p.note, p.marked.len()) {
        (Some(note), _) => Line::styled(clip(note, room), Style::new().fg(t.error)),
        (None, 0) => Line::raw(""),
        (None, n) => Line::styled(
            format!("{n} marked: ⏎ attaches them"),
            Style::new().fg(t.soft),
        ),
    };
    lines.push(note);
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(if p.folders {
            dialog(
                t,
                area.width,
                "Save to",
                "⏎ into · ⏎ on . save · ← up · ~ home · esc",
            )
        } else {
            dialog(
                t,
                area.width,
                "Attach files",
                "⏎ attach · ␣ mark · C-a all · ← up · ~ home · esc",
            )
        }),
        area,
    );
    let targets = (offset..shown.len().min(offset + rows))
        .map(|i| {
            (
                spot(area, top + i - offset, 0, room as u16),
                Target::Item(i),
            )
        })
        .collect();
    (area, targets)
}

/// The name to save a file under, in `dir`: the part picked lit, to be
/// typed over, else the cursor where it is.
pub(super) fn save_as(
    f: &mut Frame,
    t: &Theme,
    dir: &std::path::Path,
    name: &crate::app::Name,
) -> (Rect, Targets) {
    let field = Style::new().fg(t.text).bg(t.field);
    let lit = Style::new().fg(t.on_accent).bg(t.accent);
    let asked = format!("In {}, saved as:", folder(dir));
    let (area, _) = question(
        f,
        t,
        "Save as",
        "⏎ save · ← → edit · esc back",
        &asked,
        |room| {
            let chars: Vec<char> = name.text.chars().collect();
            let (a, b) = name.range().unwrap_or((name.at, name.at));
            let part = |r: std::ops::Range<usize>| chars[r].iter().collect::<String>();
            let (before, picked, after) = (part(0..a), part(a..b), part(b..chars.len()));
            let cursor = if picked.is_empty() { "▏" } else { "" };
            let used = before.width() + picked.width() + after.width() + cursor.width();
            // Its end shown, if it is longer than the field.
            let skip = used.saturating_sub(room);
            let before = before.chars().skip(skip).collect::<String>();
            let pad = " ".repeat(room.saturating_sub(used));
            Line::from(vec![
                Span::styled(before, field),
                Span::styled(cursor, field.fg(t.accent)),
                Span::styled(picked, lit),
                Span::styled(format!("{after}{pad}"), field),
            ])
        },
    );
    (area, Vec::new())
}

/// `dir` as a folder is named: from `~` in home, a `/` after it, once.
pub(super) fn folder(dir: &std::path::Path) -> String {
    let named = crate::compose::named(dir);
    if named.ends_with('/') {
        named
    } else {
        format!("{named}/")
    }
}

/// A link as the dialog shows it: its name, else its address; and beside
/// it, named, the host it truly goes to, as a text may name another
/// address than its own; then what opens it.
pub(crate) fn link_label(url: &str, name: Option<&str>, what: &str) -> (String, String) {
    match name {
        Some(name) => {
            let to = crate::papers::unwrapped(url);
            let host = to.split("://").nth(1).unwrap_or(&to);
            let host = host.split(['/', '?', '#']).next().unwrap_or(host);
            let host = host.trim_start_matches("www.");
            (name.to_string(), format!("{host} · {what}"))
        }
        None => (url.to_string(), what.to_string()),
    }
}
