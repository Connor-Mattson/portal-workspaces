//! Quick open: a fuzzy finder over the project's files.

use std::sync::Arc;

use pw_code::fuzzy::{Fuzzy, Match};

/// Results shown at once.
pub const LIMIT: usize = 60;

#[derive(Default)]
pub struct QuickOpen {
    pub query: String,
    /// Files, relative to the root, as `/`-separated strings. `None` until the first index arrives.
    pub files: Option<Arc<Vec<String>>>,
    pub matches: Vec<Match>,
    pub selected: usize,
    fuzzy: Fuzzy,
}

impl QuickOpen {
    pub fn new(files: Option<Arc<Vec<String>>>) -> Self {
        let mut quick = Self { files, ..Self::default() };
        quick.rank();
        quick
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.selected = 0;
        self.rank();
    }

    pub fn set_files(&mut self, files: Arc<Vec<String>>) {
        self.files = Some(files);
        self.rank();
    }

    pub fn step(&mut self, delta: isize) {
        let n = self.matches.len();
        if n > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
        }
    }

    /// The path of the `index`th result.
    pub fn path(&self, index: usize) -> Option<&str> {
        let m = self.matches.get(index)?;
        self.files.as_ref()?.get(m.index).map(String::as_str)
    }

    fn rank(&mut self) {
        self.matches = match &self.files {
            Some(files) => self.fuzzy.rank(&self.query, files, LIMIT),
            None => Vec::new(),
        };
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_steps_through_results() {
        let files = Arc::new(vec!["src/main.rs".to_owned(), "src/lib.rs".to_owned(), "README.md".to_owned()]);
        let mut q = QuickOpen::new(None);
        assert!(q.matches.is_empty());
        q.set_files(files);
        assert_eq!(q.matches.len(), 3);
        q.set_query("lib".into());
        assert_eq!(q.path(0), Some("src/lib.rs"));
        q.set_query("rs".into());
        q.step(-1);
        assert_eq!(q.selected, q.matches.len() - 1);
        q.step(1);
        assert_eq!(q.selected, 0);
    }
}
