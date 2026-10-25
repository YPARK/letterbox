//! The papers a message tells of: a Google Scholar alert's, a journal's
//! table of contents, an arXiv listing. Each is found by its link, the
//! address Scholar or a tracker sends it on to read through; by its DOI or
//! arXiv id; or, in mail from a journal, by a link whose words are a title.
//! None is fetched: what is found is what the message says.

/// A paper a message tells of: its title, who wrote it where, and what
/// paperstack is to be given to add it, a DOI, an arXiv id, an address or
/// the title itself; and its link, to open.
#[derive(Clone, Debug, PartialEq)]
pub struct Paper {
    pub title: String,
    pub about: String,
    pub what: String,
    pub link: Option<String>,
}

/// Senders whose mail is papers: alerts, tables of contents, listings.
const SOURCES: &[&str] = &[
    "scholar.google.com",
    "google.com",
    "nejm.org",
    "nature.com",
    "cell.com",
    "science.org",
    "sciencemag.org",
    "elsevier.com",
    "sciencedirect.com",
    "springer.com",
    "springernature.com",
    "wiley.com",
    "biorxiv.org",
    "medrxiv.org",
    "arxiv.org",
    "plos.org",
    "pnas.org",
    "oup.com",
    "ncbi.nlm.nih.gov",
    "researchgate.net",
];

/// Where a link that says it is a paper goes.
const PAPER_HOSTS: &[&str] = &[
    "doi.org",
    "arxiv.org",
    "biorxiv.org",
    "medrxiv.org",
    "nature.com",
    "cell.com",
    "science.org",
    "nejm.org",
    "pnas.org",
    "plos.org",
    "sciencedirect.com",
    "springer.com",
    "wiley.com",
    "academic.oup.com",
    "pubmed.ncbi.nlm.nih.gov",
    "ncbi.nlm.nih.gov/pmc",
    "openreview.net",
    "aclanthology.org",
    "proceedings.mlr.press",
    "papers.nips.cc",
];

/// Words of a link that say it is no paper.
const NOT_PAPERS: &[&str] = &[
    "unsubscribe",
    "table of contents",
    "view in browser",
    "privacy",
    "preferences",
    "manage",
    "alert",
    "sign in",
    "subscribe",
    "contact us",
    "terms of",
    "advertis",
    "follow us",
];

/// The papers the message from `from` tells of, in `html`, else in `text`,
/// each once, in the order told.
pub fn in_message(from: &str, html: Option<&str>, text: &str) -> Vec<Paper> {
    let source = from
        .rsplit_once('@')
        .map(|(_, d)| d.to_lowercase())
        .is_some_and(|d| {
            SOURCES
                .iter()
                .any(|s| d == *s || d.ends_with(&format!(".{s}")))
        });
    let mut found = Vec::new();
    if let Some(html) = html {
        for (href, inner, after) in anchors(html) {
            let title = clean(&text_of(&inner));
            if !titled(&title) {
                continue;
            }
            let link = unwrapped(&href);
            let what = doi_of(&link)
                .or_else(|| arxiv_of(&link).map(|id| format!("arXiv:{id}")))
                .or_else(|| paper_host(&link).then(|| link.clone()))
                .or_else(|| source.then(|| title.clone()));
            let Some(what) = what else {
                continue;
            };
            found.push(Paper {
                about: about_after(&after),
                title,
                what,
                link: Some(link),
            });
        }
    }
    if found.is_empty() {
        found.extend(arxiv_listing(text));
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|p| seen.insert(fold(&p.title)));
    found
}

/// Whether `title` reads as a paper's: several words, and none of those
/// that say a link is something else.
fn titled(title: &str) -> bool {
    let lower = title.to_lowercase();
    title.split_whitespace().count() >= 4
        && title.chars().count() >= 25
        && !NOT_PAPERS.iter().any(|n| lower.contains(n))
}

/// `title` folded, to tell the same paper twice.
pub fn fold(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Each link of `html`: where it goes, what is inside it, and what follows
/// it, up to the next link.
fn anchors(html: &str) -> Vec<(String, String, String)> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<a ").map(|i| at + i) {
        let Some(open_end) = lower[start..].find('>').map(|i| start + i) else {
            break;
        };
        let Some(close) = lower[open_end..].find("</a>").map(|i| open_end + i) else {
            break;
        };
        let href = attribute(&html[start..open_end], "href").unwrap_or_default();
        let inner = html[open_end + 1..close].to_string();
        let next = lower[close..].find("<a ").map_or(html.len(), |i| close + i);
        out.push((decode(&href), inner, html[close + 4..next].to_string()));
        at = close + 4;
    }
    out
}

