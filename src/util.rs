use std::{io, path::Path};

/// Check if a component of a path looks like it may be a glob pattern.
///
/// Note: this function is being used when splitting a glob pattern into a long possible
/// base and the glob remainder (scanning through components until we hit the first component
/// for which this function returns true). It is acceptable for this function to return
/// false positives (e.g. patterns like 'foo[bar' or 'foo{bar') in which case correctness
/// will not be affected but efficiency might be (because we'll traverse more than we should),
/// however it should not return false negatives.
pub fn is_glob_like(part: &str) -> bool {
    ["*", "{", "}", "?", "[", "]"].into_iter().any(|c| part.contains(c))
}

#[cfg(unix)]
pub fn device_num<P: AsRef<Path>>(path: P) -> io::Result<u64> {
    // borrowed from walkdir crate
    use std::os::unix::fs::MetadataExt;

    path.as_ref().metadata().map(|md| md.dev())
}

#[cfg(windows)]
pub fn device_num<P: AsRef<Path>>(path: P) -> io::Result<u64> {
    // borrowed from walkdir crate
    use winapi_util::{file, Handle};

    let h = Handle::from_path_any(path)?;
    file::information(h).map(|info| info.volume_serial_number())
}

#[cfg(not(any(unix, windows)))]
pub fn device_num<P: AsRef<Path>>(_: P) -> io::Result<u64> {
    // borrowed from walkdir crate
    Err(io::Error::new(
        io::ErrorKind::Other,
        "walkdir: same_file_system option not supported on this platform",
    ))
}
