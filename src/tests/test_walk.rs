use std::{
    fmt::format,
    path::{Path, PathBuf},
};

use current_dir::Cwd;
use log::debug;
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
    dir.touch(format!("{base}/d.1"));
    dir.touch(format!("{base}/d.2"));
    dir.touch(format!("{base}/d.3"));
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
fn test_walk_path() {
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

#[test]
fn test_walk_glob() {
    let dir = setup_dir_with_syms();
    let p = dir.path();

    let res = mg_collect(p.join("base/x"), &["a*"]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x/asym")]);

    let res = mg_collect(p.join("base/x"), &["d.{1,2}", "asym"]);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("base/x/asym"), p.join("base/x/d.1"), p.join("base/x/d.2")]
    );

    let res = mg_collect(p.join("base/x"), &["d.[12]", "asym"]);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("base/x/asym"), p.join("base/x/d.1"), p.join("base/x/d.2")]
    );

    let res = mg_collect(p.join("base/x"), &["d.{1,2}", "."]);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("base/x"), p.join("base/x/d.1"), p.join("base/x/d.2")]
    );

    let res = mg_collect("nope", &["**"]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());
}

#[test]
fn test_walk_rel() {
    let dir = setup_dir_with_syms();
    let p = dir.path();

    let mut cwd = Cwd::mutex().lock().unwrap();
    cwd.set(p.join("base/x")).unwrap();

    let res = mg_collect("", &[] as &[&str]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());

    for b in ["", "."] {
        for p in ["", "."] {
            debug!("b={b:?} p={p:?}");
            let res = mg_collect(b, &[p]);
            assert_eq!(res.sorted_paths(), vec![PathBuf::from(".")]);
        }
    }

    let res = mg_collect(".", &["d.1"]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("./d.1")]);

    let res = mg_collect(".", &["d.[12]", ""]);
    assert_eq!(
        res.sorted_paths(),
        vec![PathBuf::from("."), PathBuf::from("./d.1"), PathBuf::from("./d.2")]
    );

    let res = mg_collect("..", &[""]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("..")]);

    let res = mg_collect(".", &[".."]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("./..")]);

    let res = mg_collect("", &[".."]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("./..")]);

    let res = mg_collect("nope", &["*"]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());
}
