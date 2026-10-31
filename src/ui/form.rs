//! The form an account is added with, drawn: each field, the one typed in
//! lit, and what is to be known of the provider.
use super::*;

/// The form an account is added with: each field on a line, the one typed
/// in lit; the password shown as dots; where it is kept, picked; what to
/// know, or what went wrong, below; and its buttons.
pub(super) fn form_dialog(f: &mut Frame, t: &Theme, form: &Form) -> (Rect, Targets) {
    let width = f.area().width.min(72);
    let room = (width as usize).saturating_sub(4);
    let value_w = room.saturating_sub(KEY + 1);
    let mut lines = vec![Line::raw("")];
    let shown = form.shown();
    for &i in &shown {
        let name = FIELDS[i];
        let here = i == form.at && !form.checking;
        let label = Span::styled(
            format!("{name:<KEY$}"),
            if here {
                Style::new().fg(t.title).bold()
            } else {
                Style::new().fg(t.dim)
            },
        );
        let pick = |picked: bool| {
            if picked {
                Style::new()
                    .fg(t.title)
                    .bg(if here { t.focus } else { t.row })
                    .bold()
            } else {
                Style::new().fg(t.dim)
            }
        };
        if i == SIGN_IN {
            let mut spans = vec![label, Span::raw(" ")];
            for (auth, name) in [(Auth::Password, "password"), (Auth::Microsoft, "microsoft")] {
                spans.push(Span::styled(format!(" {name} "), pick(form.auth == auth)));
                spans.push(Span::raw(" "));
            }
            lines.push(Line::from(spans));
            continue;
        }
        if i == KEEP {
            let mut spans = vec![label, Span::raw(" ")];
            for (k, keeper) in form.keepers.iter().enumerate() {
                let style = if k == form.keeper {
                    Style::new()
                        .fg(t.title)
                        .bg(if here { t.focus } else { t.row })
                        .bold()
                } else {
                    Style::new().fg(t.dim)
                };
                spans.push(Span::styled(format!(" {} ", keeper.name()), style));
                spans.push(Span::raw(" "));
            }
            if form.keepers.is_empty() {
                spans.push(Span::styled("nowhere here", Style::new().fg(t.error)));
            }
            lines.push(Line::from(spans));
            continue;
        }
        let value = if [PASSWORD, PASSPHRASE, AGAIN].contains(&i) {
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
            label,
            Span::styled(
                format!(" {value}{}", " ".repeat(pad)),
                Style::new()
                    .fg(t.text)
                    .bg(if here { t.field } else { t.page }),
            ),
        ]));
    }
    lines.push(Line::raw(""));
    let note_style = if form.failed.is_some() {
        Style::new().fg(t.error)
    } else {
        Style::new().fg(t.soft)
    };
    // How to make an app password, where the provider takes one: said
    // with the key that opens its page.
    let help = config::app_password_help(&form.fields[EMAIL], &form.fields[SERVER])
        .filter(|_| form.auth == Auth::Password)
        .map(|(note, _)| format!("{note} ^o opens the page."));
    let guiding = form.failed.is_none() && !form.checking;
    if let Some(note) = help.as_ref().filter(|_| guiding).or(form.note.as_ref()) {
        lines.extend(
            wrap(note, room)
                .into_iter()
                .map(|l| Line::styled(l, note_style)),
        );
    }
    if let Some(w) = &form.waiting {
        lines.push(Line::styled(
            "Sign in on Microsoft's page, opened in your browser.",
            Style::new().fg(t.text),
        ));
        if let Some((page, code)) = &w.device {
            lines.extend(
                wrap(&format!("Or, on any device, open {page} and enter"), room)
                    .into_iter()
                    .map(|l| Line::styled(l, Style::new().fg(t.text))),
            );
            lines.push(Line::styled(
                format!("    {code}"),
                Style::new().fg(t.accent).bold(),
            ));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "If the browser did not open, open this there:",
            Style::new().fg(t.faint),
        ));
        lines.extend(
            wrap(&w.page, room)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(t.faint))),
        );
    } else if let Some(help) = help.filter(|_| form.failed == Some(Failure::Refused)) {
        // Refused, why takes the help's place above: it is said again here,
        // where what is kept is said otherwise.
        lines.extend(
            wrap(&help, room)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(t.soft))),
        );
    } else {
        let secret = match form.auth {
            Auth::Password => "The password",
            Auth::Microsoft => "Microsoft's token",
        };
        let mut about = format!(
            "It signs in before it is added. {secret} is kept {}, never in the accounts file.",
            form.kept().map_or("nowhere", Keeper::about)
        );
        if shown.contains(&AGAIN) {
            about.push_str(&format!(
                " The vault is new, at {}: its passphrase is asked for each time letterbox \
                 starts.",
                tilde(&vault::path())
            ));
        }
        lines.extend(
            wrap(&about, room)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(t.faint))),
        );
    }
    lines.push(Line::raw(""));
    let (row, places) = if form.checking {
        buttons(t, &[("Cancel", Button::Cancel)])
    } else {
        buttons(
            t,
            &[("Sign in and add", Button::Add), ("Cancel", Button::Cancel)],
        )
    };
    let buttons_at = lines.len();
    lines.push(row);
    let foot = if form.waiting.is_some() {
        "waiting for Microsoft… · esc stops"
    } else if form.checking {
        "signing in…"
    } else {
        "tab next field · ←→ pick · ⏎ on the last adds · esc cancel"
    };
    let area = centered(f.area(), width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(dialog(t, area.width, "Add account", foot)),
        area,
    );
    let mut targets: Targets = shown
        .iter()
        .enumerate()
        .map(|(line, &i)| (spot(area, 1 + line, 0, room as u16), Target::Field(i)))
        .collect();
    targets.extend(
        places
            .into_iter()
            .map(|(x, w, b)| (spot(area, buttons_at, x, w), Target::Button(b))),
    );
    (area, targets)
}
