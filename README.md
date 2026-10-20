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

In `~/.config/letterbox/accounts.toml` (`$LETTERBOX_CONFIG`, or
`--config`):

```toml
[[account]]
name = "Work"
email = "me@example.com"
host = "imap.example.com"
# port = 993, or 143 with security = "starttls"
# user = "me", if it is not the address
# password_command = "pass show mail/work"
```

The password is printed by `password_command`, or, without one, on macOS,
kept in the keychain:

```sh
security add-internet-password -s imap.example.com -a me@example.com -w
```

`letterbox accounts --check` signs in to each, and says what went wrong.
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
