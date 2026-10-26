//! Accounts added and removed, the mouse, and the vault.

use super::*;

// ---- adding and removing accounts ----

/// Where a test keeps a password: nowhere, the system's store left alone.
pub(super) fn keep_in_test(_: Keeper, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
    Ok(())
}

pub(super) fn empty_app(config: PathBuf) -> App {
    microsoft();
    let mut app = App::new(Vec::new(), config);
    app.keep = keep_in_test;
    app.keepers = || vec![Keeper::Keyring, Keeper::Vault];
    app.browse = |_| {};
    app
}

/// The form filled in for the fake server, `password` typed last.
pub(super) fn fill_form(app: &mut App, port: u16, password: &str) {
    let port = port.to_string();
    press(
        app,
        &[
            "A",
            "me@example.com",
            "<tab>",
            "<tab>",
            "<c-u>",
            "127.0.0.1",
            "<tab>",
            "<c-u>",
            &port,
            "<tab>",
            "<c-u>",
            "me",
            "<tab>",
            "<tab>",
            "<tab>",
            password,
            "<enter>",
        ],
    );
}

#[test]
fn an_account_is_added_from_the_tui_once_it_signs_in_and_removed() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("letterbox/accounts.toml");
    let mut app = empty_app(path.clone());
    assert!(app::screen(&mut app, 120, 30).contains("Press A to add one"));
    press(&mut app, &["A", "me@example.com"]);
    let form = app.dialog.form().unwrap();
    assert_eq!(form.fields[1], "example.com");
    assert_eq!(form.fields[2], "imap.example.com");
    assert_eq!(form.fields[4], "me@example.com");
    press(&mut app, &["<esc>"]);
    assert!(app.dialog.form().is_none());

    fill_form(&mut app, account.port.unwrap(), "secret");
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(app.status, "added example.com");
    assert_eq!(app.folder_name(), "Inbox");
    assert_eq!(app.shown.len(), 3);
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("host = \"127.0.0.1\""), "{file}");
    assert!(!file.contains("secret") && !file.contains("keep"), "{file}");
    assert!(file.contains("user = \"me\""), "{file}");
    assert!(file.contains("security = \"none\""), "{file}");
    // As read again, it is the account added.
    let accounts = crate::config::load(&path).unwrap();
    assert_eq!(accounts[0].port, account.port);

    press(&mut app, &["h", "D", "n"]);
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);
    press(&mut app, &["D", "y"]);
    assert!(crate::config::load(&path).unwrap().is_empty());
    assert!(app.listing.is_none());
    assert!(app::screen(&mut app, 120, 30).contains("No accounts yet."));
}

#[test]
fn an_account_that_cannot_sign_in_is_not_added() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    fill_form(&mut app, account.port.unwrap(), "wrong");
    let form = app.dialog.form().expect("the form stays open");
    assert_eq!(form.failed, Some(app::Failure::Refused));
    assert!(
        form.note.as_ref().unwrap().contains("cannot sign in"),
        "{:?}",
        form.note
    );
    assert!(!path.exists());
    let screen = app::screen(&mut app, 100, 30);
    assert!(
        screen.contains("Add account") && screen.contains("•••••▏"),
        "{screen}"
    );
}

/// The form filled in for the fake server, as for a Gmail address, up to
/// the password.
pub(super) fn fill_gmail_form(app: &mut App, port: u16) {
    let port = port.to_string();
    press(
        app,
        &[
            "A",
            "me@gmail.com",
            "<tab>",
            "<tab>",
            "<c-u>",
            "127.0.0.1",
            "<tab>",
            "<c-u>",
            &port,
            "<tab>",
            "<c-u>",
            "me",
            "<tab>",
            "<tab>",
            "<tab>",
        ],
    );
}