/// The value of `name` in the tag `tag`, quoted or not.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let i = from + lower[from..].find(name)?;
        let before = lower[..i].chars().last();
        let rest = tag[i + name.len()..].trim_start();
        if before.is_some_and(char::is_whitespace)
            && let Some(rest) = rest.strip_prefix('=')
        {
            let rest = rest.trim_start();
            return Some(match rest.chars().next() {
                Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default().to_string(),
                _ => rest
                    .split(|c: char| c.is_whitespace() || c == '>')
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        from = i + name.len();
    }
}

/// What `html` says, its tags taken out and its entities read.
fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode(&out)
}

/// Whitespace made single spaces, and a space before punctuation that
/// a tag left taken out.
fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `s` with HTML's entities read: named ones mail uses, and numbered ones.
fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "hellip" => Some('…'),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "rdquo" => Some('”'),
            "ldquo" => Some('“'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Where `href` sends one on to, through Scholar's and Google's redirects.
fn unwrapped(href: &str) -> String {
    for (host, key) in [
        ("scholar.google.", "url="),
        ("www.google.", "q="),
        ("www.google.", "url="),
    ] {
        if href.contains(host)
            && let Some(q) = href.split_once('?').map(|(_, q)| q)
            && let Some(v) = q.split('&').find_map(|kv| kv.strip_prefix(key))
            && v.starts_with("http")
        {
            return percent_decoded(v);
        }
    }
    href.to_string()
}

fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b[i], hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (v, _) => {
                out.push(v);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The DOI in `link`'s path: `10.`, digits, `/`, up to its query.
fn doi_of(link: &str) -> Option<String> {
    let path = link.split(['?', '#']).next()?;
    let i = path.find("/10.")? + 1;
    let doi = &path[i..];
    let (prefix, suffix) = doi.split_once('/')?;
    let digits = &prefix[3..];
    (digits.len() >= 4 && digits.chars().all(|c| c.is_ascii_digit()) && !suffix.is_empty())
        .then(|| doi.trim_end_matches(['.', '/']).to_string())
}

/// The arXiv id in `link`: what follows `/abs/` or `/pdf/`.
fn arxiv_of(link: &str) -> Option<String> {
    if !link.contains("arxiv.org/") {
        return None;
    }
    let path = link.split(['?', '#']).next()?;
    let (_, id) = path
        .split_once("/abs/")
        .or_else(|| path.split_once("/pdf/"))?;
    let id = id.trim_end_matches(".pdf").trim_end_matches('/');
    id_like(id).then(|| id.to_string())
}

/// Whether `id` is a new-style arXiv id, `2410.01234`, a version after it
/// or not.
fn id_like(id: &str) -> bool {
    let id = id.split('v').next().unwrap_or(id);
    let Some((a, b)) = id.split_once('.') else {
        return false;
    };
    a.len() == 4
        && (4..=5).contains(&b.len())
        && (a.to_owned() + b).chars().all(|c| c.is_ascii_digit())
}

/// Whether `link` goes to where papers are kept.
fn paper_host(link: &str) -> bool {
    let rest = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    // A journal's mail is sent through trackers of its own: they tell
    // nothing of where the paper is.
    if [
        "click.", "links.", "link.", "email.", "e.", "t.", "mail.", "track",
    ]
    .iter()
    .any(|t| rest.starts_with(t))
    {
        return false;
    }
    PAPER_HOSTS.iter().any(|h| {
        rest.starts_with(h)
            || rest
                .split_once('/')
                .is_some_and(|(host, _)| host.ends_with(&format!(".{h}")))
    })
}

/// Who wrote it where, as the line after its link says: the first words
/// that are not its abstract.
fn about_after(html: &str) -> String {
    // Scholar's authors' line is a block of its own, its snippet the next.
    let first = html
        .split("</div>")
        .map(|d| clean(&text_of(d)))
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let short: String = first.chars().take(120).collect();
    if short.chars().count() < first.chars().count() {
        format!("{short}…")
    } else {
        short
    }
}

/// The papers of an arXiv listing: each `arXiv:` id and the `Title:` after
/// it, its lines run together.
fn arxiv_listing(text: &str) -> Vec<Paper> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let Some(id) = line.trim().strip_prefix("arXiv:") else {
            continue;
        };
        let id = id.split_whitespace().next().unwrap_or_default();
        if !id_like(id) {
            continue;
        }
        let Some(t) = lines[i + 1..]
            .iter()
            .take(6)
            .position(|l| l.starts_with("Title:"))
            .map(|t| i + 1 + t)
        else {
            continue;
        };
        let mut title = lines[t].trim_start_matches("Title:").trim().to_string();
        for more in lines[t + 1..].iter().take_while(|l| l.starts_with(' ')) {
            title.push(' ');
            title.push_str(more.trim());
        }
        out.push(Paper {
            title: clean(&title),
            about: String::new(),
            what: format!("arXiv:{id}"),
            link: Some(format!("https://arxiv.org/abs/{id}")),
        });
    }
    out
}
