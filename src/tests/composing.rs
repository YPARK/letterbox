//! Writing a message, as it is first met: what tab, C-a and the C-c keys
//! do in it, what its grammar check looks at, and what it shows in a
//! narrow window.

use super::*;
use crate::edit::{self, Buffer, Done};
use crate::grammar;

/// `buffer` with `keys` pressed in it, as `app::keys` names them: what the
/// last did.
fn typed(buffer: &mut Buffer, keys: &[&str]) -> Done {
    let mut prefix = None;
    let mut done = Done::Stay;
    for key in keys.iter().flat_map(|k| app::keys(k)) {
        done = edit::handle(buffer, &mut prefix, key);
    }
    done
}

fn written(app: &App) -> String {
    app.compose.as_ref().unwrap().buffer.text()
}

#[test]
fn the_grammar_check_leaves_out_the_message_forwarded() {
    let text = format!(
        "To: bob@x.org\nSubject: Fwd: the paper\n\nHere it is, i think.\n\n{}\nFrom: Alan Turing <alan@bletchley.example>\nDate: Thursday 29 October 2026, 12:45 PM\n\nThe paper is attached.\n",
        crate::compose::FORWARD_LINE
    );
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let said: Vec<String> = grammar::pieces(&lines)
        .into_iter()
        .map(|p| p.text)
        .collect();
    assert_eq!(said, ["Here it is, i think."]);
}

#[test]
fn after_a_send_warning_c_c_c_c_sends_without_asking_the_grammar_again() {
    for answer in ["i", "n"] {
        let mut app = demo();
        app.check_on_send = true;
        app.send_hold = Duration::from_secs(10);
        app.grammar_open = |_| {
            Ok(Box::new(grammar::Using(|s: &str| {
                Ok(s.replace("thanks i", "Thanks, I"))
            })))
        };
        press(
            &mut app,
            &["c", "bob@x.org", "<tab>", "<tab>", "<tab>", "thanks i will"],
        );
        press(&mut app, &["<c-c>", "<c-c>"]);
        assert!(app.checking.is_some(), "{}", app.status);
        press(&mut app, &[answer, "y"]);
        assert!(app.status.contains("no subject"), "{}", app.status);
        press(&mut app, &["<c-c>", "<c-c>"]);
        assert!(app.checking.is_none(), "{}", app.status);
        assert!(
            app.compose.as_ref().unwrap().held.is_some(),
            "{}",
            app.status
        );
    }
    // Another key first, and the grammar is asked again.
    let mut app = demo();
    app.check_on_send = true;
    app.grammar_open = |_| {
        Ok(Box::new(grammar::Using(|s: &str| {
            Ok(s.replace("thanks i", "Thanks, I"))
        })))
    };
    press(
        &mut app,
        &["c", "bob@x.org", "<tab>", "<tab>", "<tab>", "thanks i will"],
    );
    press(&mut app, &["<c-c>", "<c-c>", "i", "y", "!"]);
    press(&mut app, &["<c-c>", "<c-c>"]);
    assert!(app.checking.is_some(), "{}", app.status);
}

#[test]
fn tab_passes_over_the_attach_lines() {
    let mut buffer = Buffer::from_text(
        "To: \nCc: \nSubject: Fwd: the paper\nAttach: a.txt (forwarded)\nAttach: b.pdf\n\nbody\n",
    );
    buffer.place(2, 0);
    typed(&mut buffer, &["<tab>"]);
    assert_eq!(buffer.row(), 6);
    typed(&mut buffer, &["<tab>"]);
    assert_eq!(buffer.row(), 0);
    typed(&mut buffer, &["<s-tab>"]);
    assert_eq!(buffer.row(), 6);
    typed(&mut buffer, &["<s-tab>"]);
    assert_eq!(buffer.row(), 2);
    // From an Attach line, on to what is written.
    buffer.place(3, 0);
    typed(&mut buffer, &["<tab>"]);
    assert_eq!(buffer.row(), 6);
    // And back from one, to the field just above.
    buffer.place(4, 0);
    typed(&mut buffer, &["<s-tab>"]);
    assert_eq!(buffer.row(), 2, "Subject");
}