#[test]
fn a_failed_sign_in_still_says_where_an_app_password_is_made() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let mut app = empty_app(dir.path().join("accounts.toml"));
    fill_gmail_form(&mut app, account.port.unwrap());
    press(&mut app, &["wrong", "<enter>"]);
    let form = app.dialog.form().expect("the form stays open");
    assert_eq!(form.failed, Some(app::Failure::Refused));
    let screen = app::screen(&mut app, 100, 40);
    assert!(screen.contains("cannot sign in"), "{screen}");
    assert!(screen.contains("apppasswords"), "{screen}");
    let screen = app::screen(&mut app, 80, 24);
    assert!(screen.contains("Sign in and add"), "{screen}");
}

#[test]
fn a_mistake_in_the_form_is_said_without_the_providers_note() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = empty_app(dir.path().join("accounts.toml"));
    press(
        &mut app,
        &[
            "A",
            "me@gmail.com",
            "<tab>",
            "<tab>",
            "<tab>",
            "<c-u>",
            "x",
            "<tab>",
            "<tab>",
            "<tab>",
            "<tab>",
            "pw",
            "<enter>",
        ],
    );
    let screen = app::screen(&mut app, 100, 40);
    assert!(screen.contains("the port is a number"), "{screen}");
    assert!(!screen.contains("apppasswords"), "{screen}");
}

#[test]
fn ctrl_o_opens_the_page_an_app_password_is_made_on() {
    static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let dir = tempfile::tempdir().unwrap();
    let mut app = empty_app(dir.path().join("accounts.toml"));
    app.browse = |url| OPENED.lock().unwrap().push(url.to_string());
    press(&mut app, &["A", "me@example.edu", "<c-o>"]);
    assert!(
        OPENED.lock().unwrap().is_empty(),
        "no page for an unknown provider"
    );
    assert!(!app::screen(&mut app, 120, 40).contains("^o"));
    press(&mut app, &["<c-u>", "me@gmail.com"]);
    assert!(app::screen(&mut app, 120, 40).contains("^o"));
    press(&mut app, &["<tab>", "<c-o>"]);
    assert_eq!(
        *OPENED.lock().unwrap(),
        ["https://myaccount.google.com/apppasswords"]
    );
    assert!(app.status.contains("opened"), "{}", app.status);
    assert_eq!(
        app.dialog.form().unwrap().fields[app::NAME],
        "gmail.com",
        "nothing typed"
    );
}

#[test]
fn what_is_pasted_in_the_form_is_typed_in_its_field() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = empty_app(dir.path().join("accounts.toml"));
    press(&mut app, &["A"]);
    app.paste("me@gmail.com");
    let form = app.dialog.form().unwrap();
    assert_eq!(form.fields[app::EMAIL], "me@gmail.com");
    assert_eq!(
        form.fields[app::NAME],
        "gmail.com",
        "filled from the address"
    );
    while app.dialog.form().unwrap().at != app::PASSWORD {
        press(&mut app, &["<tab>"]);
    }
    // A password as copied, with the line it ended.
    app.paste("x7#Qv!9z L$2p\r\n");
    assert_eq!(
        app.dialog.form().unwrap().fields[app::PASSWORD],
        "x7#Qv!9z L$2p"
    );
    // Nothing is written where nothing is typed.
    while app.dialog.form().unwrap().at != app::SIGN_IN {
        press(&mut app, &["<tab>"]);
    }
    app.paste("abc");
    assert_eq!(app.dialog.form().unwrap().fields[app::SIGN_IN], "");
}

#[test]
fn the_demos_account_is_not_changed() {
    let mut app = demo();
    app.fix_accounts();
    press(&mut app, &["A"]);
    assert!(app.dialog.form().is_none());
    assert!(app.status.contains("without --demo"));
    assert!(!app.rows.contains(&app::Row::Add));
}

#[test]
fn the_accounts_file_keeps_its_comments_as_accounts_come_and_go() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    std::fs::write(
        &path,
        "# my mail\n[[account]]\nname = \"Home\" # the old one\nemail = \"a@b.c\"\nhost = \"imap.b.c\"\n",
    )
    .unwrap();
    let mut work = crate::setup::account(
        "Work",
        "me@example.edu",
        "imap.example.edu",
        993,
        "me@example.edu",
    );
    work.password_command = Some("pass mail".to_string());
    crate::config::add(&path, &work).unwrap();
    assert!(crate::config::add(&path, &work).is_err(), "a name twice");
    let names: Vec<String> = crate::config::load(&path)
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, ["Home", "Work"]);
    let removed = crate::setup::remove(&path, "home", true).unwrap();
    assert_eq!(removed.0.host, "imap.b.c");
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.starts_with("# my mail\n"), "{file}");
    assert!(
        !file.contains("Home") && file.contains("name = \"Work\""),
        "{file}"
    );
    assert!(!file.contains("port") && !file.contains("user"), "{file}");
    assert!(crate::setup::remove(&path, "Home", true).is_err());
}

