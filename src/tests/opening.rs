//! The dialog of a message's files and links, to open: each named so it
//! can be told apart, its keys said as the width allows.

use super::*;

#[test]
fn a_link_is_named_by_its_text_else_by_where_a_redirect_goes() {
    let html = "<a href=\"https://scholar.google.com/scholar_url?url=https://arxiv.org/abs/1&amp;hl=en\">\
                A <b>paper</b>\n title</a> \
                <a href='https://example.org/x'><img src=x></a> \
                <a href=\"https://www.google.com/url?q=https%3A%2F%2Fexample.org%2Fy&amp;sa=D\">https://www.google.com/url?q=https%3A%2F%2Fexample.org%2Fy&amp;sa=D</a>";
    let links = mail::links("", Some(html));
    assert_eq!(links.len(), 3, "{links:?}");
    assert_eq!(
        mail::link_names(&links, Some(html)),
        [
            Some("A paper title".to_string()),
            None,
            Some("https://example.org/y".to_string()),
        ]
    );
    // In text alone, a link has no name but where a redirect goes.
    assert_eq!(
        mail::link_names(&links, None),
        [
            Some("https://arxiv.org/abs/1".to_string()),
            None,
            Some("https://example.org/y".to_string()),
        ]
    );
}

#[test]
fn o_shows_each_link_by_its_name_and_where_it_goes() {
    let mut app = reading_the_alert();
    press(&mut app, &["o"]);
    let names: Vec<Option<String>> = app
        .dialog
        .files()
        .expect("listed")
        .items
        .iter()
        .filter_map(|i| match i {
            app::Item::Link { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        [
            Some(
                "Population-scale immune multiome atlas reveals regulatory disease mechanisms"
                    .to_string()
            ),
            Some("Learning gene programs from single-cell data with transformers".to_string()),
            Some("Cancel alert".to_string()),
        ]
    );
    // Its name as far as the width goes, and where it goes beside it.
    let screen = app::screen(&mut app, 80, 24);
    assert!(
        screen.contains("↗ Population-scale immune multiome atlas")
            && screen.contains("nature.com · browser")
            && screen.contains("arxiv.org · browser")
            && screen.contains("↗ Cancel alert"),
        "{screen}"
    );
}

#[test]
fn y_on_what_is_not_a_link_says_only_a_link_is_copied() {
    let mut app = reading_the_alert();
    // Begun on its first link, the message itself above it.
    press(&mut app, &["o", "k", "y"]);
    assert!(app.launch.is_none());
    let files = app.dialog.files().expect("still open");
    assert_eq!(
        files.note.as_deref(),
        Some("only a link's address is copied")
    );
    // On a link, its address.
    press(&mut app, &["j", "j", "j", "y"]);
    let Some(app::Launch::Copy(url)) = app.launch.take() else {
        panic!("not copied: {:?}", app.launch);
    };
    assert!(url.contains("scholar_alerts"), "{url}");
}

#[test]
fn keys_that_do_not_fit_are_dropped_whole_esc_kept() {
    let mut app = reading_the_alert();
    press(&mut app, &["o"]);
    let screen = app::screen(&mut app, 50, 20);
    let foot = screen
        .lines()
        .find(|l| l.starts_with('╰'))
        .expect("its bottom edge");
    assert!(
        foot.contains("⏎ open") && foot.contains("· esc close ") && !foot.contains("y copy"),
        "{foot}"
    );
    press(&mut app, &["<esc>", "P"]);
    let screen = app::screen(&mut app, 50, 20);
    let foot = screen.lines().find(|l| l.starts_with('╰')).unwrap();
    assert!(foot.contains("· esc close "), "{foot}");
}
