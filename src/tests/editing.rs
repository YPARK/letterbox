//! The message being written, edited: the mark and the region, searching
//! as you type, and undo.

use super::*;

// ---- the mark, the region, and searching as you type, in compose ----

/// Press `keys`, as `app::keys` names them, in `buffer`; what the last did.
pub(super) fn edit_keys(buffer: &mut crate::edit::Buffer, keys: &[&str]) -> crate::edit::Done {
    let mut prefix = None;
    let mut done = crate::edit::Done::Stay;
    for key in keys.iter().flat_map(|k| app::keys(k)) {
        done = crate::edit::handle(buffer, &mut prefix, key);
    }
    done
}

#[test]
fn the_region_from_the_mark_is_cut_with_c_w_and_copied_with_m_w() {
    use crate::edit::{Buffer, Done};
    let mut buffer = Buffer::from_text("hello world");
    edit_keys(&mut buffer, &["<c- >"]);
    assert_eq!(buffer.region(), Some(((0, 0), (0, 0))));
    edit_keys(&mut buffer, &["<m-f>"]);
    assert_eq!(buffer.region(), Some(((0, 0), (0, 5))));
    edit_keys(&mut buffer, &["<c-w>"]);
    assert_eq!(buffer.text(), " world\n");
    assert_eq!(buffer.region(), None, "the mark goes with the region");
    edit_keys(&mut buffer, &["<c-e>", "<c-y>"]);
    assert_eq!(buffer.text(), " worldhello\n");

    // M-w copies: the text stays, and C-y puts the copy at point.
    let mut buffer = Buffer::from_text("abc");
    edit_keys(&mut buffer, &["<c-@>", "<c-e>"]);
    edit_keys(&mut buffer, &["<m-w>"]);
    assert_eq!(buffer.text(), "abc\n");
    assert_eq!(buffer.region(), None);
    edit_keys(&mut buffer, &["<c-y>"]);
    assert_eq!(buffer.text(), "abcabc\n");

    // Across lines.
    let mut buffer = Buffer::from_text("one\ntwo\nthree");
    edit_keys(&mut buffer, &["<c-f>", "<c- >", "<c-n>", "<c-n>", "<c-w>"]);
    assert_eq!(buffer.text(), "ohree\n");

    // With no mark, C-w cuts nothing.
    let mut buffer = Buffer::from_text("keep");
    edit_keys(&mut buffer, &["<c-e>", "<c-w>"]);
    assert_eq!(buffer.text(), "keep\n");

    // C-g lets the mark go, and then the message, as C-c C-k does.
    let mut buffer = Buffer::from_text("x");
    assert_eq!(edit_keys(&mut buffer, &["<c- >", "<c-g>"]), Done::Stay);
    assert_eq!(buffer.region(), None);
    assert_eq!(edit_keys(&mut buffer, &["<c-g>"]), Done::Cancel);
    assert_eq!(edit_keys(&mut buffer, &["<c-c>", "<c-k>"]), Done::Cancel);
}

#[test]
fn c_s_and_c_r_search_as_you_type() {
    use crate::edit::{Buffer, Done};
    let mut buffer = Buffer::from_text("one two\none Two");
    edit_keys(&mut buffer, &["<c-s>", "t", "w", "o"]);
    assert_eq!((buffer.row(), buffer.col()), (0, 7), "after the first");
    assert_eq!(buffer.searching(), Some("I-search: two".to_string()));
    edit_keys(&mut buffer, &["<c-s>"]);
    assert_eq!(
        (buffer.row(), buffer.col()),
        (1, 7),
        "case is ignored in lower case"
    );
    // ⏎ ends the search where it is; what is typed then is typed.
    edit_keys(&mut buffer, &["<enter>"]);
    assert_eq!(buffer.searching(), None);
    assert_eq!((buffer.row(), buffer.col()), (1, 7));

    // A capital is looked for as it is; failing, it says so.
    let mut buffer = Buffer::from_text("one two\none Two");
    edit_keys(&mut buffer, &["<c-s>", "T", "w", "o"]);
    assert_eq!((buffer.row(), buffer.col()), (1, 7));
    edit_keys(&mut buffer, &["<c-s>"]);
    assert_eq!(
        buffer.searching(),
        Some("Failing I-search: Two".to_string())
    );
    // ⌫ takes a letter back, searching again from where it began.
    edit_keys(
        &mut buffer,
        &["<backspace>", "<backspace>", "<backspace>", "o"],
    );
    assert_eq!((buffer.row(), buffer.col()), (0, 1));

    // C-g goes back to where the search began.
    let mut buffer = Buffer::from_text("one two");
    edit_keys(&mut buffer, &["<c-e>", "<c-r>", "o", "n"]);
    assert_eq!(
        (buffer.row(), buffer.col()),
        (0, 0),
        "C-r lands at its start"
    );
    assert_eq!(edit_keys(&mut buffer, &["<c-g>"]), Done::Stay);
    assert_eq!(buffer.searching(), None);
    assert_eq!((buffer.row(), buffer.col()), (0, 7));

    // Another key ends the search and does what it does.
    let mut buffer = Buffer::from_text("abc");
    edit_keys(&mut buffer, &["<c-s>", "b", "<c-a>"]);
    assert_eq!((buffer.searching(), buffer.col()), (None, 0));
}

