//! The command line: each command and its flags, as clap reads them.
use super::*;

#[derive(Parser)]
#[command(version, about)]
pub(super) struct Args {
    /// The accounts: a TOML file of [[account]] tables
    #[arg(long, env = "LETTERBOX_CONFIG", default_value_os_t = config::default_path(), global = true)]
    pub(super) config: PathBuf,
    /// Made-up mail to try letterbox with, instead of your accounts
    #[arg(long, global = true)]
    pub(super) demo: bool,
    /// The TUI's colours: a light page, or the terminal's own dark one
    #[arg(
        long,
        env = "LETTERBOX_THEME",
        value_enum,
        default_value_t,
        global = true
    )]
    pub(super) theme: theme::Name,
    #[command(subcommand)]
    pub(super) command: Option<Command>,
}

#[derive(clap::Args)]
pub(super) struct Which {
    /// The account, by its name; else the first
    #[arg(long, short)]
    pub(super) account: Option<String>,
    /// The folder, by its name on the server
    #[arg(long, short, default_value = "INBOX")]
    pub(super) folder: String,
}

#[derive(Subcommand)]
pub(super) enum Command {
    /// The accounts set up, and where; add and remove them
    Accounts {
        #[command(subcommand)]
        action: Option<AccountAction>,
        /// Sign in to each, and say what went wrong
        #[arg(long)]
        check: bool,
    },
    /// An account's folders, and how many messages each holds
    Folders {
        #[arg(long, short)]
        account: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// A folder's newest messages
    List {
        #[command(flatten)]
        which: Which,
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// A message, by its UID in the folder; it is not marked read
    Read {
        uid: u32,
        #[command(flatten)]
        which: Which,
        #[arg(long)]
        json: bool,
        /// Its bytes, as sent
        #[arg(long, conflicts_with = "json")]
        raw: bool,
    },
    /// The messages in a folder with all of these words, on the server
    Search {
        #[arg(required = true)]
        words: Vec<String>,
        #[command(flatten)]
        which: Which,
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Move every password letterbox keeps, and the map's key, to the
    /// system's store or to letterbox's vault; each is forgotten where it
    /// was only once it is kept in the other
    Keep {
        #[arg(value_enum)]
        to: secret::Keeper,
    },
    /// letterbox's vault: give it a new passphrase, or a key file instead
    Vault {
        #[command(subcommand)]
        action: VaultAction,
    },
    /// The map of every account's mail: lay it out again, or say its key
    Map {
        #[command(subcommand)]
        action: MapAction,
    },
    /// What letterbox has learnt of the mail you throw away: how well it
    /// would have done, or learnt from the Trash and your labels too
    Filter {
        #[command(subcommand)]
        action: FilterAction,
    },
    /// Serve the mail to an AI agent over MCP, on stdin and stdout: its
    /// program, as Cursor or Claude Code, runs `letterbox mcp`
    Mcp,
    /// The TUI's screen after the keys given, as text: `j <enter>`
    #[command(hide = true)]
    Screen {
        #[arg(long, default_value_t = 140)]
        width: u16,
        #[arg(long, default_value_t = 40)]
        height: u16,
        /// As an SVG, with its colours
        #[arg(long)]
        svg: bool,
        keys: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(super) enum AccountAction {
    /// Add an account, asking for what is not given; it signs in first, and
    /// its password is kept in the keychain, not the file
    Add {
        #[arg(long)]
        email: Option<String>,
        /// What it is called here; its address's domain, else
        #[arg(long)]
        name: Option<String>,
        /// The IMAP server; the provider's, or imap. and the domain, else
        #[arg(long)]
        host: Option<String>,
        #[arg(long, default_value_t = 993)]
        port: u16,
        /// The name signed in with, if not the address
        #[arg(long)]
        user: Option<String>,
        /// The server mail is sent through, and its port after a colon if
        /// not 587; Microsoft's for an account signed in there, the
        /// provider's, or smtp. and the domain, else
        #[arg(long)]
        outgoing: Option<String>,
        /// A command that prints the password, instead of keeping it
        #[arg(long, conflicts_with = "keep")]
        password_command: Option<String>,
        /// Where to keep the password, or Microsoft's token: the system's
        /// store, unless the vault is asked for
        #[arg(long, value_enum)]
        keep: Option<secret::Keeper>,
        /// Sign in with a password, or on Microsoft's page; Microsoft's for
        /// an address Microsoft says is its own
        #[arg(long, value_enum)]
        auth: Option<SignIn>,
    },
    /// Remove an account; its mail stays on its server
    Remove {
        name: String,
        /// Keep the password letterbox kept for it
        #[arg(long)]
        keep_password: bool,
    },
    /// Call an account something else; only letterbox's name for it changes
    Rename { name: String, new: String },
}

#[derive(Subcommand)]
pub(super) enum VaultAction {
    /// A new passphrase for it, asked for as letterbox starts; or none, its
    /// key a file beside it only you may read, never asked for
    Passphrase,
}

#[derive(Subcommand)]
pub(super) enum FilterAction {
    /// How it would have done: each decision said before it was learnt,
    /// from those before it alone; what it caught, and what you kept that
    /// it would have thrown away
    Check {
        /// How much of each weight is kept a decision, to try another
        #[arg(long, hide = true)]
        decay: Option<f32>,
    },
    /// Learn from mail already in each account's Trash, thrown away
    /// unread, as thrown away, and filed under your labels, as kept, the
    /// newest of each; then check
    Learn {
        /// How many of each, at most, from each place
        #[arg(long, default_value_t = 500)]
        limit: usize,
    },
    /// Forget what was learnt from the Trash and your labels: only your
    /// own decisions are kept; then check
    Forget,
    /// Throw new mail away on its own, when 95% sure, or not: told of as
    /// letterbox opens, and by B, never learnt from
    Auto {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// What was thrown away on its own, and why, and what was learnt since
    Debrief {
        /// As far back as this many days
        #[arg(long, default_value_t = 7)]
        days: i64,
    },
}

#[derive(Subcommand)]
pub(super) enum MapAction {
    /// Lay the map out again by every lens, from nothing, as its seed has
    /// it: the same mail, the same map
    Rebuild {
        /// Read every account's mail again first: what the servers said is
        /// let go, and what only letterbox knows is kept
        #[arg(long)]
        from_servers: bool,
    },
    /// How well the map would file what you filed: each message under a
    /// label, its own hidden, put back by the label nearest it
    Check,
    /// The map's key: where it is kept
    Key {
        /// Print its secret, as age and rage read a key file
        #[arg(long)]
        export: bool,
    },
}
