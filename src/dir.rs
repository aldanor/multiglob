use std::{
    ffi::OsStr,
    fmt,
    fs::{self},
    io,
    path::{Path, PathBuf},
};

// note/credits: most of DirEntryPath-related code is borrowed from walkdir with minor adjustments

/// A directory entry.
///
/// This is the type of value that is yielded from ['MultiGlobWalker'] iterator.
/// this crate.
///
/// ### Differences with `std::fs::DirEntry`
///
/// This type mostly mirrors the type by the same name in [`std::fs`]. There
/// are some differences however:
///
/// * All recursive directory iterators must inspect the entry's type.
///   Therefore, the value is stored and its access is guaranteed to be cheap and
///   successful.
/// * [`path`] and [`file_name`] return borrowed variants.
/// * If [`follow_links`] was enabled in the builder, then all
///   operations except for [`path`] operate on the link target. Otherwise, all
///   operations operate on the symbolic link.
///
/// [`MultiGlobWalker`]: struct.MultiGlobWalker.html
/// [`std::fs`]: https://doc.rust-lang.org/stable/std/fs/index.html
/// [`path`]: #method.path
/// [`file_name`]: #method.file_name
/// [`follow_links`]: struct.WalkDir.html#method.follow_links
/// [`DirEntryExt`]: trait.DirEntryExt.html
#[derive(Clone)]
pub struct DirEntry(DirEntryInner);

#[derive(Clone)]
enum DirEntryInner {
    /// The entry was created from following a direct path link.
    Path(DirEntryPath),
    /// The entry was created by walking over a glob.
    Walk(walkdir::DirEntry),
}

#[derive(Clone)]
struct DirEntryPath {
    /// The path as reported by the [`fs::ReadDir`] iterator (even if it's a
    /// symbolic link).
    ///
    /// [`fs::ReadDir`]: https://doc.rust-lang.org/stable/std/fs/struct.ReadDir.html
    path: PathBuf,
    /// The file type.
    ty: fs::FileType,
    /// Is set when this entry was created from a symbolic link and the user
    /// expects the iterator to follow symbolic links.
    follow_link: bool,
    /// The underlying metadata (Windows only). We store this on Windows
    /// because this comes for free while reading a directory.
    #[cfg(windows)]
    metadata: fs::Metadata,
}

fn error_with_path(err: io::Error, path: &Path) -> io::Error {
    io::Error::new(err.kind(), format!("{err} (path: {})", path.display()))
}

impl DirEntryPath {
    pub fn from_meta(path: PathBuf, metadata: fs::Metadata, follow: bool) -> Self {
        Self {
            path,
            ty: metadata.file_type(),
            follow_link: follow,
            #[cfg(windows)]
            metadata,
        }
    }

    #[cfg(windows)]
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        if self.follow_link { fs::metadata(&self.path) } else { Ok(self.metadata.clone()) }
            .map_err(|err| error_with_path(err, &self.path))
    }

    #[cfg(not(windows))]
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        if self.follow_link { fs::metadata(&self.path) } else { fs::symlink_metadata(&self.path) }
            .map_err(|err| error_with_path(err, &self.path))
    }
}

impl DirEntry {
    /// The full path that this entry represents.
    ///
    /// The full path is created by joining the parents of this entry up to the
    /// root initially given to [`MultiGlobBuilder::new`] with the file name of this
    /// entry.
    ///
    /// Note that this *always* returns the path reported by the underlying
    /// directory entry, even when symbolic links are followed. To get the
    /// target path, use [`path_is_symlink`] to (cheaply) check if this entry
    /// corresponds to a symbolic link, and [`std::fs::read_link`] to resolve
    /// the target.
    ///
    /// [`path_is_symlink`]: struct.DirEntry.html#method.path_is_symlink
    /// [`MultiGlobBuilder::new`]: struct.MultiGlobBuilder.html#method.new
    /// [`std::fs::read_link`]: https://doc.rust-lang.org/stable/std/fs/fn.read_link.html
    pub fn path(&self) -> &Path {
        match &self.0 {
            DirEntryInner::Path(e) => &e.path,
            DirEntryInner::Walk(e) => e.path(),
        }
    }