#[test]
fn providers_servers_are_guessed_from_the_address() {
    use crate::config::guess;
    assert_eq!(guess("x@gmail.com").0, "imap.gmail.com");
    assert!(guess("x@gmail.com").1.unwrap().contains("app password"));
    assert_eq!(
        guess("x@example.edu"),
        ("imap.example.edu".to_string(), None)
    );
    assert_eq!(guess("nothing").0, "");
    assert_eq!(crate::config::guess_smtp("x@gmail.com"), "smtp.gmail.com");
    assert_eq!(
        crate::config::guess_smtp("x@outlook.com"),
        "smtp.office365.com"
    );
    assert_eq!(
        crate::config::guess_smtp("x@example.edu"),
        "smtp.example.edu"
    );
}

#[test]
fn mail_is_sent_where_the_account_signs_in_not_where_its_address_says() {
    let at = |email: &str, host: &str, auth| {
        let mut a = crate::setup::account("A", email, host, 993, email);
        a.auth = auth;
        a.submission()
    };
    use crate::config::Auth::{Microsoft, Password};
    assert_eq!(
        at("me@uni.example", "outlook.office365.com", Microsoft),
        ("smtp.office365.com".to_string(), 587),
        "an organisation's Microsoft 365"
    );
    assert_eq!(
        at("me@firm.example", "imap.gmail.com", Password),
        ("smtp.gmail.com".to_string(), 587),
        "Google Workspace"
    );
    assert_eq!(
        at("me@example.edu", "mail.example.edu", Password),
        ("smtp.example.edu".to_string(), 587)
    );
    assert_eq!(
        crate::config::guess_outgoing("me@uni.example", "outlook.office365.com", Microsoft),
        "smtp.office365.com"
    );
}

#[test]
fn the_form_says_where_mail_is_sent_and_takes_another_server() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    press(&mut app, &["A", "me@example.com"]);
    let form = app.dialog.form().unwrap();
    assert_eq!(form.fields[app::OUTGOING], "smtp.example.com");
    assert!(app::screen(&mut app, 120, 30).contains("Outgoing"));
    press(&mut app, &["<esc>", "A", "me@uni.example", "<tab>"]);
    assert_eq!(
        app.dialog.form().unwrap().fields[app::OUTGOING],
        "smtp.office365.com",
        "Microsoft's, as it signs in there"
    );
    press(&mut app, &["<esc>"]);

    let port = account.port.unwrap().to_string();
    press(
        &mut app,
        &[
            "A",
            "me@example.com",
            "<tab>",
            "<tab>",
            "<c-u>",
            "127.0.0.1",
            "<tab>",
            "<c-u>",
            &port,
            "<tab>",
            "<c-u>",
            "me",
            "<tab>",
        ],
    );
    assert_eq!(app.dialog.form().unwrap().at, app::OUTGOING);
    press(
        &mut app,
        &[
            "<c-u>",
            "mail.example.com:465",
            "<tab>",
            "<tab>",
            "secret",
            "<enter>",
        ],
    );
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    let added = &crate::config::load(&path).unwrap()[0];
    assert_eq!(added.submission(), ("mail.example.com".to_string(), 465));

    press(
        &mut app,
        &["A", "me@example.com", "<tab>", "<tab>", "<tab>"],
    );
    press(
        &mut app,
        &["<tab>", "<tab>", "<c-u>", "mail.example.com:port"],
    );
    press(&mut app, &["<tab>", "<tab>", "secret", "<enter>"]);
    let form = app.dialog.form().unwrap();
    assert_eq!(form.at, app::OUTGOING);
    assert_eq!(form.failed, Some(app::Failure::Mistake));
}

