use std::io::{BufRead, BufReader, BufWriter, Write};
use std::ops::Index;
use std::path::Path;

use rustyline::config::Config;
use rustyline::history::{History, MemHistory, SearchDirection, SearchResult};
use rustyline::Result as RustylineResult;

pub struct PortableHistory {
    mem: MemHistory,
}

impl PortableHistory {
    pub fn new() -> Self {
        Self {
            mem: MemHistory::new(),
        }
    }

    pub fn with_config(config: &Config) -> Self {
        Self {
            mem: MemHistory::with_config(config),
        }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &String> + '_ {
        (&self.mem).into_iter()
    }
}

impl Default for PortableHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl History for PortableHistory {
    fn get(&self, index: usize, dir: SearchDirection) -> RustylineResult<Option<SearchResult<'_>>> {
        self.mem.get(index, dir)
    }

    fn add(&mut self, line: &str) -> RustylineResult<bool> {
        self.mem.add(line)
    }

    fn add_owned(&mut self, line: String) -> RustylineResult<bool> {
        self.mem.add_owned(line)
    }

    fn len(&self) -> usize {
        self.mem.len()
    }

    fn is_empty(&self) -> bool {
        self.mem.is_empty()
    }

    fn set_max_len(&mut self, len: usize) -> RustylineResult<()> {
        self.mem.set_max_len(len)
    }

    fn ignore_dups(&mut self, yes: bool) -> RustylineResult<()> {
        self.mem.ignore_dups(yes)
    }

    fn ignore_space(&mut self, yes: bool) {
        self.mem.ignore_space(yes);
    }

    fn save(&mut self, path: &Path) -> RustylineResult<()> {
        let file = std::fs::File::create(path)?;
        let mut writer = BufWriter::new(file);
        for entry in &self.mem {
            let json = serde_json::to_string(entry).map_err(std::io::Error::other)?;
            writeln!(writer, "{json}")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn append(&mut self, path: &Path) -> RustylineResult<()> {
        self.save(path)
    }

    fn load(&mut self, path: &Path) -> RustylineResult<()> {
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let entry: String = serde_json::from_str(trimmed).map_err(std::io::Error::other)?;
                self.add_owned(entry)?;
            }
        }
        Ok(())
    }

    fn clear(&mut self) -> RustylineResult<()> {
        self.mem.clear()
    }

    fn search(
        &self,
        term: &str,
        start: usize,
        dir: SearchDirection,
    ) -> RustylineResult<Option<SearchResult<'_>>> {
        self.mem.search(term, start, dir)
    }

    fn starts_with(
        &self,
        term: &str,
        start: usize,
        dir: SearchDirection,
    ) -> RustylineResult<Option<SearchResult<'_>>> {
        self.mem.starts_with(term, start, dir)
    }
}

impl Index<usize> for PortableHistory {
    type Output = String;

    fn index(&self, index: usize) -> &String {
        &self.mem[index]
    }
}

impl<'a> IntoIterator for &'a PortableHistory {
    type IntoIter = <&'a MemHistory as IntoIterator>::IntoIter;
    type Item = <&'a MemHistory as IntoIterator>::Item;

    fn into_iter(self) -> Self::IntoIter {
        (&self.mem).into_iter()
    }
}
