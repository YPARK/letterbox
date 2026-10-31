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
fn only_scholar_and_google_themselves_are_followed_through() {
    let to = "https://bank.example/login";
    for href in [
        "https://scholar.google.com/scholar_url?url=https://bank.example/login&hl=en",
        "https://www.google.co.uk/url?q=https%3A%2F%2Fbank.example%2Flogin&sa=D",
        "https://google.com/url?url=https://bank.example/login",
        "https://scholar.google.com.tr/scholar_url?url=https://bank.example/login",
        "https://www.google.co.za/url?q=https://bank.example/login",
        "https://www.google.ru/url?q=https://bank.example/login",
        "https://google.ru/url?q=https://bank.example/login",
    ] {
        assert_eq!(crate::papers::unwrapped(href), to, "{href}");
    }
    for href in [
        "https://evil.example/www.google.com/url?q=https://bank.example/login",
        "https://evil.example/?scholar.google.com/scholar_url?url=https://bank.example/login",
        "https://scholar.google.com.evil.example/scholar_url?url=https://bank.example/login",
        "https://www.google.com@evil.example/url?q=https://bank.example/login",
        "https://www.google.com/search?q=https://bank.example/login",
        "https://www.google.com\\@evil.example/url?q=https://bank.example/login",
        "https://www.google.co.evil/url?q=https://bank.example/login",
    ] {
        assert_eq!(crate::papers::unwrapped(href), href, "{href}");
    }
}

#[test]
fn a_link_named_as_another_address_shows_where_it_truly_goes() {
    let (said, host, what) = crate::app::link_label(
        "https://login.evil.example/x",
        Some("https://bank.example/login"),
    );
    assert_eq!(said, "https://bank.example/login");
    assert_eq!(
        (host.as_deref(), what),
        (Some("login.evil.example"), "browser")
    );
    // Who it says it is, before an @, is not where it goes.
    let (_, host, _) =
        crate::app::link_label("https://bank.example@evil.example/x", Some("Your bank"));
    assert_eq!(host.as_deref(), Some("evil.example"));
}

#[test]
fn a_long_host_beside_a_link_keeps_its_end_and_what_opens_it() {
    let mut app = demo();
    app.dialog = app::Dialog::Files(app::Files {
        items: vec![app::Item::Link {
            url: "https://www.paypal.com.cgi-bin.webscr.login-verify.example.ru/".to_string(),
            name: Some("Your PayPal account".to_string()),
        }],
        at: 0,
        note: None,
        saving: false,
    });
    let screen = app::screen(&mut app, 80, 24);
    let line = screen.lines().find(|l| l.contains('↗')).expect("listed");
    assert!(
        line.contains("login-verify.example.ru · browser") && line.contains("Your PayPal"),
        "{line}"
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

#[test]
fn googles_redirect_is_gone_through_however_its_host_is_written() {
    assert_eq!(
        crate::papers::unwrapped("https://WWW.Google.COM/url?q=https://example.org/x"),
        "https://example.org/x"
    );
    assert_eq!(
        crate::papers::unwrapped("https://example.org/plain"),
        "https://example.org/plain"
    );
}

#[test]
fn googles_redirect_is_gone_through_by_its_host_as_parsed() {
    assert_eq!(
        crate::papers::unwrapped("https://%67oogle.com/url?q=https://example.org/x"),
        "https://example.org/x"
    );
}

#[test]
fn a_paper_host_is_known_by_its_host_as_parsed() {
    for link in [
        "https://arxiv.org/abs/2410.01234",
        "https://www.nature.com/articles/s41586-024-00001-x",
        "https://ncbi.nlm.nih.gov/pmc/articles/PMC123456/",
        "https://www.ncbi.nlm.nih.gov/pmc/articles/PMC123456/",
    ] {
        assert!(crate::papers::paper_host(link), "{link}");
    }
    for link in [
        "https://arxiv.org.evil.example/abs/1",
        "https://user@arxiv.org-x.com/abs/1",
        "https://ncbi.nlm.nih.gov/pmcx/articles/1",
        "https://ncbi.nlm.nih.gov/gene/1",
        "https://notarxiv.org/abs/1",
    ] {
        assert!(!crate::papers::paper_host(link), "{link}");
    }
}

#[test]
fn a_link_written_with_any_entity_is_found_by_its_name() {
    let html = "<a href=\"https://x.org/p?a&#61;1&amp;b=2\">Read the paper</a>";
    let links = mail::links("", Some(html));
    assert_eq!(links, ["https://x.org/p?a=1&b=2"]);
    assert_eq!(
        mail::link_names(&links, Some(html)),
        [Some("Read the paper".to_string())]
    );
}
