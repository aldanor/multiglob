use std::path::{Path, PathBuf};

use current_dir::Cwd;
use log::debug;

use crate::MultiGlobBuilder;

use super::util::{Dir, RecursiveResults};

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

fn mg_collect_no_err<B, P, S>(base: B, patterns: P) -> RecursiveResults
where
    B: AsRef<Path>,
    P: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    mg_collect_custom(base, patterns, |x| x)
}

fn mg_collect_custom<B, P, S>(
    base: B,
    patterns: P,
    build: impl Fn(MultiGlobBuilder) -> MultiGlobBuilder,
) -> RecursiveResults
where
    B: AsRef<Path>,
    P: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    RecursiveResults::collect(build(MultiGlobBuilder::new(base, patterns)).build().unwrap())
}

#[test]
fn test_walk_missing() {
    let dir = setup_dir_with_syms();
    let p = dir.path();

    let res = mg_collect_no_err(p.join("base/x"), ["", "asym", "wrong"]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x"), p.join("base/x/asym")]);

    let res = mg_collect_no_err(p.join("base/xyz"), ["", "asym", "wrong"]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());

    let res = mg_collect_no_err("nope", ["**"]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());

    let res = mg_collect_no_err("nope", ["*"]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());

    let res = mg_collect_no_err("../nope", ["*"]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());
}

#[test]
fn test_walk_path() {
    let dir = setup_dir_with_syms();
    let p = dir.path();

    let res = mg_collect_no_err(p.join("base/x"), &[] as &[&str]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());

    let res = mg_collect_no_err(p.join("a"), ["b"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b")]);

    let res = mg_collect_no_err(p.join("a"), ["b/c"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b/c")]);

    let res = mg_collect_no_err(p.join("a"), ["b", "b/c"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b"), p.join("a/b/c")]);

    let res = mg_collect_no_err(p.join("base/x"), ["."]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x")]);

    let res = mg_collect_no_err(p.join("base/x"), [""]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x")]);
}

#[test]
fn test_walk_glob() {
    let dir = setup_dir_with_syms();
    let p = dir.path();

    let res = mg_collect_no_err(p.join("base/x"), ["a*"]);
    assert_eq!(res.sorted_paths(), vec![p.join("base/x/asym")]);

    let res = mg_collect_no_err(p.join("base/x"), ["d.{1,2}", "asym"]);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("base/x/asym"), p.join("base/x/d.1"), p.join("base/x/d.2")]
    );

    let res = mg_collect_no_err(p.join("base/x"), ["d.[12]", "asym"]);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("base/x/asym"), p.join("base/x/d.1"), p.join("base/x/d.2")]
    );

    let res = mg_collect_no_err(p.join("base/x"), ["d.{1,2}", "."]);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("base/x"), p.join("base/x/d.1"), p.join("base/x/d.2")]
    );
}

#[test]
fn test_walk_rel() {
    let dir = setup_dir_with_syms();
    let p = dir.path();

    let mut cwd = Cwd::mutex().lock().unwrap();
    cwd.set(p.join("base/x")).unwrap();

    let res = mg_collect_no_err("", &[] as &[&str]);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());

    for b in ["", "."] {
        for p in ["", "."] {
            debug!("b={b:?} p={p:?}");
            let res = mg_collect_no_err(b, [p]);
            assert_eq!(res.sorted_paths(), vec![PathBuf::from(".")]);
        }
    }

    let res = mg_collect_no_err(".", ["d.1"]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("./d.1")]);

    let res = mg_collect_no_err(".", ["d.[12]", ""]);
    assert_eq!(
        res.sorted_paths(),
        vec![PathBuf::from("."), PathBuf::from("./d.1"), PathBuf::from("./d.2")]
    );

    let res = mg_collect_no_err("..", [""]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("..")]);

    let res = mg_collect_no_err(".", [".."]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("./..")]);

    let res = mg_collect_no_err("", [".."]);
    assert_eq!(res.sorted_paths(), vec![PathBuf::from("./..")]);
}

#[test]
fn test_walk_loop() {
    let dir = Dir::tmp();
    dir.mkdirp("x/base/a/b");
    dir.symlink_dir("x", "x/base/a/b/c");
    let p = dir.path();

    let res = mg_collect_no_err(p.join("x/base"), ["**"]);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("x/base"), p.join("x/base/a"), p.join("x/base/a/b"), p.join("x/base/a/b/c")]
    );

    let res = mg_collect_custom(p.join("x/base"), ["**"], |b| b.follow_links(true));
    assert_eq!(res.errs().len(), 1);
    // walkdir behaviour
    assert_eq!(
        res.sorted_paths(),
        vec![
            p.join("x/base"),
            p.join("x/base/a"),
            p.join("x/base/a/b"),
            p.join("x/base/a/b/c"),
            p.join("x/base/a/b/c/base"),
            p.join("x/base/a/b/c/base/a"),
            p.join("x/base/a/b/c/base/a/b")
        ]
    );
}
