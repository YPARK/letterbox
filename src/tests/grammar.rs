//! The message being written, its grammar checked: what is checked of it,
//! and what a model's sentence, made over, says to change.

use super::*;
use crate::grammar::{self, Edit};

/// `line` with `edits`, its own, made: the last first, so each one's place
/// holds.
fn made(line: &str, edits: &[Edit]) -> String {
    let mut chars: Vec<char> = line.chars().collect();
    for e in edits.iter().rev() {
        chars.splice(e.start..e.end, e.with.chars());
    }
    chars.into_iter().collect()
}

#[test]
fn what_a_model_makes_of_a_sentence_is_said_as_the_least_to_change() {
    for (was, is) in [
        ("Thank you for you time.", "Thank you for your time."),
        ("I has went to the store.", "I went to the store."),
        (
            "Please find attach the revised manuscript, which address all comments.",
            "Please find the attached revised manuscript, which addresses all the comments.",
        ),
        (
            "We should meet on monday to discuss about the results.",
            "We should meet on Monday to discuss the results.",
        ),
        ("Their going.", "They are going."),
        ("Nothing to do here.", "Nothing to do here."),
    ] {
        let edits = grammar::edits(0, 0, was, is);
        assert_eq!(made(was, &edits), is, "{was}: {edits:?}");
    }
    // Each the least: a word for a word, where that is all.
    let edits = grammar::edits(3, 10, "Thank you for you time.", "Thank you for your time.");
    assert_eq!(edits.len(), 1, "{edits:?}");
    let e = &edits[0];
    assert_eq!((e.row, e.start, e.end), (3, 24, 27));
    assert_eq!((e.said.as_str(), e.with.as_str()), ("you", "your"));
    assert!(grammar::edits(0, 0, "Nothing to do here.", "Nothing to do here.").is_empty());
}

#[test]
fn only_what_is_written_is_checked_not_the_head_quotes_or_signature() {
    let text = "To: ada@analytical.example\nSubject: Re: the egnine\n\nThank you for you time. I has went home.\n\nOn Monday, Ada wrote:\n> Their going.\n\n-- \nAda's freind\n";
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let pieces = grammar::pieces(&lines);
    let said: Vec<&str> = pieces.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(said, ["Thank you for you time.", "I has went home."]);
    assert_eq!((pieces[0].row, pieces[0].start), (3, 0));
    assert_eq!((pieces[1].row, pieces[1].start), (3, 24));
    // Made over by a model, each sentence's edits in the line's places.
    let fix = |s: &str| -> anyhow::Result<String> {
        Ok(s.replace("you time", "your time")
            .replace("has went", "went"))
    };
    let edits = grammar::check(&lines, &mut grammar::Using(fix)).unwrap();
    assert_eq!(
        made(&lines[3], &edits),
        "Thank you for your time. I went home."
    );
    assert!(edits.iter().all(|e| e.row == 3));
}

/// The demo, a message begun in it, `body` written.
fn writing(body: &str) -> App {
    let mut app = demo();
    press(
        &mut app,
        &[
            "c",
            "ada@analytical.example",
            "<c-n>",
            "<c-n>",
            "Hello",
            "<c-n>",
            "<c-n>",
            body,
        ],
    );
    app
}

fn written(app: &App) -> String {
    app.compose.as_ref().unwrap().buffer.text()
}

#[test]
fn m_dollar_checks_what_is_written_and_each_change_is_taken_or_not() {
    let mut app = writing("Thank you for you time. I has went home on monday.");
    app.grammar_open = |_| {
        Ok(Box::new(grammar::Using(|s: &str| {
            Ok(s.replace("you time", "your time")
                .replace("has went", "went")
                .replace("monday", "Monday"))
        })))
    };
    press(&mut app, &["<m-$>"]);
    app.settle(Duration::from_secs(5));
    // Each change said in turn, as Emacs's query-replace says it.
    assert!(app.status.contains("“you” → “your”"), "{}", app.status);
    assert!(app.status.contains("1 of 3"), "{}", app.status);
    // y takes it; n leaves it; ! takes the rest.
    press(&mut app, &["y"]);
    assert!(app.status.contains("take out “has”"), "{}", app.status);
    press(&mut app, &["n", "!"]);
    assert!(
        written(&app)
            .trim_end()
            .ends_with("Thank you for your time. I has went home on Monday."),
        "{}",
        written(&app)
    );
    assert!(app.status.contains("2 changed"), "{}", app.status);
    // Checked again, nothing to change but what was left; q stops.
    app.grammar_open = |_| Ok(Box::new(grammar::Using(|s: &str| Ok(s.to_string()))));
    press(&mut app, &["<m-$>"]);
    app.settle(Duration::from_secs(5));
    // The model read once stays: what it says is as before.
    assert!(app.status.contains("take out “has”"), "{}", app.status);
    press(&mut app, &["q"]);
    assert!(app.status.contains("0 changed"), "{}", app.status);
    // Any other key: the checking let go, and the key written, where the
    // change was.
    press(&mut app, &["<m-$>"]);
    app.settle(Duration::from_secs(5));
    press(&mut app, &["x"]);
    assert!(app.checking.is_none());
    assert!(written(&app).contains("I xhas went"), "{}", written(&app));
    // ! takes all that is left.
    press(&mut app, &["<backspace>", "<m-$>"]);
    app.settle(Duration::from_secs(5));
    press(&mut app, &["!"]);
    assert!(written(&app).contains("I went home"), "{}", written(&app));
}

#[cfg(not(feature = "grammar"))]
#[test]
fn without_the_grammar_feature_m_dollar_says_how_to_have_it() {
    let mut app = writing("Thank you for you time.");
    press(&mut app, &["<m-$>"]);
    app.settle(Duration::from_secs(5));
    assert!(app.status.contains("--features grammar"), "{}", app.status);
}

/// The default model, fetched from Hugging Face and read: run by hand, as
/// `cargo test --features grammar -- --ignored the_default_model`.
#[cfg(feature = "grammar")]
#[test]
#[ignore = "fetches the model from Hugging Face"]
fn the_default_model_corrects_a_sentence() {
    let mut model = grammar::open(grammar::DEFAULT).unwrap();
    let lines: Vec<String> = "Subject: x\n\nThank you for you time. We should meet on monday."
        .lines()
        .map(str::to_string)
        .collect();
    let edits = grammar::check(&lines, model.as_mut()).unwrap();
    let said: Vec<(&str, &str)> = edits
        .iter()
        .map(|e| (e.said.as_str(), e.with.as_str()))
        .collect();
    assert!(said.contains(&("you", "your")), "{said:?}");
    assert!(said.contains(&("monday.", "Monday.")), "{said:?}");
}