    /// The full path that this entry represents.
    ///
    /// Analogous to [`path`], but moves ownership of the path.
    ///
    /// [`path`]: struct.DirEntry.html#method.path
    pub fn into_path(self) -> PathBuf {
        match self.0 {
            DirEntryInner::Path(e) => e.path,
            DirEntryInner::Walk(e) => e.into_path(),
        }
    }

    /// Returns `true` if and only if this entry was created from a symbolic
    /// link. This is unaffected by the [`follow_links`] setting.
    ///
    /// When `true`, the value returned by the [`path`] method is a
    /// symbolic link name. To get the full target path, you must call
    /// [`std::fs::read_link(entry.path())`].
    ///
    /// [`path`]: struct.DirEntry.html#method.path
    /// [`follow_links`]: struct.MultiGlobBuilder.html#method.follow_links
    /// [`std::fs::read_link(entry.path())`]: https://doc.rust-lang.org/stable/std/fs/fn.read_link.html
    pub fn path_is_symlink(&self) -> bool {
        match &self.0 {
            DirEntryInner::Path(e) => e.ty.is_symlink() || e.follow_link,
            DirEntryInner::Walk(e) => e.path_is_symlink(),
        }
    }

    /// Return the metadata for the file that this entry points to.
    ///
    /// This will follow symbolic links if and only if the [`MultiGlobBuilder`] value
    /// has [`follow_links`] enabled.
    ///
    /// # Platform behavior
    ///
    /// This always calls [`std::fs::symlink_metadata`].
    ///
    /// If this entry is a symbolic link and [`follow_links`] is enabled, then
    /// [`std::fs::metadata`] is called instead.
    ///
    /// # Errors
    ///
    /// Similar to [`std::fs::metadata`], returns errors for path values that
    /// the program does not have permissions to access or if the path does not
    /// exist.
    ///
    /// [`MultiGlobBuilder`]: struct.MultiGlobBuilder.html
    /// [`follow_links`]: struct.MultiGlobBuilder.html#method.follow_links
    /// [`std::fs::metadata`]: https://doc.rust-lang.org/std/fs/fn.metadata.html
    /// [`std::fs::symlink_metadata`]: https://doc.rust-lang.org/stable/std/fs/fn.symlink_metadata.html
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        Ok(match &self.0 {
            DirEntryInner::Path(e) => e.metadata()?,
            DirEntryInner::Walk(e) => e.metadata()?,
        })
    }

    /// Return the file type for the file that this entry points to.
    ///
    /// If this is a symbolic link and [`follow_links`] is `true`, then this
    /// returns the type of the target.
    ///
    /// This never makes any system calls.
    ///
    /// [`follow_links`]: struct.MultiGlobBuilder.html#method.follow_links
    pub fn file_type(&self) -> fs::FileType {
        match &self.0 {
            DirEntryInner::Path(e) => e.ty,
            DirEntryInner::Walk(e) => e.file_type(),
        }
    }

    /// Return the file name of this entry.
    ///
    /// If this entry has no file name (e.g., `/`), then the full path is
    /// returned.
    pub fn file_name(&self) -> &OsStr {
        let path = self.path();
        path.file_name().unwrap_or(path.as_os_str())
    }

    pub(crate) fn from_meta(path: PathBuf, metadata: fs::Metadata, follow: bool) -> Self {
        Self(DirEntryInner::Path(DirEntryPath::from_meta(path, metadata, follow)))
    }

    pub(crate) fn from_walk(entry: walkdir::DirEntry) -> Self {
        Self(DirEntryInner::Walk(entry))
    }
}

impl From<walkdir::DirEntry> for DirEntry {
    fn from(entry: walkdir::DirEntry) -> Self {
        Self(DirEntryInner::Walk(entry))
    }
}

impl fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DirEntry({:?})", self.path())
    }
}