#[test]
fn an_app_password_page_is_known_for_the_providers_that_take_one() {
    use crate::config::app_password_help;
    let page = |email, host| app_password_help(email, host).map(|(_, page)| page);
    assert_eq!(
        page("x@gmail.com", ""),
        Some("https://myaccount.google.com/apppasswords")
    );
    assert!(page("x@icloud.com", "").is_some());
    assert!(page("x@yahoo.com", "").is_some());
    // An address of its own domain, on Gmail's server, is Gmail's.
    assert_eq!(
        page("x@example.edu", "imap.gmail.com"),
        page("x@gmail.com", "")
    );
    assert_eq!(
        page("x@outlook.com", ""),
        None,
        "it signs in in the browser"
    );
    assert_eq!(page("x@example.edu", "imap.example.edu"), None);
}

#[test]
fn an_app_password_is_taken_with_its_spaces_or_without() {
    use crate::config::app_password;
    let gmail = "imap.gmail.com";
    // Yahoo's and Fastmail's are shown in groups of four too.
    for host in ["imap.mail.yahoo.com", "imap.fastmail.com"] {
        assert_eq!(
            app_password(host, "ab12 cd34 ef56 gh78"),
            "ab12cd34ef56gh78"
        );
    }
    assert_eq!(
        app_password(gmail, "abcdefgh ijklmnop"),
        "abcdefgh ijklmnop",
        "not in groups of four"
    );
    assert_eq!(
        app_password(gmail, "abcd efgh ijkl mnop"),
        "abcdefghijklmnop"
    );
    assert_eq!(
        app_password(gmail, " abcd efgh ijkl mnop "),
        "abcdefghijklmnop"
    );
    assert_eq!(app_password(gmail, "abcdefghijklmnop"), "abcdefghijklmnop");
    // Not an app password's shape: kept as typed.
    assert_eq!(app_password(gmail, "my pass word"), "my pass word");
    // Elsewhere a space may be in a password.
    assert_eq!(
        app_password("imap.example.edu", "abcd efgh ijkl mnop"),
        "abcd efgh ijkl mnop"
    );
}

#[test]
fn with_no_accounts_it_says_how_to_add_one() {
    let mut app = App::new(Vec::new(), PathBuf::from("/nowhere/accounts.toml"));
    let screen = app::screen(&mut app, 120, 40);
    assert!(screen.contains("No accounts yet."), "{screen}");
    assert!(screen.contains("letterbox --demo"), "{screen}");
}

// ---- the mouse ----

pub(super) fn click_at(app: &mut App, target: app::Target) {
    let _ = app::screen(app, 120, 40);
    let (rect, _) = *app
        .targets
        .iter()
        .rev()
        .find(|(_, t)| *t == target)
        .unwrap_or_else(|| panic!("no {target:?} on the screen"));
    app.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        rect.x,
        rect.y,
    ));
    app.settle(Duration::from_secs(5));
}

pub(super) fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn the_forms_fields_and_buttons_are_clicked() {
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut app = empty_app(path.clone());
    press(&mut app, &["A", "me@example.com"]);
    click_at(&mut app, app::Target::Field(2));
    assert_eq!(app.dialog.form().unwrap().at, 2);
    press(&mut app, &["<c-u>", "127.0.0.1", "<tab>", "<c-u>"]);
    press(&mut app, &[&account.port.unwrap().to_string()]);
    click_at(&mut app, app::Target::Field(4));
    press(&mut app, &["<c-u>", "me"]);
    click_at(&mut app, app::Target::Field(7));
    click_at(&mut app, app::Target::Field(7));
    assert_eq!(
        app.dialog.form().unwrap().keeper,
        1,
        "a second click picks the vault"
    );
    click_at(&mut app, app::Target::Field(7));
    assert_eq!(
        app.dialog.form().unwrap().keeper,
        0,
        "and a third the keyring again"
    );
    click_at(&mut app, app::Target::Field(6));
    press(&mut app, &["secret"]);
    click_at(&mut app, app::Target::Button(app::Button::Add));
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);

    press(&mut app, &["h", "D"]);
    click_at(&mut app, app::Target::Button(app::Button::Keep));
    assert!(!matches!(app.dialog, app::Dialog::Remove(_)));
    assert_eq!(crate::config::load(&path).unwrap().len(), 1);
    press(&mut app, &["D"]);
    click_at(&mut app, app::Target::Button(app::Button::Remove));
    assert!(crate::config::load(&path).unwrap().is_empty());
}

