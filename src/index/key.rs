//! The map's key: an age x25519 key letterbox makes for itself. Its secret
//! half is kept where passwords are, in the system's store or the vault;
//! its public half is in `map.toml`, in the clear, beside the journals,
//! with where the secret is. The journals are encrypted to the public half,
//! so what is new is written down without the secret, which is needed
//! only to read them back.

use crate::secret::Keeper;
use age::secrecy::ExposeSecret;
use age::x25519::Identity;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The index's version, as `map.toml` says it.
pub const VERSION: u32 = 1;
const CARD: &str = "map.toml";
/// What the secret is kept as, as a password is kept for a user at a
/// host.
const HOST: &str = "map";
const USER: &str = "identity";

/// What `map.toml` says: the index's version, the key's public half, and
/// where its secret is kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub version: u32,
    pub recipient: String,
    pub keeper: Keeper,
}

/// `$LETTERBOX_MAP`, else `map` beside the vault, in its folder.
pub fn dir() -> PathBuf {
    if let Some(p) = std::env::var_os("LETTERBOX_MAP").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    crate::vault::path()
        .parent()
        .map(|d| d.join("map"))
        .unwrap_or_else(|| PathBuf::from("map"))
}

/// What `map.toml` in `dir` says, if there is one.
pub fn card(dir: &Path) -> anyhow::Result<Option<Card>> {
    let path = dir.join(CARD);
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .with_context(|| format!("cannot read {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Make a key, keep its secret in `keeper` by `keep`, and only then write
/// `map.toml` in `dir`: a card is never left naming a secret not kept.
pub fn make(
    dir: &Path,
    keeper: Keeper,
    keep: crate::setup::Keep,
) -> anyhow::Result<(Card, Identity)> {
    let identity = Identity::generate();
    keep(keeper, HOST, USER, identity.to_string().expose_secret())
        .context("cannot keep the map's key")?;
    let card = Card {
        version: VERSION,
        recipient: identity.to_public().to_string(),
        keeper,
    };
    write(dir, &card)?;
    Ok((card, identity))
}

/// Write `card` as `map.toml` in `dir`, beside it first.
pub fn write(dir: &Path, card: &Card) -> anyhow::Result<()> {
    super::journal::create_private(dir)
        .with_context(|| format!("cannot make {}", dir.display()))?;
    let path = dir.join(CARD);
    let tmp = path.with_extension("toml.new");
    let text = format!(
        "# letterbox's map of your mail. Its index is encrypted to this key;\n\
         # the secret half is kept {}.\n{}",
        card.keeper.about(),
        toml::to_string(card)?
    );
    crate::vault::write_private(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))
}

/// The key's secret, from where `card` says it is kept, by `find`.
pub fn identity(
    card: &Card,
    find: fn(Keeper, &str, &str) -> anyhow::Result<String>,
) -> anyhow::Result<Identity> {
    let secret = find(card.keeper, HOST, USER).context("cannot find the map's key")?;
    let identity: Identity = secret
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("the map's key is not one letterbox can use: {e}"))?;
    if identity.to_public().to_string() != card.recipient {
        anyhow::bail!("the map's key is not the one map.toml names");
    }
    Ok(identity)
}

/// The secret of `identity`, as `age` and `rage` read a key file.
pub fn export(identity: &Identity) -> String {
    format!(
        "# letterbox's map key\n# public key: {}\n{}\n",
        identity.to_public(),
        identity.to_string().expose_secret()
    )
}
