use std::error;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::result;

use crate::DirEntry;

type Error = std::io::Error;

// note/credits: some of this is borrowed from walkdir crate's tests

/// Create an error from a format!-like syntax.
#[macro_export]
macro_rules! err {
    ($($tt:tt)*) => {
        Box::<dyn error::Error + Send + Sync>::from(format!($($tt)*))
    }
}

pub fn windowsify(path: impl AsRef<str>) -> String {
    let mut path = path.as_ref().to_owned();
    if cfg!(windows) {
        if path.starts_with("/") {
            path = path.replacen('/', "\\\\", 1);
        }
        path = path.replace('/', "\\");
    }
    path
}

pub fn dewindowsify(path: impl AsRef<str>) -> String {
    let mut path = path.as_ref().to_owned();
    if cfg!(windows) {
        if path.starts_with("\\\\") {
            path = path.replacen("\\\\", "/", 1);
        }
        path = path.replace('\\', "/");
    }
    path
}

/// A convenient result type alias.
pub type Result<T> = result::Result<T, Box<dyn error::Error + Send + Sync>>;

/// The result of running a recursive directory iterator on a single directory.
#[derive(Debug)]
pub struct RecursiveResults {
    ents: Vec<DirEntry>,
    errs: Vec<Error>,
}

impl RecursiveResults {
    /// Run the given iterator and return the result as a distinct collection
    /// of directory entries and errors.
    pub fn collect<I, D, E>(it: I) -> Self
    where
        I: IntoIterator<Item = result::Result<D, E>>,
        D: Into<DirEntry>,
        E: Into<Error>,
    {
        let mut results = Self { ents: vec![], errs: vec![] };
        for result in it {
            match result {
                Ok(ent) => results.ents.push(ent.into()),
                Err(err) => results.errs.push(err.into()),
            }
        }
        results
    }

    /// Return all of the errors encountered during traversal.
    pub fn errs(&self) -> &[Error] {
        &self.errs
    }

    /// Assert that no errors have occurred.
    #[track_caller]
    pub fn assert_no_errors(&self) {
        assert!(self.errs.is_empty(), "expected to find no errors, but found: {:?}", self.errs);
    }

    /// Return all the successfully retrieved directory entries in the order
    /// in which they were retrieved.
    pub fn ents(&self) -> &[DirEntry] {
        &self.ents
    }

    /// Return all paths from all successfully retrieved directory entries.
    ///
    /// This does not include paths that correspond to an error.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.ents.iter().map(|d| d.path().to_path_buf()).collect()
    }

    /// Return all the successfully retrieved directory entries, sorted
    /// lexicographically by their full file path.
    pub fn sorted_ents(&self) -> Vec<DirEntry> {
        let mut ents = self.ents.clone();
        ents.sort_by(|e1, e2| e1.path().cmp(e2.path()));
        ents
    }

    /// Return all paths from all successfully retrieved directory entries,
    /// sorted lexicographically.
    ///
    /// This does not include paths that correspond to an error.
    pub fn sorted_paths(&self) -> Vec<PathBuf> {
        self.sorted_ents().into_iter().map(|d| d.into_path()).collect()
    }
}

/// A helper for managing a directory in which to run tests.
///
/// When manipulating paths within this directory, paths are interpreted
/// relative to this directory.
#[derive(Debug)]
pub struct Dir {
    dir: tempfile::TempDir,
}

impl Dir {
    /// Create a new empty temporary directory.
    pub fn tmp() -> Dir {
        let dir = tempfile::TempDir::new().unwrap();
        Dir { dir }
    }

    /// Return the path to this directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Return a path joined to the path to this directory.
    pub fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.path().join(path)
    }

    /// Create a directory at the given path, while creating all intermediate
    /// directories as needed.
    pub fn mkdirp<P: AsRef<Path>>(&self, path: P) {
        let full = self.join(path);
        fs::create_dir_all(&full)
            .map_err(|e| err!("failed to create directory {}: {}", full.display(), e))
            .unwrap();
    }

    /// Create an empty file at the given path. All ancestor directories must
    /// already exists.
    pub fn touch<P: AsRef<Path>>(&self, path: P) {
        let full = self.join(path);
        File::create(&full)
            .map_err(|e| err!("failed to create file {}: {}", full.display(), e))
            .unwrap();
    }

    /// Create empty files at the given paths. All ancestor directories must
    /// already exists.
    pub fn touch_all<P: AsRef<Path>>(&self, paths: &[P]) {
        for p in paths {
            self.touch(p);
        }
    }

    /// Create a file symlink to the given src with the given link name.
    pub fn symlink_file<P1: AsRef<Path>, P2: AsRef<Path>>(&self, src: P1, link_name: P2) {
        #[cfg(windows)]
        fn imp(src: &Path, link_name: &Path) -> io::Result<()> {
            use std::os::windows::fs::symlink_file;
            symlink_file(src, link_name)
        }

        #[cfg(unix)]
        fn imp(src: &Path, link_name: &Path) -> io::Result<()> {
            use std::os::unix::fs::symlink;
            symlink(src, link_name)
        }

        let (src, link_name) = (self.join(src), self.join(link_name));
        imp(&src, &link_name)
            .map_err(|e| {
                err!(
                    "failed to symlink file {} with target {}: {}",
                    src.display(),
                    link_name.display(),
                    e
                )
            })
            .unwrap()
    }

    /// Create a directory symlink to the given src with the given link name.
    pub fn symlink_dir<P1: AsRef<Path>, P2: AsRef<Path>>(&self, src: P1, link_name: P2) {
        #[cfg(windows)]
        fn imp(src: &Path, link_name: &Path) -> io::Result<()> {
            use std::os::windows::fs::symlink_dir;
            symlink_dir(src, link_name)
        }

        #[cfg(unix)]
        fn imp(src: &Path, link_name: &Path) -> io::Result<()> {
            use std::os::unix::fs::symlink;
            symlink(src, link_name)
        }

        let (src, link_name) = (self.join(src), self.join(link_name));
        imp(&src, &link_name)
            .map_err(|e| {
                err!(
                    "failed to symlink directory {} with target {}: {}",
                    src.display(),
                    link_name.display(),
                    e
                )
            })
            .unwrap()
    }
}
