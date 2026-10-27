//! The message being written, its grammar checked: what is written of it,
//! not its head, what it quotes nor its signature, sentence by sentence,
//! each made over by a model and said as the least to change, word by
//! word. The model is a small T5 from Hugging Face, read the first time it
//! is asked for, behind the `grammar` feature.

#[cfg(feature = "grammar")]
mod t5;

/// The model asked for when none is named: small, quick and MIT.
pub const DEFAULT: &str = "visheratin/t5-efficient-tiny-grammar-correction";

/// What a model would change: on the row `row`, the characters from
/// `start` to `end`, `said`, to be `with`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub row: usize,
    pub start: usize,
    pub end: usize,
    pub said: String,
    pub with: String,
}

/// A sentence to be checked: on the row `row`, from the character `start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub row: usize,
    pub start: usize,
    pub text: String,
}

/// What makes a sentence over, as it should be written.
pub trait Corrector: Send {
    fn correct(&mut self, sentence: &str) -> anyhow::Result<String>;
}

/// A sentence made over by a function, as a model would: for tests.
#[cfg(test)]
pub struct Using<F>(pub F);

#[cfg(test)]
impl<F: FnMut(&str) -> anyhow::Result<String> + Send> Corrector for Using<F> {
    fn correct(&mut self, sentence: &str) -> anyhow::Result<String> {
        (self.0)(sentence)
    }
}

/// The model `name`, from Hugging Face, read: downloaded the first time.
#[cfg(feature = "grammar")]
pub fn open(name: &str) -> anyhow::Result<Box<dyn Corrector>> {
    Ok(Box::new(t5::Model::open(name)?))
}

/// Built without the `grammar` feature, no model is at hand.
#[cfg(not(feature = "grammar"))]
pub fn open(_name: &str) -> anyhow::Result<Box<dyn Corrector>> {
    anyhow::bail!("letterbox was built without grammar: cargo install letterbox --features grammar")
}

/// What is written of a message, as `lines`, in sentences: past its head,
/// to its signature, and none of what it quotes or the line saying who
/// wrote it.
pub fn pieces(lines: &[String]) -> Vec<Piece> {
    let body = lines
        .iter()
        .position(|l| l.trim().is_empty())
        .map_or(0, |b| b + 1);
    let mut out = Vec::new();
    for (row, line) in lines.iter().enumerate().skip(body) {
        if line == "-- " {
            break;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('>') || trimmed.ends_with("wrote:") {
            continue;
        }
        out.extend(
            sentences(line)
                .into_iter()
                .map(|(start, text)| Piece { row, start, text }),
        );
    }
    out
}

/// The sentences of `line`, each with the character it starts at: ended by
/// `.`, `!` or `?` and a space, or by the line's end; none without a
/// letter.
fn sentences(line: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..=chars.len() {
        let ends = i == chars.len()
            || (matches!(chars[i], '.' | '!' | '?')
                && chars.get(i + 1).is_none_or(|c| c.is_whitespace()));
        if !ends {
            continue;
        }
        let end = (i + 1).min(chars.len());
        let text: String = chars[start..end].iter().collect();
        let lead = text.chars().take_while(|c| c.is_whitespace()).count();
        let text = text.trim().to_string();
        if text.chars().any(char::is_alphabetic) {
            out.push((start + lead, text));
        }
        start = end;
    }
    out
}

/// What `corrector` would change of what is written in `lines`.
pub fn check(lines: &[String], corrector: &mut dyn Corrector) -> anyhow::Result<Vec<Edit>> {
    let mut out = Vec::new();
    for piece in pieces(lines) {
        let made = corrector.correct(&piece.text)?;
        let made = made.trim();
        if made != piece.text {
            out.extend(edits(piece.row, piece.start, &piece.text, made));
        }
    }
    Ok(out)
}

/// The words of `text`, each with the characters it is from and to.
fn words(text: &str) -> Vec<(usize, usize, &str)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (i, part) in text.split(' ').enumerate() {
        if i > 0 {
            at += 1;
        }
        let n = part.chars().count();
        if !part.is_empty() {
            out.push((at, at + n, part));
        }
        at += n;
    }
    out
}

/// What makes `was`, on the row `row` from the character `start`, into
/// `is`: each run of words changed, the fewest, by the longest run of words
/// both share.
pub fn edits(row: usize, start: usize, was: &str, is: &str) -> Vec<Edit> {
    let (a, b) = (words(was), words(is));
    let (n, m) = (a.len(), b.len());
    // The longest common run of words, from each place on.
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i].2 == b[j].2 {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let chars: Vec<char> = was.chars().collect();
    let said = |from: usize, to: usize| chars[from..to].iter().collect::<String>();
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i].2 == b[j].2 {
            i += 1;
            j += 1;
            continue;
        }
        // A run changed: as far as the two go apart.
        let (i0, j0) = (i, j);
        while (i < n || j < m) && !(i < n && j < m && a[i].2 == b[j].2) {
            if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
        let with: Vec<&str> = b[j0..j].iter().map(|w| w.2).collect();
        let with = with.join(" ");
        let (from, to, with) = if i0 == i {
            // Words put in: before the next word, or after the last.
            match a.get(i0) {
                Some(next) => (next.0, next.0, format!("{with} ")),
                None => {
                    let end = a.last().map_or(0, |w| w.1);
                    (end, end, format!(" {with}"))
                }
            }
        } else if j0 == j {
            // Words taken out, and a space beside them.
            match a.get(i) {
                Some(next) => (a[i0].0, next.0, String::new()),
                None if i0 > 0 => (a[i0 - 1].1, a[i - 1].1, String::new()),
                None => (a[i0].0, a[i - 1].1, String::new()),
            }
        } else {
            (a[i0].0, a[i - 1].1, with)
        };
        out.push(Edit {
            row,
            start: start + from,
            end: start + to,
            said: said(from, to),
            with,
        });
    }
    out
}
