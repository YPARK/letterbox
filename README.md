# letterbox

Mail in the terminal: IMAP accounts read in a TUI laid out as a mail
app's, folders on the left, messages, and the message read beside them.
It runs in any terminal, Emacs's vterm among them, and narrows to one pane
at a time in a narrow window.

Early days. `letterbox --demo` shows made-up mail to try it with.

![letterbox's TUI, in its light theme](docs/tui.png)

`--theme dark`, or `LETTERBOX_THEME=dark`, keeps the terminal's own
background, as in an Emacs buffer.

## Install

Rust 1.92 or later:

```sh
cargo install --git https://github.com/YPARK/letterbox
```

Pure Rust, as paperstack and ohp: nothing in C is compiled. TLS is rustls
with RustCrypto's algorithms, which are not yet audited.

## Accounts

Press `A` in letterbox, or, from the shell:

```sh
letterbox accounts add                     # asks for the address and password
letterbox accounts add --email me@example.com --host imap.example.com
letterbox accounts                         # those set up; --check signs in to each
letterbox accounts remove Work             # its mail stays on its server
```

The server is filled in from the address, as the large providers have it
(Gmail, iCloud, Yahoo and Fastmail take an app password, which letterbox
says), else `imap.` and the domain. An account is added only once it signs
in, so a mistyped server or password is said at once. In the TUI, `D`
removes the account picked in the sidebar, or the one listed, after asking.

## Passwords

A password is kept encrypted, never in the accounts file, and forgotten
when its account is removed. letterbox keeps it itself, in Rust, with no
other program run, in one of two places, picked in the form (`Keep in`)
or with `--keep`:

| | |
|---|---|
| `keyring` | the system's own store, by [keyring-rs](https://github.com/open-source-cooperative/keyring-rs): the keychain on macOS, the Credential Manager on Windows, the Secret Service (GNOME's keyring, KWallet) on Linux. Unlocked as you log in; the default where there is one. |
| `vault` | one file, `~/.local/share/letterbox/vault.age`, encrypted with a passphrase of yours in the [age](https://age-encryption.org) format, by the age crate. The same on every system, as a server reached over ssh, with no keyring; its passphrase is asked for as letterbox starts, and the file opens with `age -d` or `rage -d` too. |

Or an account gives a `password_command` of yours, as `pass show mail/work`,
which runs before the TUI takes the terminal, so a passphrase it asks for
is asked for first.

The accounts are in `~/.config/letterbox/accounts.toml` (`$LETTERBOX_CONFIG`,
or `--config`), to edit by hand as well; letterbox keeps its comments:

```toml
[[account]]
name = "Work"
email = "me@example.com"
host = "imap.example.com"
# port = 993, or 143 with security = "starttls"
# user = "me", if it is not the address
# keep = "vault", if its password is in the vault, not the keyring
# password_command = "pass show mail/work", instead of either
```

Outlook signed into in the browser, without a password, is to come.

## Use

```sh
letterbox                                  # the TUI
letterbox --demo                           # made-up mail
letterbox folders                          # the first account's folders, and counts
letterbox list --folder INBOX -n 20 --json
letterbox read 4182 --json                 # a message, by its UID; not marked read
letterbox search grant report --account Work
```

| Key | |
|---|---|
| `j` / `k`, arrows | move; in a message, scroll |
| `h` / `l`, `tab` | between folders, messages and the message read |
| `enter` | open the folder, or read the message |
| `space` / `b` | a page down, or up, in the message |
| `n` / `p` | read the next message, or the one before |
| `/` | search the messages listed as you type; `enter` searches the server |
| `u` | mark read, or unread |
| `s` | flag, or unflag |
| `o` | open the message in the browser, as its HTML |
| `a` | save its attachments to `~/Downloads` (`$LETTERBOX_DOWNLOADS`) |
| `r` | look for new mail now; it is looked for every two minutes |
| `A` | add an account |
| `D` | remove the account picked in the sidebar, or the one listed |
| `?` | all the keys |
| `q` | quit |

Moving through the list, the message picked is shown beside it, and marked
read, once you pause on it, as a mail app does. The mouse picks and
scrolls, and presses the keys shown at the foot and the forms' fields and
buttons. Each account is asked from a thread of its own, so the screen
never waits on a server.

## In Emacs

With [vterm](https://github.com/akermu/emacs-libvterm):

```elisp
(defun letterbox ()
  (interactive)
  (let ((vterm-shell "letterbox --theme dark")
        (vterm-buffer-name "*letterbox*"))
    (vterm)))
```

## License

MIT; see [LICENSE](LICENSE).