#[test]
fn the_keys_at_the_foot_are_clicked_and_the_help_scrolls() {
    let mut app = demo();
    click_at(
        &mut app,
        app::Target::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
    );
    assert!(matches!(app.dialog, app::Dialog::Help(_)));
    let screen = app::screen(&mut app, 60, 12);
    assert!(screen.contains("j/k or the wheel scroll"), "{screen}");
    let (rect, _) = *app
        .targets
        .iter()
        .find(|(_, t)| *t == app::Target::Popup)
        .unwrap();
    for _ in 0..3 {
        app.mouse(mouse(MouseEventKind::ScrollDown, rect.x + 1, rect.y + 1));
    }
    let _ = app::screen(&mut app, 60, 12);
    assert!(matches!(app.dialog, app::Dialog::Help(3)));
    // The wheel does not run it off its end.
    for _ in 0..1000 {
        app.mouse(mouse(MouseEventKind::ScrollDown, rect.x + 1, rect.y + 1));
    }
    let end = app::screen(&mut app, 60, 12);
    assert!(end.contains("quit"), "{end}");
    app.key(ratatui::crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    ));
    assert!(!matches!(app.dialog, app::Dialog::Help(_)));
    click_at(
        &mut app,
        app::Target::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(app.message().is_some());
}

// ---- the vault ----

/// Held by each test that points `$XDG_CACHE_HOME` somewhere, so that
/// none writes where another looks.
pub(super) static CACHE_AT: Mutex<()> = Mutex::new(());

/// Held by each test that points `$LETTERBOX_VAULT` somewhere, so that
/// none reads another's, or the vault of whoever runs them.
pub(super) static VAULT_AT: Mutex<()> = Mutex::new(());

