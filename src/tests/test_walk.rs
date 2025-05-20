use std::path::{Path, PathBuf};

use current_dir::Cwd;
use rstest::rstest;
use walkdir::WalkDir;

use crate::{DirEntry, MultiGlobBuilder, MultiGlobWalker};

use super::util::{Dir, RecursiveResults, Result};

fn setup_dir_with_syms() -> Dir {
    let dir = Dir::tmp();
    let base = "base/x";
    dir.mkdirp(base);
    dir.mkdirp("a/b");
    dir.symlink_dir("a", format!("{base}/asym"));
    dir.symlink_dir("a/b", "a/bsym");
    dir.touch("a/b/c");
    dir
}

fn mg_collect<B, P, S>(base: B, patterns: P) -> RecursiveResults
where
    B: AsRef<Path>,
    P: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let res = RecursiveResults::collect(MultiGlobBuilder::new(base, patterns).build().unwrap());
    res.assert_no_errors();
    res
}

#[test]
fn test_simple_cases() {
    let dir = setup_dir_with_syms();
    let p = dir.path();

    let res = mg_collect(p.join("base/x"), &[] as &[&str]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());

    let res = mg_collect(p.join("a"), &["b"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b")]);

    let res = mg_collect(p.join("a"), &["b/c"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b/c")]);

    let res = mg_collect(p.join("a"), &["b", "b/c"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b"), p.join("a/b/c")]);

    let res = mg_collect(p.join("base/x"), &["."]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x")]);

    let res = mg_collect(p.join("base/x"), &[""]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x")]);

    let res = mg_collect(p.join("base/x"), &["", "asym", "wrong"]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x"), p.join("base/x/asym")]);
}
