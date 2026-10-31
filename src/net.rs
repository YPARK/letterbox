//! The web, as letterbox asks it: HTTPS in pure Rust, rustls with
//! RustCrypto's algorithms, as for IMAP; for Microsoft's sign-in, and for
//! the documents a message links to.

use anyhow::{Context, bail};
use std::time::Duration;

/// The largest document downloaded.
const LIMIT: u64 = 200 << 20;
const BROWSER: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                       (KHTML, like Gecko) Chrome/141.0 Safari/537.36";

pub fn agent() -> ureq::Agent {
    // Once a process; again, it is already the provider.
    let _ = rustls_rustcrypto::provider().install_default();
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .user_agent(concat!("letterbox/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The largest picture fetched for a page.
#[cfg(feature = "pages")]
const PICTURE: u64 = 10 << 20;

/// The picture at `url`, its type and bytes; anything else is none.
#[cfg(feature = "pages")]
pub fn image(agent: &ureq::Agent, url: &str) -> Option<(String, Vec<u8>)> {
    let mut resp = agent
        .get(url)
        .header("User-Agent", BROWSER)
        .header("Accept", "image/*")
        .call()
        .ok()?;
    let mime = resp
        .headers()
        .get("content-type")?
        .to_str()
        .ok()?
        .split(';')
        .next()?
        .trim()
        .to_lowercase();
    if !resp.status().is_success() || !mime.starts_with("image/") {
        return None;
    }
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(PICTURE)
        .read_to_vec()
        .ok()?;
    Some((mime, bytes))
}

/// Whether `name` is a PDF's, by its extension.
pub fn is_pdf(name: &str) -> bool {
    name.trim().to_lowercase().ends_with(".pdf")
}

/// The address of the PDF a link stands for, where it is plain which: an
/// arXiv paper's page, as its PDF; an address ending `.pdf`; else none.
pub fn pdf_of(url: &str) -> Option<String> {
    let bare = url.split(['?', '#']).next().unwrap_or(url);
    for page in ["arxiv.org/abs/", "arxiv.org/pdf/"] {
        if let Some((_, id)) = bare.split_once(page) {
            let id = id.trim_end_matches('/').trim_end_matches(".pdf");
            return (!id.is_empty()).then(|| format!("https://arxiv.org/pdf/{id}"));
        }
    }
    is_pdf(bare).then(|| url.to_string())
}

/// The PDF at `url`, and a name for it, from its address; anything else, as
/// a page asking to sign in, is an error.
pub fn pdf(url: &str) -> anyhow::Result<(String, Vec<u8>)> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        bail!("{url} is not a web address");
    }
    let mut resp = agent()
        .get(url)
        .header("User-Agent", BROWSER)
        .header("Accept", "application/pdf,*/*;q=0.5")
        .call()
        .with_context(|| format!("cannot download {url}"))?;
    if !resp.status().is_success() {
        bail!("{url} gave {}", resp.status());
    }
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(LIMIT)
        .read_to_vec()
        .with_context(|| format!("cannot download {url}"))?;
    if !bytes.starts_with(b"%PDF") {
        bail!("{url} gave no PDF: it may need signing in, in the browser");
    }
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.trim_end_matches('/').rsplit('/').next())
        .filter(|n| !n.is_empty())
        .unwrap_or("document");
    let name = if is_pdf(name) {
        name.to_string()
    } else {
        format!("{name}.pdf")
    };
    Ok((name, bytes))
}
