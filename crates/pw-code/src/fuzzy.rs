//! Fuzzy matching of file paths for Quick open, on helix's `nucleo` matcher.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Index into the candidates.
    pub index: usize,
    pub score: u32,
    /// Matched character positions (char indices, ascending), for highlighting.
    pub positions: Vec<u32>,
}

/// A reusable matcher (it keeps scratch memory between queries).
pub struct Fuzzy {
    matcher: Matcher,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Self { matcher: Matcher::new(Config::DEFAULT.match_paths()) }
    }
}

impl Fuzzy {
    /// The best `limit` candidates for `query`, best first. A file-name match beats a match spread over
    /// folders; ties go to the shorter path. An empty query keeps the candidates' order.
    pub fn rank<S: AsRef<str>>(&mut self, query: &str, candidates: &[S], limit: usize) -> Vec<Match> {
        if query.trim().is_empty() {
            return (0..candidates.len().min(limit))
                .map(|index| Match { index, score: 0, positions: Vec::new() })
                .collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut out: Vec<Match> = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                let path = path.as_ref();
                let mut positions = Vec::new();
                let score = pattern.indices(Utf32Str::new(path, &mut buf), &mut self.matcher, &mut positions)?;
                positions.sort_unstable();
                positions.dedup();
                // Favour matches in the file name, the more of the name they cover the better.
                let name_start = path.rfind('/').map_or(0, |i| path[..=i].chars().count()) as u32;
                let name_len = (path.chars().count() as u32 - name_start).max(1);
                let in_name = positions.iter().filter(|&&p| p >= name_start).count() as u32;
                Some(Match { index, score: score + in_name * 64 / name_len, positions })
            })
            .collect();
        out.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| candidates[a.index].as_ref().len().cmp(&candidates[b.index].as_ref().len()))
        });
        out.truncate(limit);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_file_names_first() {
        let paths =
            ["crates/pw-app/src/app.rs", "crates/pw-app/src/ui/editor/mod.rs", "docs/app_notes.md", "README.md"];
        let mut fuzzy = Fuzzy::default();
        let found = fuzzy.rank("app", &paths, 10);
        assert_eq!(found[0].index, 0);
        assert!(found.iter().all(|m| m.index != 3));
        let found = fuzzy.rank("edmod", &paths, 10);
        assert_eq!(found[0].index, 1);
        assert_eq!(found[0].positions.len(), 5);
        assert!(fuzzy.rank("zzz", &paths, 10).is_empty());
    }

    #[test]
    fn empty_query_keeps_order_up_to_limit() {
        let paths = ["b", "a", "c"];
        let found = Fuzzy::default().rank("", &paths, 2);
        assert_eq!(found.iter().map(|m| m.index).collect::<Vec<_>>(), vec![0, 1]);
    }
}