#[test]
fn the_vault_keeps_passwords_encrypted_with_a_passphrase() {
    let _held = VAULT_AT.lock().unwrap_or_else(|e| e.into_inner());
    let (account, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault_at = dir.path().join("data/vault.age");
    // SAFETY: tests that read it hold VAULT_AT.
    unsafe { std::env::set_var("LETTERBOX_VAULT", &vault_at) };
    crate::vault::lock();
    let path = dir.path().join("accounts.toml");
    microsoft();
    let mut app = App::new(Vec::new(), path.clone());
    app.keepers = || vec![Keeper::Keyring, Keeper::Vault];
    let port = account.port.unwrap().to_string();
    press(
        &mut app,
        &[
            "A",
            "me@example.com",
            "<tab>",
            "<tab>",
            "<c-u>",
            "127.0.0.1",
            "<tab>",
            "<c-u>",
        ],
    );
    press(
        &mut app,
        &[
            &port, "<tab>", "<c-u>", "me", "<tab>", "<tab>", "<tab>", "secret", "<tab>", "<right>",
        ],
    );
    let form = app.dialog.form().unwrap();
    assert_eq!(form.kept(), Some(Keeper::Vault));
    assert_eq!(
        form.shown(),
        [0, 1, 2, 3, 4, app::OUTGOING, 5, 6, 7, 8, 9],
        "a new vault's passphrase, twice"
    );
    press(
        &mut app,
        &[
            "<tab>",
            "correct horse",
            "<tab>",
            "correct house",
            "<enter>",
        ],
    );
    let form = app.dialog.form().expect("still open");
    assert_eq!(form.note.as_deref(), Some("the two passphrases differ"));
    assert!(!vault_at.exists());
    press(&mut app, &["<c-u>", "correct horse", "<enter>"]);
    assert!(
        app.dialog.form().is_none(),
        "{:?}",
        app.dialog.form().and_then(|f| f.note.clone())
    );
    assert_eq!(app.shown.len(), 3);

    let file = std::fs::read_to_string(&path).unwrap();
    assert!(
        file.contains("keep = \"vault\"") && !file.contains("secret"),
        "{file}"
    );
    let sealed = std::fs::read(&vault_at).unwrap();
    assert!(sealed.starts_with(b"age-encryption.org/v1\n"));
    assert!(!String::from_utf8_lossy(&sealed).contains("secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&vault_at).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Opened again, as letterbox does as it starts.
    crate::vault::lock();
    let e = crate::vault::unlock("correct house").unwrap_err();
    assert_eq!(e.to_string(), "that is not the vault's passphrase");
    crate::vault::unlock("correct horse").unwrap();
    let saved = crate::config::load(&path).unwrap().remove(0);
    assert_eq!(saved.password().unwrap(), "secret");

    press(&mut app, &["h", "D", "y"]);
    assert!(crate::config::load(&path).unwrap().is_empty());
    assert!(crate::vault::get("127.0.0.1", "me").is_err(), "forgotten");
    crate::vault::lock();

    // One made with no passphrase: its key a file only you may read, and
    // opened by it, nothing asked.
    let e = crate::vault::make_key().unwrap_err();
    assert!(e.to_string().contains("a vault already"), "{e}");
    let keyed = dir.path().join("keyed/vault.age");
    unsafe { std::env::set_var("LETTERBOX_VAULT", &keyed) };
    assert!(crate::vault::unlock_by_key().is_none(), "none yet");
    crate::vault::make_key().unwrap();
    crate::vault::put("imap.example.com", "me", "pw").unwrap();
    crate::vault::lock();
    crate::vault::unlock_by_key().unwrap().unwrap();
    assert_eq!(crate::vault::get("imap.example.com", "me").unwrap(), "pw");
    // Given a passphrase instead: the key file gone, the passphrase asked.
    assert!(crate::vault::rekey(Some("short")).is_err());
    crate::vault::rekey(Some("a new horse")).unwrap();
    assert!(!crate::vault::key_path().exists());
    crate::vault::lock();
    assert!(crate::vault::unlock_by_key().is_none());
    crate::vault::unlock("a new horse").unwrap();
    assert_eq!(crate::vault::get("imap.example.com", "me").unwrap(), "pw");
    // And a key file again.
    crate::vault::rekey(None).unwrap();
    crate::vault::lock();
    assert!(crate::vault::unlock("a new horse").is_err());
    crate::vault::unlock_by_key().unwrap().unwrap();
    assert_eq!(crate::vault::get("imap.example.com", "me").unwrap(), "pw");
    // A change cut short, the new key beside the vault it opens: it opens.
    let key = std::fs::read_to_string(crate::vault::key_path()).unwrap();
    std::fs::rename(
        crate::vault::key_path(),
        crate::vault::key_path().with_extension("key.new"),
    )
    .unwrap();
    std::fs::write(crate::vault::key_path(), "AGE-SECRET-KEY-1STALE").unwrap();
    crate::vault::lock();
    crate::vault::unlock_by_key().unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(crate::vault::key_path()).unwrap(),
        key
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(crate::vault::key_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    crate::vault::lock();
    unsafe { std::env::remove_var("LETTERBOX_VAULT") };
}

/// The system's own store, by keyring-rs, written to, read and emptied:
/// run by hand, as it touches the keychain of whoever runs it.
#[test]
#[ignore]
fn the_systems_store_keeps_a_password() {
    let (host, user) = ("letterbox.test.invalid", "probe@letterbox.test");
    crate::secret::store(Keeper::Keyring, host, user, "p'w \"d").unwrap();
    assert_eq!(
        crate::secret::find(Keeper::Keyring, host, user).unwrap(),
        "p'w \"d"
    );
    let mut a = crate::setup::account("T", user, host, 993, user);
    a.keep = Keeper::Keyring;
    assert!(crate::secret::forget(&a));
    assert!(crate::secret::find(Keeper::Keyring, host, user).is_err());
}
