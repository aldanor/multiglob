use std::path::{Path, PathBuf};

use log::debug;
use walkdir::WalkDir;

use crate::{cluster::cluster_globs, walk::MultiGlobWalker, GlobError};

#[derive(Clone)]
pub(crate) struct MultiGlobOptions {
    pub follow_links: bool,
    pub max_depth: usize,
    pub max_open: usize,
    pub same_file_system: bool,
    pub case_insensitive: bool,
}

impl Default for MultiGlobOptions {
    fn default() -> Self {
        Self {
            follow_links: false,
            max_depth: usize::MAX,
            max_open: 10,
            same_file_system: false,
            case_insensitive: false,
        }
    }
}

impl MultiGlobOptions {
    pub fn configure_walkdir(&self, walkdir: WalkDir) -> WalkDir {
        walkdir
            .sort_by_file_name()
            .follow_links(self.follow_links)
            .max_open(self.max_open)
            .same_file_system(self.same_file_system)
    }
}

pub struct MultiGlobBuilder {
    base: PathBuf,
    patterns: Vec<String>,
    opts: MultiGlobOptions,
}

impl MultiGlobBuilder {
    /// Construct a new multiglob walker builder from a base directory and a list of patterns.
    ///
    /// When iterated, the `base` directory will be recursively searched for paths
    /// matching `patterns`.
    pub fn new<B, P, S>(base: B, patterns: P) -> Self
    where
        B: AsRef<Path>,
        P: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            base: base.as_ref().to_owned(),
            patterns: patterns.into_iter().map(|s| s.as_ref().to_owned()).collect(),
            opts: MultiGlobOptions::default(),
        }
    }

    fn impl_build(&self, skip_invalid: bool) -> Result<MultiGlobWalker, GlobError> {
        let mut walker = MultiGlobWalker::new(self.opts.clone());
        let glob_groups = cluster_globs(&self.patterns);
        for (base, patterns) in glob_groups {
            let mut base = self.base.join(base);
            let is_root = base == self.base;
            if is_root {
                base = self.base.clone();
            }
            debug!(base:?, patterns:?, is_root; "adding a glob group");
            walker.add(base, is_root, patterns, skip_invalid)?;
        }
        Ok(walker.rev())
    }

    /// Construct a multiglob walker; error may occur when parsing globs.
    pub fn build(&self) -> Result<MultiGlobWalker, GlobError> {
        self.impl_build(false)
    }

    /// Construct a multiglob walker and skip all invalid globs patterns.
    pub fn build_skip_invalid(&self) -> MultiGlobWalker {
        self.impl_build(true).unwrap()
    }

    /// Toggle whether the globs should be matched case insensitively or not.
    ///
    /// When this option is changed, only globs added after the change will be affected.
    ///
    /// This is disabled by default.
    pub fn case_insensitive(mut self, yes: bool) -> Self {
        self.opts.case_insensitive = yes;
        self
    }

    /// Set the maximum depth of all recursive globs (those containing `**`).
    ///
    /// The smallest depth is `0` and always corresponds to the path given
    /// to the `new` function on this type. Its direct descendents have depth
    /// `1`, and their descendents have depth `2`, and so on.
    ///
    /// This will not simply filter the entries of the iterator, but
    /// it will actually avoid descending into directories when the depth is
    /// exceeded.
    ///
    /// Note that the depth is counted not from the base directory, but from a point
    /// where a recursive pattern is encountered. For example, if maximum depth is 2
    /// and patterns are `../a/**` and `b/**`, then the deepest entries will look
    // like `../a/x/y` and `b/x/y`.
    ///
    /// By default, there's no max depth limit.
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.opts.max_depth = depth;
        if self.opts.max_depth < self.opts.max_depth {
            self.opts.max_depth = self.opts.max_depth;
        }
        self
    }

    /// Follow symbolic links. By default, this is disabled.
    ///
    /// When `yes` is `true`, symbolic links are followed as if they were
    /// normal directories and files. If a symbolic link is broken or is
    /// involved in a loop, an error is yielded.
    ///
    /// When enabled, the yielded [`DirEntry`] values represent the target of
    /// the link while the path corresponds to the link. See the [`DirEntry`]
    /// type for more details.
    ///
    /// Note, this only affects parts of globs starting from the first glob-like
    /// component. For example, in a pattern `a/b/*/c/**` this will only affect
    /// the `*/c/**` part of the pattern.
    ///
    /// [`DirEntry`]: struct.DirEntry.html
    pub fn follow_links(mut self, yes: bool) -> Self {
        self.opts.follow_links = yes;
        self
    }

    /// Set the maximum number of simultaneously open file descriptors used
    /// by the iterator.
    ///
    /// `n` must be greater than or equal to `1`. If `n` is `0`, then it is set
    /// to `1` automatically. If this is not set, then it defaults to some
    /// reasonably low number.
    ///
    /// This setting has no impact on the results yielded by the iterator
    /// (even when `n` is `1`). Instead, this setting represents a trade off
    /// between scarce resources (file descriptors) and memory. Namely, when
    /// the maximum number of file descriptors is reached and a new directory
    /// needs to be opened to continue iteration, then a previous directory
    /// handle is closed and has its unyielded entries stored in memory. In
    /// practice, this is a satisfying trade off because it scales with respect
    /// to the *depth* of your file tree. Therefore, low values (even `1`) are
    /// acceptable.
    ///
    /// Note that this value does not impact the number of system calls made by
    /// an exhausted iterator.
    ///
    /// # Platform behavior
    ///
    /// On Windows, if `follow_links` is enabled, then this limit is not
    /// respected. In particular, the maximum number of file descriptors opened
    /// is proportional to the depth of the directory tree traversed.
    pub fn max_open(mut self, mut n: usize) -> Self {
        if n == 0 {
            n = 1;
        }
        self.opts.max_open = n;
        self
    }

    /// Do not cross file system boundaries.
    ///
    /// When this option is enabled, directory traversal will not descend into
    /// directories that are on a different file system from the root path.
    ///
    /// Currently, this option is only supported on Unix and Windows. If this
    /// option is used on an unsupported platform, then directory traversal
    /// will immediately return an error and will not yield any entries.
    pub fn same_file_system(mut self, yes: bool) -> Self {
        self.opts.same_file_system = yes;
        self
    }
}