#[test]
fn the_search_shows_in_the_compose_panes_mode_line() {
    let mut app = demo();
    press(&mut app, &["c", "<c-s>", "S", "u", "b"]);
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("I-search: Sub"), "{screen}");
}

#[test]
fn the_search_and_the_region_keep_to_emacs_in_the_corners() {
    use crate::edit::{Buffer, Done, handle};
    let shout = |c: char| {
        KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )
    };
    // C-G, as some terminals say C-g, ends the search, not the message.
    let mut buffer = Buffer::from_text("one two");
    edit_keys(&mut buffer, &["<c-s>", "t"]);
    assert_eq!(handle(&mut buffer, &mut None, shout('G')), Done::Stay);
    assert_eq!((buffer.searching(), buffer.col()), (None, 0));
    // Turned about, the search lands on the other end of what it found.
    let mut buffer = Buffer::from_text("one two");
    edit_keys(&mut buffer, &["<c-s>", "t", "w", "o"]);
    assert_eq!(buffer.col(), 7);
    edit_keys(&mut buffer, &["<c-r>"]);
    assert_eq!(
        (
            buffer.col(),
            buffer.searching().unwrap().starts_with("I-search")
        ),
        (4, true)
    );
    // Going back, what is found ends at or before point.
    let mut buffer = Buffer::from_text("abcdef");
    edit_keys(&mut buffer, &["<c-f>", "<c-r>", "a", "b"]);
    assert!(buffer.searching().unwrap().starts_with("Failing"));
    // A search taken back to nothing leaves C-n aiming where it began.
    let mut buffer = Buffer::from_text("ab\nabcdefgh\nabcdefgh");
    edit_keys(
        &mut buffer,
        &["<c-s>", "h", "<backspace>", "<enter>", "<c-n>"],
    );
    assert_eq!((buffer.row(), buffer.col()), (1, 0));
    // After C-w, C-n aims from where the cut left point.
    let mut buffer = Buffer::from_text("hello world\nabcdefghijkl");
    edit_keys(&mut buffer, &["<c- >", "<m-f>", "<c-w>", "<c-n>"]);
    assert_eq!((buffer.row(), buffer.col()), (1, 0));
    // C-w then C-k: one kill, put back together.
    let mut buffer = Buffer::from_text("abc def");
    edit_keys(
        &mut buffer,
        &["<c- >", "<c-f>", "<c-f>", "<c-w>", "<c-k>", "<c-y>"],
    );
    assert_eq!(buffer.text(), "abc def\n");
}

#[test]
fn tab_and_shift_tab_go_from_field_to_field_in_the_message_written() {
    let mut app = demo();
    press(&mut app, &["c", "ada@analytical.example"]);
    let at = |app: &App| {
        let b = &app.compose.as_ref().unwrap().buffer;
        (b.row(), b.col())
    };
    // To, Cc, Subject, then what is written, and round to To again; each
    // field at the end of what it says.
    press(&mut app, &["<tab>"]);
    assert_eq!(at(&app), (1, 4), "Cc");
    press(&mut app, &["<tab>"]);
    assert_eq!(at(&app), (2, 9), "Subject");
    press(&mut app, &["Plans", "<tab>"]);
    assert_eq!(at(&app).0, 4, "what is written");
    press(&mut app, &["Hello", "<tab>"]);
    assert_eq!(at(&app), (0, 26), "To again");
    // Shift-Tab, back: what is written, then Subject.
    press(&mut app, &["<s-tab>"]);
    assert_eq!(at(&app), (4, 5), "what is written");
    press(&mut app, &["<s-tab>"]);
    assert_eq!(at(&app), (2, 14), "Subject");
    let text = app.compose.as_ref().unwrap().buffer.text();
    assert!(
        text.starts_with("To: ada@analytical.example\nCc: \nSubject: Plans\n\nHello"),
        "{text}"
    );
}

