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
in, so a mistyped server or password is said at once. Its password is
kept in the macOS keychain, or on Linux the Secret Service through
`secret-tool`, never in the file; removing the account forgets it. In the
TUI, `x` on an account in the sidebar removes it, after asking.

The accounts are in `~/.config/letterbox/accounts.toml` (`$LETTERBOX_CONFIG`,
or `--config`), to edit by hand as well; letterbox keeps its comments:

```toml
[[account]]
name = "Work"
email = "me@example.com"
host = "imap.example.com"
# port = 993, or 143 with security = "starttls"
# user = "me", if it is not the address
# password_command = "pass show mail/work"
```

Without a `password_command`, the password is the keychain's, for the host
and user. Outlook signed into in the browser, without a password, is to
come.

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
| `x` | in the sidebar, remove the account picked |
| `?` | all the keys |
| `q` | quit |

Moving through the list, the message picked is shown beside it, and marked
read, once you pause on it, as a mail app does. The mouse picks and
scrolls. Each account is asked from a thread of its own, so the screen
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