#[test]
fn c_a_in_a_header_goes_past_its_name_as_message_mode_does() {
    let mut buffer = Buffer::from_text("To: bob@x.org\nSubject: Hi\n\nTo: me, said the line\n");
    buffer.place(0, 9);
    typed(&mut buffer, &["<c-a>"]);
    assert_eq!(buffer.col(), 4);
    // C-a C-k clears the address and keeps the name.
    typed(&mut buffer, &["<c-k>"]);
    assert!(buffer.text().starts_with("To: \n"), "{}", buffer.text());
    // There already, C-a goes to the line's start; home as C-a.
    typed(&mut buffer, &["<c-a>"]);
    assert_eq!(buffer.col(), 0);
    buffer.place(1, 10);
    typed(&mut buffer, &["<home>"]);
    assert_eq!(buffer.col(), 9);
    // What is written is no header, whatever it says.
    buffer.place(3, 8);
    typed(&mut buffer, &["<c-a>"]);
    assert_eq!(buffer.col(), 0);
}

#[test]
fn reading_it_first_is_not_said_once_the_answer_is_open() {
    let mut app = demo();
    press(&mut app, &["r"]);
    assert!(app.compose.is_some());
    assert!(!app.status.contains("reading it first"), "{}", app.status);
}

#[test]
fn c_c_and_c_x_are_said_while_writing_and_a_key_they_lack_is_undefined() {
    let mut app = demo();
    press(&mut app, &["c", "hi", "<c-c>"]);
    assert_eq!(app.status, "C-c-");
    press(&mut app, &["z"]);
    assert_eq!(app.status, "C-c z is undefined");
    assert!(written(&app).starts_with("To: hi\n"), "{}", written(&app));
    press(&mut app, &["<c-x>"]);
    assert_eq!(app.status, "C-x-");
    press(&mut app, &["<c-z>"]);
    assert_eq!(app.status, "C-x C-z is undefined");
    // C-g lets C-c go, saying nothing more.
    press(&mut app, &["<c-c>", "<c-g>"]);
    assert_eq!(app.status, "");
    assert!(app.compose.is_some());
}

#[test]
fn a_line_too_long_for_the_box_is_marked_where_it_is_cut() {
    let long = "This is a long sentence that keeps going and going so that it must reach past the right edge of the box END";
    let mut app = demo();
    press(&mut app, &["c", "<tab>", "<tab>", "<tab>", long]);
    // The cursor at its end, it is scrolled, and says so at its start.
    let screen = app::screen(&mut app, 120, 35);
    assert!(
        screen
            .lines()
            .any(|l| l.contains("│$ ") && l.contains("END")),
        "{screen}"
    );
    press(&mut app, &["<enter>", "short"]);
    let screen = app::screen(&mut app, 120, 35);
    let line = screen
        .lines()
        .find(|l| l.contains("This is a long"))
        .expect("the long line");
    assert!(line.contains("$│"), "{line}");
    assert!(!line.contains("END"), "{line}");
}

#[test]
fn f1_while_writing_leads_with_the_keys_of_writing() {
    let mut app = demo();
    press(&mut app, &["c", "<F1>"]);
    let screen = app::screen(&mut app, 120, 35);
    let lines: Vec<&str> = screen.lines().collect();
    let top = lines
        .iter()
        .position(|l| l.contains("╭ Keys"))
        .expect("the keys");
    assert!(lines[top + 1].contains("C-c C-c"), "{screen}");
    // Elsewhere, the help is as it was.
    let mut app = demo();
    press(&mut app, &["<F1>"]);
    let screen = app::screen(&mut app, 120, 35);
    let lines: Vec<&str> = screen.lines().collect();
    let top = lines.iter().position(|l| l.contains("╭ Keys")).unwrap();
    assert!(lines[top + 1].contains("enter"), "{screen}");
}

#[test]
fn a_narrow_box_shows_whole_hints_with_undo_and_f1() {
    for (width, height) in [(60, 20), (70, 30), (80, 24)] {
        let mut app = demo();
        press(&mut app, &["c"]);
        let screen = app::screen(&mut app, width, height);
        assert!(screen.contains("F1: every key."), "{width}: {screen}");
        assert!(screen.contains("F1 all keys"), "{width}: {screen}");
        assert!(screen.contains("C-/ undo"), "{width}: {screen}");
        // No hint cut short of what it does.
        for line in screen.lines().filter(|l| l.contains("C-g")) {
            assert!(line.contains("C-g stops"), "{width}: {line}");
        }
    }
    // The buttons on the top edge at an ordinary width, by name alone.
    let mut app = demo();
    press(&mut app, &["c"]);
    let screen = app::screen(&mut app, 120, 35);
    let top = screen
        .lines()
        .find(|l| l.contains("From you@example.com"))
        .unwrap();
    assert!(top.contains("Grammar"), "{top}");
    assert!(top.contains("Send"), "{top}");
}