#[test]
fn c_slash_c_underscore_and_c_x_u_undo_what_was_written_a_step_at_a_time() {
    use crate::edit::Buffer;
    let mut b = Buffer::from_text("");
    // Typed in a row: one step, as Emacs makes them.
    edit_keys(&mut b, &["hello world", "<enter>", "bye"]);
    assert_eq!(b.text(), "hello world\nbye\n");
    edit_keys(&mut b, &["<c-/>"]);
    assert_eq!(b.text(), "hello world\n\n");
    edit_keys(&mut b, &["<c-_>"]);
    assert_eq!(b.text(), "hello world\n");
    edit_keys(&mut b, &["<c-x>", "u"]);
    assert_eq!(b.text(), "\n");
    // Nothing left: nothing more undone; C-x and another key, nothing.
    edit_keys(&mut b, &["<c-/>", "<c-x>", "z"]);
    assert_eq!(b.text(), "\n");
    // Twenty-five typed: two steps, of twenty and five.
    edit_keys(&mut b, &["abcdefghijklmnopqrstuvwxy"]);
    edit_keys(&mut b, &["<c-/>"]);
    assert_eq!(b.text(), "abcdefghijklmnopqrst\n");
    // A kill and its yank, each a step; point back where it was.
    let mut b = Buffer::from_text("one\ntwo");
    edit_keys(&mut b, &["<c-k>", "<c-n>", "<c-y>"]);
    assert_eq!(b.text(), "\nonetwo\n");
    edit_keys(&mut b, &["<c-/>"]);
    assert_eq!(b.text(), "\ntwo\n");
    edit_keys(&mut b, &["<c-/>", "x"]);
    assert_eq!(b.text(), "xone\ntwo\n");
    // A backspace with nothing before it changes nothing, and is no step.
    let mut b = Buffer::from_text("ab");
    edit_keys(&mut b, &["<c-e>", "c", "<c-a>", "<backspace>", "<c-/>"]);
    assert_eq!(b.text(), "ab\n");
    // A grammar fix put in is undone as one.
    let mut b = Buffer::from_text("teh cat");
    b.replace(0, 0, 3, "the");
    edit_keys(&mut b, &["<c-/>"]);
    assert_eq!(b.text(), "teh cat\n");
}

#[test]
fn in_the_message_being_written_undo_is_its_text_not_the_mail_moved() {
    let mut app = demo();
    press(&mut app, &["c"]);
    let opened = app.compose.as_ref().unwrap().buffer.text();
    press(&mut app, &["ada", "<c-/>"]);
    assert_eq!(app.compose.as_ref().unwrap().buffer.text(), opened);
    assert!(!app.status.contains("put back"), "{}", app.status);
}

#[test]
fn undo_after_any_other_key_undoes_the_undoing_as_emacs_does() {
    use crate::edit::Buffer;
    let mut b = Buffer::from_text("");
    edit_keys(&mut b, &["one", "<enter>", "two"]);
    edit_keys(&mut b, &["<c-/>", "<c-/>"]);
    assert_eq!(b.text(), "one\n");
    // Another key, then undo: what was undone back, a step at a time.
    edit_keys(&mut b, &["<c-f>", "<c-/>"]);
    assert_eq!(b.text(), "one\n\n");
    edit_keys(&mut b, &["<c-/>"]);
    assert_eq!(b.text(), "one\ntwo\n");
    // C-x u goes on as C-/ does, its C-x no other key.
    edit_keys(&mut b, &["<c-x>", "u", "<c-x>", "u"]);
    assert_eq!(b.text(), "one\n");
}

#[test]
fn undo_while_searching_ends_the_search_and_never_loses_its_place() {
    use crate::edit::Buffer;
    let mut b = Buffer::from_text("");
    edit_keys(
        &mut b,
        &["<enter>", "<enter>", "<c-s>", "<c-/>", "<c-/>", "a"],
    );
    assert_eq!(b.text(), "a\n");
    let mut b = Buffer::from_text("");
    edit_keys(&mut b, &["<enter>", "<enter>", "<c-s>", "<c-x>", "u", "a"]);
    assert_eq!(b.text(), "\na\n");
}

#[test]
fn m_d_undone_puts_point_back_where_it_was() {
    use crate::edit::Buffer;
    let mut b = Buffer::from_text("one two");
    edit_keys(&mut b, &["<m-d>", "<c-/>", "x"]);
    assert_eq!(b.text(), "xone two\n");
}

#[test]
fn fixes_put_in_together_are_undone_together_and_steps_kept_are_few() {
    use crate::edit::Buffer;
    let mut b = Buffer::from_text("teh cat sat");
    edit_keys(&mut b, &["<c-e>", "!"]);
    b.begin_step();
    b.replace(0, 0, 3, "the");
    b.replace(0, 8, 11, "sits");
    assert_eq!(b.text(), "the cat sits!\n");
    edit_keys(&mut b, &["<c-/>"]);
    assert_eq!(b.text(), "teh cat sat!\n");
    // Far more steps than are kept: the oldest gone, the text never.
    let mut b = Buffer::from_text("");
    for _ in 0..300 {
        edit_keys(&mut b, &["x", "<enter>"]);
    }
    for _ in 0..400 {
        edit_keys(&mut b, &["<c-/>"]);
    }
    assert!(b.text().len() > 100, "{}", b.text().len());
}
