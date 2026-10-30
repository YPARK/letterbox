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
cargo install letterbox
```

or `cargo install --path .` in a clone of this repository.

This installs `letterbox`, and `lbx`, the same program by a shorter name:
`lbx` for the TUI, `lbx list -n 5`, and so on.

It is built with two features, both on by default:

- `pages`, to read a message as its pages, `V`: its HTML, and the images
  it carries, typeset by ohp and drawn in the terminal where it draws
  images (kitty, WezTerm, Ghostty, iTerm2). That brings typst, so it
  builds for longer, and typst's evaluator assembles one small file with
  the system's C toolchain.
- `grammar`, to check the grammar of what you write, `C-c C-s`, with a
  small model.

The model is a T5 from Hugging Face, made to correct English grammar, read
with candle, in pure Rust; it is fetched the first time it is asked for,
once, into `~/.cache/letterbox/models`. The default is
`visheratin/t5-efficient-tiny-grammar-correction`: 62 MB, MIT, a sentence
in a few hundredths of a second. `--grammar` (or `LETTERBOX_GRAMMAR`)
names another, as `visheratin/t5-efficient-mini-grammar-correction`, twice
the size.

`cargo install letterbox --no-default-features` leaves both out, and
`--features pages` or `--features grammar` then puts one back.

Pure Rust, as paperstack and ohp, but for typst's one small file of C:
built `--no-default-features`, none is compiled. TLS is rustls
with RustCrypto's algorithms, which are not yet audited.

## Accounts

Press `A` in letterbox, or, from the shell:

```sh
letterbox accounts add                     # asks for the address and password
letterbox accounts add --email me@example.com --host imap.example.com
letterbox accounts                         # those set up; --check signs in to each
letterbox accounts remove Work             # its mail stays on its server
letterbox accounts rename Work Office      # only letterbox's name for it
```

The server is filled in from the address, as the large providers have it
(Gmail, iCloud, Yahoo and Fastmail take an app password, which letterbox
says), else `imap.` and the domain; and the server mail is sent through,
`Outgoing` in the form, `--outgoing` from the shell, as [Sending](#sending)
says, to be changed if it is not that one. An account is added only once it signs
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

Made with no passphrase (nothing typed when asked for one), the vault's
key is a file beside it, `vault.key`, which only you may read: nothing is
ever asked, as letterbox starts or after, and it is as safe as that file,
as an ssh key with no passphrase is. `letterbox vault passphrase` gives
it a new passphrase, or, given none, a key file instead, either way: the
vault is written again under the new one, and nothing in it is lost if
that is cut short.

`letterbox keep vault` moves every password letterbox keeps, and the
map's key, into the vault, and `letterbox keep keyring` moves them back:
each is read where it is, kept in the other, its account set to say so,
and only then forgotten where it was. On macOS, the keychain asks again
whenever letterbox's program changes, as it does with each build and
each version, for each password it keeps; in the vault, there is one
passphrase, asked for as letterbox starts.

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
typed, and fills in `outlook.office365.com`, and `smtp.office365.com` to
send through, whatever the address's domain; `Sign in` in the form, or
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
| `j` / `k`, the arrows, `C-n` / `C-p` | move; in a message, scroll |
| `h` / `l`, `C-b` / `C-f`, `tab` / `S-tab` | between folders, messages and the message read |
| `enter` | open the folder, or read the message |
| `space` / `b` | a page down, or up, in the message; `C-v` / `M-v` as the page keys |
| the mouse | several selected, in the list or the Inbox looked through: dragged over their rows, Ctrl and a click adding one or taking it away, Shift and a click as far as one (where the terminal passes Shift and a click on). Selected rows are shown `▪`, and the foot says how many; `m`, `d`, `e`, `u` and `!` then act on all of it, asked first, and `esc` lets it go. No key selects alone, and nothing is selected on the map |
| `<` / `>` | make the message read wider, or narrower; or drag the line before it. It opens as wide again next time |
| `n` / `p` | read the next message, or the one before; in the map, searched, the next found, or the one before |
| `g` / `G`, `M-<` / `M->` | go to the top, or the end |
| `/` | search the messages listed as you type; `enter` searches every folder of every account at once, from what letterbox already knows (each message's sender, address, subject, first lines and labels, every word to be there, case let go): what is found is listed newest first, and `esc` goes back to what was listed. With a cluster of the map listed, or what fits nowhere, `enter` searches only it, and a search again of what was found there stays in it. Before the map is begun, `enter` asks the servers, of the folder listed. In the map, what holds every word is lit as you type, the rest faded |
| `esc` / `C-g` | back to the list; clear the search; close a dialog; out of a cluster gone into, a level. `C-g` as Emacs has it, wherever `esc` goes back |
| `u` | mark read, or unread |
| `!` | flag, or unflag |
| `d` | move to Trash; in Trash, put it back. If the server marks no Trash folder, letterbox asks which existing one. Several selected: asked first, how many from where; mail in an Inbox and mail you sent are left out unless `i` or `s` takes them, and `y` does it; what is trashed together is not learnt from. letterbox never removes mail from Trash for good |
| `c` | write a message |
| `r`, `C-r` | reply to the sender |
| `a` | reply to all: the sender, the others it went to copied |
| `f` | forward it, its files with it |
| `e` | archive it: out of the Inbox, to Archive; several selected, asked first |
| `o` | open the message in the browser, as its HTML, or one of its files and links: a PDF or markdown presented in [ohp](https://github.com/YPARK/ohp), a linked PDF, or an arXiv paper's, downloaded first; anything else opened by the system, or in the browser. There `s` and `S` save as they do from the message |
| `s` `S` | save the message read's attachments: `s` lists them, `⏎` saving the one picked, `a` all of them; `S` saves them all at once, into a folder you pick, browsed as files to attach are: `⏎` goes into a folder, and on `.` saves there; a name typed that no folder there has is made, as a new folder, to save in. It begins where you last saved, else in `~/Downloads` (`$LETTERBOX_DOWNLOADS`). The one picked is named first: its name, all but the extension picked, to type over; `←` `→` edit it, `⏎` keeps it |
| `O` | present the message read with [ohp](https://github.com/YPARK/ohp), typeset as a page, the images it carries in it; quit ohp to come back. Where the terminal speaks kitty's graphics protocol it is drawn as a page, elsewhere as its text |
| `V` | the message read as its pages, typeset by ohp and drawn in the terminal, where it draws images (kitty, WezTerm, Ghostty, iTerm2): `j` `k` or `space` `b` turn them, `V` again its text |
| `I` | the message read as its pages, as `V` shows them, with the pictures it shows from the web fetched. Never else: fetching them tells the sender it was read, and when |
| `B` | what was thrown away as junk in the last 7 days, and why: `⏎` puts the one picked back, `u` all of them, each learnt as kept. Told of as letterbox opens too |
| `J` | with the auto filter on, the new mail that looks like junk, each, how sure, and why, as what weighed most: `space` chooses one, `d` throws those chosen away, `n` says the one picked is not junk, learnt so, `i` looks through the Inbox, any other key leaves them. None chosen till you choose; never shown, nor thrown away, unasked |
| `P` | the papers the message read tells of, a Scholar alert's or a journal's table of contents, each said to be in your [paperstack](https://github.com/YPARK/paperstack), like what you starred there, or new: `⏎` puts the one picked there, starred, `o` opens it. paperstack is asked over MCP, `paperstack mcp` run as it is first wanted (`$LETTERBOX_PAPERSTACK` names another program) |
| `U` | look for new mail now; it is looked for every two minutes |
| `A` | add an account |
| `R` | rename the account picked in the sidebar, or the one listed; in the map, the label picked: asked first, how many carry it and where, and never onto a label there is. A right-click on a label's anchor on the map offers it too, with splitting it and removing it, as [the map](#the-map) says. `C-x u` undoes each |
| `D` | remove the account picked in the sidebar, or the one listed |
| `M` | the map, or back to the list from it, in any terminal: as `M-g` and `M-l` |
| `M-g`, `s-g` | the map: every account's mail laid out by what it is about and who it is from. `s-g` is ⌘G on a Mac, Super+G elsewhere; or click Map at the top |
| `M-l`, `s-l` | back to the list, of what the sidebar picks: a folder, a label, a cluster, or every cluster |
| `z` / `Z` / `0` | in the map, into the block picked, its parts a mosaic too, or out a level; `0` all the clusters. The wheel too, up over a block and down |
| `[` / `]` | in the map, narrow to the cluster before, or after |
| `i` | the Inbox, beside the map, to look through as far as you like: each message with what is suggested for it, none chosen until you choose; `m` drops down the labels nearest it to move it to, narrowed as you type, `space` keeps it, `d` discards it, `y` takes what is suggested, `←` `→` go round them; `⏎` reads it |
| `$` | in the Inbox looked through, do what was chosen, once asked: filed, kept, or discarded; those not chosen are left as they are |
| `,` | in the map, what it shows and how, each a key: `w` lays it out by words and people, words alone, or people alone; `t` all of it, this week's, or today's; `r` shows or hides read mail; `s` the mail you sent; `f` only the mail not yet filed under a label, each where it would go; `n` dims the areas with nothing new; `l` lays it out again |
| `m` | in the map, down at a message, move it onto a cluster or a label: `enter` on one, or a click on its anchor. It is given the label and taken out of the Inbox. In the list, the labels it may go to drop down, as in the Inbox looked through, and it is filed there at once, several asked first; another account's folder, as `Grant · ubc.ca`, moves it there |
| `N` | while moving, in the list, or in the Inbox looked through, a new label for it alone: its name is asked, its commonest word given; made as it is filed |
| `y` / `Y` / `-` | a message not yet filed, filed where it would go, or, `Y`, all like it, asked first, how many, in the list or down at a message on the map; once filed, the label's first suggestion filed too, or all of them; `-` says it is not one, and the map learns it |
| `v`, `S-down` / `S-up` | select nothing: several are selected with the mouse, as above |
| `M-r` | in the map, lay it out again from where it is, and show what moved: `enter` keeps it, `esc` drops it; `,` `l` too |
| `?`, `F1` | all the keys; `F1` while writing too |
| `C-x` `u` | undo what was done last: what was moved put back as it was, however it went (trashed, archived, filed, moved onto a cluster, or what `$` did in the Inbox looked through), its labels as they were, and what moving it taught taken back; or a label renamed, split or removed. Pressed before its server has answered, it says so: `C-x u` again then |
| `C-x` `C-c` | quit, as Emacs does; `q` and `C-c` alone say so, and do nothing more. While mail is still being moved, filed or sent, it asks first: `y` quits anyway, any other key waits |

No key holds Ctrl and Shift together, as kitty keeps those for itself.
⌘G and Super+G reach letterbox only in a terminal that speaks kitty's
keyboard protocol, as kitty, WezTerm and Ghostty do, and keeps no
shortcut of its own on them; Terminal and iTerm2 keep ⌘G to find. `M-g`
needs ⌥ sent as Meta (in Terminal, Settings › Profiles › Keyboard, “Use
Option as Meta key”). `M`, or a click on Map at the top right, shows the
map in any terminal, and `M` or List goes back.

`c` opens a message to write, `r` a reply, over the list or the map,
which stay faded beneath it; it is edited as Emacs edits a buffer: `C-a` and `C-e` the ends of a line, `C-f` `C-b` `C-n` `C-p`
by character and line, `C-k` kills to the end of the line and `C-y` puts it
back, `M-f` `M-b` `M-d` by word. `tab` goes from field to field, To, Cc,
Subject, then what is written, and round, `S-tab` back. `C-SPC` sets the mark: `C-w` kills the
region from it to the cursor, `M-w` copies it. `C-s` and `C-r` search
forward and back as you type, case let go unless a capital is typed; `C-s`
again finds the next, `⏎` stays there, `C-g` goes back. `C-c C-c` sends
it, held 10 seconds first, counted down at the foot: `C-g` takes it back,
still there to change, and `C-c C-c` again sends it at once. `C-c C-k` discards it, asking first if you changed it, as Emacs's
message-mode asks: `y` discards it, any other key goes on writing. `C-g`
lets go of a search, the mark, or a `C-c` begun, as Emacs's does, and
never of the message. Outside it, `C-x` `C-c` quits; `C-c` alone, pressed once more than it was needed, does nothing.

`C-c C-s`, or the Grammar button on its top edge, checks the grammar of
what you wrote; `M-$`, as Emacs checks
spelling, does too, where the terminal sends Option as Meta: not the head, what it quotes, the line
saying who wrote it, nor the signature. Each change the model would make
is asked in turn in a pop-up, the sentence it is in shown, what it would
change in red, there and in the text: Yes (`y` or `space`) takes it, Skip
(`n`) leaves it, Accept all (`a`) takes all that are left, Ignore all
(`i`) none, and any other key stops and is written. Nothing changes but
what you take.

Built with grammar, `C-c C-c` checks it first, before it is sent: with
nothing to change it goes at once; else each change is asked, and then
whether to send it now, Send (`y`) or Keep writing. `C-c C-c` again while
it is asked sends it as it is. A model that cannot be read, as with no
network the first time, keeps nothing from going: it is sent, and said
to be unchecked.

`!` and `q` do as `a` and `i`, as Emacs's query-replace has them.

If Emacs is new to you, this is all you need:

| To | Press |
|---|---|
| move, and go to a line's ends | the arrows, `Home`, `End` |
| send it | `C-c C-c`: Ctrl and c, twice |
| attach a file | `C-c C-a`, then type part of its name and `⏎` |
| stop writing, unsent | `C-c C-k`, then `y` if it asks |
| get out of anything half done | `C-g`, which never loses what you wrote |

The same guide is under the message as you write, `F1` lists every key,
and the foot says what each key did and what to press next. The file
picker says how to narrow and mark under what is typed.

`C-c C-a` attaches files, picked a folder at a time as
[senna](https://github.com/causalpathlab/senna-rs) picks them: it begins where you
last attached from, else in Downloads, on the newest file, starred.
Typing narrows to the names that hold what is typed, and from a `/` it is
a path, followed as you type; `⏎` or `→` goes into a folder, `←` or
backspace up, `~` home. `␣` marks a file, `C-a` all those shown, and `⏎`
attaches the marked, or the one picked; `esc` leaves. Each file is an
`Attach` line under the headers, with its size beside it, and is read
only as the message is sent; kill the line and it is not. Files dropped
onto the terminal are attached too. A forward's files are its `Attach`
lines, marked `(forwarded)`. What is easily got wrong `C-c C-c` says
first, and sends at the second: over 25 MB attached, no subject, more
than 10 it goes to, or a file spoken of ("attached", "enclosed") in what
you wrote, not what it quotes or forwards, and none attached.

`d` moves the message to the account's Trash folder. In Trash, `d` puts
it back where it was, or to the inbox if something else had put it there.
letterbox never removes mail from Trash for good: Trash keeps it as long
as its server does, and letterbox lets go only of its own record of where
each came from, after a month. When the server marks no folder as Trash,
letterbox asks which existing folder to use, never the Inbox, Sent or
Drafts, and remembers it on the account:

```toml
trash = "Deleted Items"
```

## What letterbox never does

On 27 October 2026, one key on a marked map trashed some 1,800 messages,
the mail you sent among them, and the filter learnt every one as thrown
away. Nothing was lost, and letterbox since keeps to these, each held by
its tests:

- No key alone selects several, and nothing is selected on the map:
  several are selected with the mouse, in the list or the Inbox looked
  through.
- Nothing done to several is done unasked: trashed, archived, filed or
  moved to another account, a pop-up says how many from where, and only
  `y` does it. Mail in an Inbox and mail you sent are left out of a
  trashing unless taken.
- Nothing is removed from Trash for good by letterbox.
- Nothing done to several, nor the filter's own guess taken, is learnt:
  only what you decided of a message alone.
- The filter throws nothing away on its own: what it is sure of is said
  at the foot, and `J` lists it, each and why, for you to choose.
- What was last moved, however (trashed, archived, filed, or done in the
  Inbox looked through), or a label renamed, split or removed, `C-x u`
  undoes; and `letterbox trash restore` puts back what was trashed in a
  time, each where it came from, `--dry-run` first.
- A folder is deleted only empty, with none under it: a folder deleted
  with mail in it would take the mail with it.
- letterbox does not quit unasked while mail is still being moved, filed
  or sent: cut off, a folder removed would be left half done.
- A message sent is held 10 seconds first, `C-g` taking it back; and no
  subject, many it goes to, or a file spoken of and none attached is
  said before it goes.

## The map

`M` (or `M-g`, ⌘G or Super+G where the terminal passes it on, or a click
on Map at the top right) shows a second way into your mail, in place of the sidebar and the list: every
message of every account in its cluster, near what it is about and who it is
from, as a mosaic, as paperstack shows its themes. Filing a
message is moving it onto a cluster; letterbox learns from what you file
and says what else would go there. `M` or `M-l` goes back to the list.

![letterbox's map of the demo's mail](docs/map.png)

Each cluster is a block, boxed, as big as how many messages it holds,
beside the clusters it lies by, the blocks filling the map between them.
Each message is a tile of its block, bright where it is unread, or found
by a search, and the message picked in the accent's colour; where the
mail is too much for a tile each, those unread at least are seen. On
each block are its name, a label's `◆` with its name, a cluster that is
no label yet `◇` with its three commonest words, or its senders when the
map is laid out by people; how many it holds, and what is new in it.
Mail you sent lies by whom it went to, as mail come in by whom it is
from, so an answer lies beside the mail it answers. Under the map, the
message picked is named: what the keys act on, its block's name lit. The
line below says how faithful the map is: of each message's ten nearest
by what they say, how many are near it on the map too. Below 70%, `M-r`
(or `,` `l`) lays it out again from where it is, the blocks shown where
they would be, to keep with `⏎` or drop with `esc`.

The arrows and `h` `j` `k` `l` go from block to block, and the message
read beside the map follows, with its labels, the cluster it is in, and
the five messages nearest it. `⏎`, `z`, a click or the wheel up over a
block goes into it: a mosaic of its parts, by what they are about, each
a block, its messages each a tile; the way in said over the map, as
`Grant › budget aims`. A part of twelve messages or more is gone into
alike, in finer parts of its own, and so on, four deep at most; one too
small to split shows its threads, by their subject, and their messages,
oldest first, as a tree drawn from the left, so that new mail comes at
the end and nothing else moves. Only the branch picked and those it is
under are open; the rest are closed, as big as how many they hold. In a
tree, `j` and `k` go along a level, `l` or `⏎` into the branch picked and
`h` out of it, the wheel along it, and `⏎` on a message reads it. `esc`,
`Z` or the wheel down goes out a level, and out of the parts back to the
clusters, no farther; `0` goes to all of them at once, and `[` `]` to the
next cluster. A message picked elsewhere, by a search or in the Inbox
looked through, takes the map into its cluster. The map opens at its
clusters; letterbox opens as you left it: the list or the map, and the
message read as wide as you made it. `/` searches it as you type: the messages
whose subject, sender, address, first lines or labels hold every word
are lit, the rest faded, and the line over the map says how many; `n`
and `p` go from one to the next, newest first, and `esc` lets it go.
Narrowed to a cluster (by `[` `]`, its row in the sidebar, or `⏎` on its
block), the search is of that cluster only: the line over the map says
"found in" it, and `n` and `p` never leave it; `0` searches all of the
map again.
`,` opens what the map shows, and how, each
by a key, kept open to change more: `w` lays it out by words and people,
by words alone or by people alone; `t` shows all of it, this week's or
today's; `r` hides read mail, `s` the mail you sent, `f` all but the
mail not yet filed under a label, what is still to be filed, and `n` dims the
clusters with nothing new. A message not yet filed that lies in a
label's cluster would go to that label: the message read says so, and
the clusters, shown with `f`, say which would take what is shown. `y`
files it there, `Y` every message that would go there too, asked first,
how many, and `-` says it is not one for that
label, which the map learns. No subject is read to say where: only what
the mail is about and who it is from, as the map has it. Where a message
would go is voted by the ten filed messages nearest it, within 45°, each
for its labels, the nearer the more; one with none so near is said to go
nowhere. `letterbox map check` grades it on your mail: each filed message,
its labels hidden, put back by the vote, and by other ways to compare.

`i` opens the Inbox beside the map, to look through as far as you like:
every message still in an Inbox, newest first, those kept before after
the rest, each with what may be done with it, `→ Grant`, `keep` or
`discard`, and what is suggested marked `[?]`: filed where it would go,
else kept. Nothing is chosen until you choose it. `m`, or a click on `→`, drops down the labels it
may go to: where it would go first, then the four nearest it by what it
is about; typing narrows them to those that hold what is typed, from all
of them, and offers a new label by that name; the last picks where on the
map. Another account's folders are offered too, as `Grant · ubc.ca`: those
named as a label shown, or holding what is typed. `↑` `↓` pick one and `⏎`
takes it. `N` names a new label
or folder for it, made as it is filed; `space` keeps it, `d`
discards it, and `y` takes what is suggested; each goes on to the next.
`←` and `→` go round them, and back to none chosen; and `C-f` and `C-b`, as Emacs's keys move everywhere; `C-v` and `M-v` a page; a click picks one, and
again leaves it. Nothing is done until `$`, or the Apply button, asks in
a pop-up, saying how many go under each label, to the Trash and stay, and
is told `y` or Do it: then each chosen is filed, kept, or moved to the
Trash, at once, and the rest are left in the Inbox as they are. Each is
followed till its server says it is done and the next look at the mail
finds it so: the foot says how far it has come, and once all have
settled a pop-up says how it went, each that failed with why. What failed
is left where it was, marked `✗`, chosen as it was, for `$` again; `$`
waits while the last is still followed. What is deleted, archived or filed
in the list or on the map is followed alike, one message alone said
`✓ confirmed` at the foot. `esc` goes back to the map and `tab` comes
back; `i` closes it; either way what was chosen is kept until it is done.
While it is looked through, the map shows where it is heading, as
chosen and, where not yet, as suggested: each label's cluster from grey,
where none of it goes, to orange, where most does, with how many beside
its name; the Inbox's own mail in blue; a trail of dots from the message
picked to where it goes, and that label's words, those the message shares
with it lit; under the map, how many go to each label, to the Trash and
stay. As it is looked through, and each choice changed, so does the map,
its clusters alike.

Read with `⏎`, a message is chosen for by `d`, `m` and `y` as it is in the
list, and `u` and `!` mark the one picked.

letterbox learns what you throw away. Each message you trash, however it
is trashed, teaches it that mail like it is thrown away; each you file,
archive, answer, flag, keep in the Inbox looked through, or put back from
the Trash, that it is not. Like is by who sent it, from where, by which
list, and its words, never its subject alone, so that a sender you always
throw away is known by a subject never seen; and by whether you ever wrote
to its sender, as your sent mail says, whether its sender takes no answer
(`noreply` and the like), and whether it was sent to many, saying how to
unsubscribe from it, so that even a sender never seen is told apart. Once it has been taught twenty
of each, a message in the Inbox it is sure of, 90% or more, comes to be
looked through with `[?] discard 92%` suggested; nothing is thrown away
unless you choose it and `$` does it.
What it learns is written down, encrypted with the map's index, as the
journal `taught`, and learnt again as letterbox starts.

What letterbox moved to Trash, from one time to another, is put back
from the shell, each where it was, as `d` in Trash does for one; and
what was learnt from it, forgotten:

```sh
letterbox trash restore --since "2026-10-27 13:50" --until "2026-10-27 14:00" --dry-run
letterbox filter forget --since "2026-10-27 13:50" --until "2026-10-27 14:00"
```

```sh
letterbox filter check    # how it would have done on what you decided
letterbox filter learn    # learn from the Trash and your labels too, then check
letterbox filter forget   # forget what was learnt from them, then check
letterbox filter debrief  # what was thrown away as junk, and why
letterbox filter auto off # ask about nothing as it comes; `on` again
```

New mail it is 95% sure is junk, from the first time this letterbox
opens, is said at the foot as it comes, never thrown away unless you
choose it, and never in a pop-up of its own, which a key pressed as it
came would act on. `J` lists each, who sent it, what about, how sure it
is, and why, the few things that weighed most, for you to say whether it
is: `space` chooses one, none chosen till you choose, and `d` throws those
chosen away; `n` says the one picked is not junk, and that is learnt;
`i` looks through the Inbox, each suggested and none chosen; any other
key leaves them. Each is said once; mail you have decided on never is. What is thrown away so is
written down, with why, but never learnt from, so a wrong guess does not
teach it to guess so again. As letterbox opens, it tells of what was
thrown away so since it last told: how sure it was, who sent
each, what about, and the few things that weighed most, as *sent to many
· takes no answer · list news.example*; then what it learnt since, and
how it does now. `⏎` puts the one picked back, `u` all of them, each
learnt as kept. `B` tells of the last seven days again, and
`letterbox filter debrief` says the same on the command line.

`filter check` goes through every decision in turn and says, of each,
what it would have said from those before it alone, before learning it,
so it is never graded on what it has already seen. It counts how many of
what you threw away it would have caught, and how many you kept it would
have thrown away, sure to 70%, 80%, 90% and 95%, and names the ones kept,
which matter most. `filter learn` reads, on each account's server, the
newest 500 messages (`--limit`) of its Trash that were never read, as
thrown away, and the newest 500 filed under your labels, as kept, oldest
first, what you filed read again from its server first, as the index
now reads a message; each only once. Mail read and then thrown away was dealt with, not
unwanted, so it is left out: learnt from, it took meetings and colleagues'
mail for what you throw away. `filter forget` takes back what was learnt
from either, and your own decisions are all that is left. Both write
what letterbox writes, so each waits till letterbox is closed; one
letterbox writes the map's index at a time, another opened beside it
reads it, and says so.

The map has no sidebar beside it, as its
clusters and labels are on it; `tab` goes between it and the message
read. In the list, the sidebar lists the clusters too, and the messages
that fit nowhere; `M-l` lists the cluster the map is narrowed to.

The map is looked at: on its clusters no key moves mail, as one is so
easily pressed on the wrong one. `⏎` goes down a cluster to a message,
and there, or reading it, `d`, `e`, `m`, `y` and `-` act on it alone. `m`
moves it onto a cluster: `enter` on its block, or a click on it. It is
given the cluster's label and taken out of the Inbox; `N` gives it a new
label instead. Several are acted on together in the list, or in the
Inbox looked through, selected with the mouse, and asked first.

On the clusters, a label is changed whole, by a right-click on its
anchor, `↑` `↓` and `⏎`; each shown first, done only on `y`, and undone
by `C-x u`:

- **Rename…**, as `R`: never onto a label there is.
- **Split into subfolders…**: its mail, twelve or more, in parts by what
  each is about, as a cluster gone into is, each a subfolder named by
  the two words most its own, `⏎` typing a name over. On a server of
  folders each is moved to its subfolder; on Gmail its label nested
  under this one; a keyword, or a label letterbox keeps, given beside it.
- **Remove…**: each of its messages filed where the map would file it
  were there no such label, as the filed mail nearest it votes, else
  unfiled, to Archive; then its folder deleted, only once its server
  finds it empty. No message is thrown away, and a label with folders
  under it is not removed.

How far each has come is said at the foot as its servers answer.

A message moves to another account's folder, picked in the labels dropped
down as `Grant · ubc.ca`, from the list or the Inbox looked through: read
whole from its server, copied to the other, found there by its
Message-ID, and only then its original put in its own account's Trash,
as `d` puts one, to be put back from there. A copy refused, or not found
where it went, leaves the original where it was; each is followed till
confirmed, and what fails is said, as for the rest. Once a message is filed, the message read says what would go to
the label too: `y` files the first, `Y` all of them, and `-` says the
first is not one, which the map learns from. `R` renames a label.

Labels are flat: a message may have several, and a label of one name is
the same label on every account. Folders are labels, every level of
them: `Projects/Grant` is `Projects` and `Grant`. A label is kept where
the server keeps labels: on Gmail, as a Gmail label, the message's
`\Inbox` taken off; on a server that takes keywords of any name, as
Dovecot's and Fastmail's do, as a keyword, the message moved from the
Inbox to Archive; on one of folders alone, as Microsoft's, by moving the
message to the folder the label is, any further label kept by letterbox.

Nothing is read for the map until it is wanted: the first time it is
opened, letterbox says what it will do and asks, and makes its key and
its index only once you agree. From then on it begins with letterbox.

The map reads every folder but Junk, Trash and Drafts, the newest first,
on a connection of its own, in the background, and then what is new
every two minutes. Its index is kept beside the vault, in `map/`
(`$LETTERBOX_MAP`), encrypted with a key letterbox makes, whose secret
is kept where passwords are, in the keyring or the vault. Nothing of
your mail is written there in the clear, and the vectors the map is
drawn from are not written at all: they are made again from the mail.

```sh
lbx map rebuild                 # lay the map out again, the same mail the same map
lbx map rebuild --from-servers  # read every account again first; your labels stay
lbx map key --export            # the map's key, as age and rage read one
```

## Sending

A message is sent by SMTP submission, on port 587, over TLS once the server
is asked for it. The server is the one given as the account was added
(`Outgoing`, as `smtp.example.com:465` with a port), else guessed: Microsoft's
`smtp.office365.com` for an account signed in on Microsoft's page, whatever
its address; the provider's, where the IMAP server is a provider's (Gmail's
`smtp.gmail.com` for a domain on Google Workspace too); else the provider's
of the address, or `smtp.` and the domain. An account may name another in
the file:

```toml
smtp_host = "smtp.example.com"
# smtp_port = 465
```

Port 465 is TLS from the start. The same password signs in, or, for an
account signed in on Microsoft's page, the same token. Sending needs
Microsoft's permission to send, which a token saved before letterbox asked
for it does not have: sign in again, and the status line says so when the
server refuses it. Gmail files the copy itself; other accounts have it
added to Sent.

`--demo` writes the message into Sent and does not send it.

Moving through the list, the message picked is shown beside it, and marked
read, once you pause on it, as a mail app does. Another folder listed,
the message beside it goes, as it is not in that list. The mouse picks
and scrolls, and presses the keys shown at the foot and the forms' fields
and buttons; drag the line before the message read, or press `<` and
`>`, to make it wider or narrower. Each account is asked from a thread of its own, so the screen
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
| `nearest_messages` | the messages nearest one on the map, by what they are about and who they are from, each with its labels and how far, in degrees | reads |
| `suggest_label` | the label a message would be filed under, and the labels nearest it | reads |
| `would_throw_away` | how likely you are to throw it away, as letterbox has learnt, and why | reads |
| `mark_message` | mark a message read or unread, flagged or not | changes |

All but the last are marked read-only, so a program may let the agent use
them without asking. The map's index is read as letterbox last kept it,
and never written, so an agent may ask while letterbox runs. Nothing is sent, moved or deleted. Mail is from
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
