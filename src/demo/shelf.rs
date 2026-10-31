//! The demo's library of papers, in memory: trying letterbox never adds
//! to your own paperstack.

/// A library in memory, as paperstack would answer: the demo's, so that
/// trying it never adds to your own; and the tests'.
#[derive(Default)]
pub struct Shelf {
    pub papers: Vec<crate::paperstack::Found>,
    pub added: Vec<String>,
}

impl crate::paperstack::Library for Shelf {
    fn search(
        &mut self,
        query: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<crate::paperstack::Found>> {
        let starred = query.contains("is:starred");
        let words: Vec<String> = query
            .replace("is:starred", "")
            .replace("title:", "")
            .replace('"', " ")
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        Ok(self
            .papers
            .iter()
            .filter(|p| !starred || p.starred)
            .filter(|p| {
                let t = p.title.to_lowercase();
                words.iter().all(|w| t.contains(w.as_str()))
            })
            .cloned()
            .collect())
    }
    fn add(&mut self, what: &str) -> anyhow::Result<crate::paperstack::Found> {
        self.added.push(what.to_string());
        let found = crate::paperstack::Found {
            key: format!("Key{}", self.added.len()),
            title: what.to_string(),
            starred: false,
        };
        self.papers.push(found.clone());
        Ok(found)
    }
    fn star(&mut self, key: &str) -> anyhow::Result<()> {
        for p in self.papers.iter_mut().filter(|p| p.key == key) {
            p.starred = true;
        }
        Ok(())
    }
}

/// The demo's library: the Scholar alert's first paper in it, starred, and
/// one like its second.
pub fn shelf() -> Shelf {
    let paper = |key: &str, title: &str| crate::paperstack::Found {
        key: key.to_string(),
        title: title.to_string(),
        starred: true,
    };
    Shelf {
        papers: vec![
            paper(
                "Kanai2026",
                "Population-scale immune multiome atlas reveals regulatory disease mechanisms",
            ),
            paper(
                "Theodoris2023",
                "Transfer learning enables predictions in network biology",
            ),
        ],
        added: Vec::new(),
    }
}
