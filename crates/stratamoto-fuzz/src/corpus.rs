//! The programs the fuzzer mutates, how it chooses among them, and where it keeps them.

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use rand::RngExt;
use stratamoto_ir::Program;

use crate::target::Behaviour;

/// One program in the corpus and what the campaign knows about it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub program: Program,
    pub behaviour: Behaviour,
    /// Whether it was kept for what it reached inside the target rather than for its
    /// behaviour.
    pub for_coverage: bool,
    /// How often it has been picked to mutate.
    pub picks: u64,
    /// How often a mutant of it earned a slot of its own.
    pub yields: u64,
}

impl Entry {
    /// How much the scheduler favours the entry.
    ///
    /// One whose mutants keep earning slots is promising, and so is one that reached new
    /// code; one picked many times for nothing is tired. A short program is favoured over a
    /// long one, since its mutations land nearer to what matters and it runs faster.
    fn weight(&self) -> u64 {
        let promise = 1 + 2 * self.yields + u64::from(self.for_coverage);
        let brevity = 8 / (1 + self.program.instructions.len() as u64 / 32);
        let fatigue = 8 + self.picks;
        (promise * brevity.max(1) * 8 / fatigue).max(1)
    }
}

/// The programs the fuzzer mutates.
///
/// A program earns a place by showing a fact of behaviour no other program has, or, when an
/// observer sees inside the target, by reaching something in there no other program did.
/// Entries are picked by weight rather than uniformly, and written to a directory as they are
/// admitted when the corpus is given one, so that a later campaign starts where this one got.
#[derive(Default)]
pub struct Corpus {
    entries: Vec<Entry>,
    /// Every fact any entry has shown.
    seen: HashSet<u64>,
    /// Where entries are written as they are admitted, and the number the next file gets.
    dir: Option<(PathBuf, usize)>,
}

impl Corpus {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Write every entry admitted from now on into `dir`, one file each. Files already there
    /// are left alone and never overwritten; [`saved_in`](Self::saved_in) reads them.
    pub fn persisted_in(mut self, dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let next = saved_files(&dir)?
            .last()
            .map_or(0, |(number, _)| number + 1);
        self.dir = Some((dir, next));
        Ok(self)
    }

    /// The programs an earlier campaign saved in `dir`, oldest first. A file that does not
    /// decode is skipped with a warning: it is not this campaign's to judge.
    pub fn saved_in(dir: &Path) -> io::Result<Vec<Program>> {
        let mut programs = Vec::new();
        for (_, path) in saved_files(dir)? {
            let bytes = fs::read(&path)?;
            match stratamoto_ir::artifact::read_program(&bytes) {
                Ok(program) => programs.push(program),
                Err(e) => log::warn!("skipping {}: {e}", path.display()),
            }
        }
        Ok(programs)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Keep the program if it showed a fact no entry has. Returns whether it was kept.
    pub fn add(&mut self, program: Program, behaviour: Behaviour) -> bool {
        if behaviour.features.iter().all(|f| self.seen.contains(f)) {
            return false;
        }
        self.admit(program, behaviour, false, true);
        true
    }

    /// Keep the program whatever it showed, because something other than its behaviour made
    /// it new.
    pub fn keep(&mut self, program: Program, behaviour: Behaviour) {
        self.admit(program, behaviour, true, true);
    }

    /// Take back an entry an earlier campaign saved, without writing it again.
    pub fn restore(&mut self, program: Program, behaviour: Behaviour) {
        self.admit(program, behaviour, false, false);
    }

    fn admit(&mut self, program: Program, behaviour: Behaviour, for_coverage: bool, save: bool) {
        self.seen.extend(behaviour.features.iter().copied());
        if save && let Some((dir, next)) = &mut self.dir {
            let path = dir.join(format!("{next:06}.program"));
            *next += 1;
            match postcard::to_allocvec(&program) {
                Ok(bytes) => {
                    if let Err(e) = fs::write(&path, bytes) {
                        log::warn!("could not save a corpus entry to {}: {e}", path.display());
                    }
                }
                Err(e) => log::warn!("could not encode a corpus entry: {e}"),
            }
        }
        self.entries.push(Entry {
            program,
            behaviour,
            for_coverage,
            picks: 0,
            yields: 0,
        });
    }

    /// Pick an entry to mutate, by weight, and count the pick. Returns its index.
    pub fn pick<R: RngExt>(&mut self, rng: &mut R) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let total: u64 = self.entries.iter().map(Entry::weight).sum();
        let mut draw = rng.random_range(0..total);
        let index = self
            .entries
            .iter()
            .position(|entry| {
                let weight = entry.weight();
                if draw < weight {
                    true
                } else {
                    draw -= weight;
                    false
                }
            })
            .unwrap_or(self.entries.len() - 1);
        self.entries[index].picks += 1;
        Some(index)
    }

    #[must_use]
    pub fn program(&self, index: usize) -> &Program {
        &self.entries[index].program
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// A mutant of the entry earned a slot of its own.
    pub fn credit(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.yields += 1;
        }
    }

    pub fn programs(&self) -> impl Iterator<Item = &Program> {
        self.entries.iter().map(|entry| &entry.program)
    }
}

/// The corpus files in `dir` with their numbers, in order.
fn saved_files(dir: &Path) -> io::Result<Vec<(usize, PathBuf)>> {
    let mut files: Vec<(usize, PathBuf)> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter_map(|path| {
                let number = path
                    .file_name()?
                    .to_str()?
                    .strip_suffix(".program")?
                    .parse()
                    .ok()?;
                Some((number, path))
            })
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    files.sort_unstable();
    Ok(files)
}
