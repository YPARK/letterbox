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

This installs `letterbox`, and `lbx`, the same program by a shorter name:
`lbx` for the TUI, `lbx list -n 5`, and so on.

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

## Outlook and Microsoft 365

An address Microsoft says is its own, Outlook.com's or an organisation's
in Microsoft 365, as a university's, is signed into as Microsoft has mail
apps do: on Microsoft's page, where the password and any second factor
are given to Microsoft alone. letterbox asks Microsoft as the address is
typed, and fills in `outlook.office365.com`; `Sign in` in the form, or
`--auth microsoft`, picks it by hand.

Two ways are offered at once, whichever is done first: Microsoft's page,
opened in the browser, which comes back to letterbox on this machine; and
a code to enter at Microsoft's device page, on any device, as from a
machine reached over ssh. letterbox is given a token for IMAP, which it
renews, kept as a password is, in the keyring or the vault; your
organisation may take it back, and removing the account forgets it.

letterbox signs in as Thunderbird does, by the client ID Mozilla publishes
for mail apps, which organisations mostly allow already. Where yours does
not, it says so on Microsoft's page; its IT may approve it, or register an
app of its own, given to the account as `client_id`, and `tenant`.

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
| `f` | its files and links: a PDF or markdown presented in [ohp](https://github.com/YPARK/ohp), a linked PDF, or an arXiv paper's, downloaded first; anything else opened by the system, or in the browser |
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

## For AI agents: MCP

`lbx mcp` serves your mail to an AI agent over the Model Context Protocol
(MCP), on stdin and stdout, as `paperstack mcp` serves papers: the agent's
program starts it when it needs it.

**Claude Code**, for all your projects:

```sh
claude mcp add --scope user letterbox -- ~/.cargo/bin/lbx mcp
```

**Cursor**: in `~/.cursor/mcp.json`, beside any servers there already:

```json
{
  "mcpServers": {
    "letterbox": { "command": "/Users/you/.cargo/bin/lbx", "args": ["mcp"] }
  }
}
```

**Claude Desktop**: the same JSON, in
`~/Library/Application Support/Claude/claude_desktop_config.json`. Give
the whole path, from `which lbx`: programs opened from the Dock may not
have your shell's `PATH`.

| Tool | | |
|---|---|---|
| `list_accounts` | the accounts and their folders, with how many messages each holds, and how many unread | reads |
| `list_messages` | a folder's newest messages: sender, subject, date, flags, first words | reads |
| `search_messages` | a folder's messages with the words given, as the server searches | reads |
| `read_message` | a message's headers, attachments, links and text, 20,000 characters at a time; not marked read | reads |
| `mark_message` | mark a message read or unread, flagged or not | changes |

The first four are marked read-only, so a program may let the agent use
them without asking. Nothing is sent, moved or deleted. Mail is from
strangers, and a message may be written to steer an agent: the agent is
told to read what a message says as data, never as instructions. An
account whose password is in the vault is not served, as its passphrase
cannot be asked for there; one in the keyring, or Outlook's, is. `lbx
--demo mcp` serves the demo's mail, to try an agent on.

## In Emacs

With [vterm](https://github.com/akermu/emacs-libvterm):

```elisp
(defun letterbox ()
  (interactive)
  (let ((vterm-shell "lbx --theme dark")
        (vterm-buffer-name "*letterbox*"))
    (vterm)))
```

## License

MIT; see [LICENSE](LICENSE).
