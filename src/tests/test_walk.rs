use std::path::{Path, PathBuf};

use current_dir::Cwd;
use pretty_assertions::assert_eq;

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

#[test]
fn test_glob_parent_dir() {
    let dir = Dir::tmp();
    dir.mkdirp("a/b");
    dir.mkdirp("a/c");
    let p = dir.path();

    let res = mg_collect_no_err(p, ["*"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a")]);
    let res = mg_collect_no_err(p.join("a"), ["*"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b"), p.join("a/c")]);
    let res = mg_collect_no_err(p.join("a"), ["*", "."]);
    assert_eq!(res.sorted_paths(), vec![p.join("a"), p.join("a/b"), p.join("a/c")]);
    let res = mg_collect_no_err(p.join("a"), ["*", "**"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a"), p.join("a/b"), p.join("a/c")]);
}

#[test]
fn test_case_sensitive() {
    let dir = Dir::tmp();
    dir.mkdirp("a/B");
    dir.touch("a/B/d");
    let p = dir.path();

    let res = mg_collect_custom(p, ["a/**"], |b| b);
    assert_eq!(res.sorted_paths(), vec![p.join("a"), p.join("a/B"), p.join("a/B/d")]);
    let res = mg_collect_custom(p, ["a/{b}/*"], |b| b);
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());
    let res = mg_collect_custom(p, ["a/{b}/*"], |b| b.case_insensitive(false));
    assert_eq!(res.sorted_paths(), Vec::<PathBuf>::new());
    let res = mg_collect_custom(p, ["a/{b}/*"], |b| b.case_insensitive(true));
    assert_eq!(res.sorted_paths(), vec![p.join("a/B/d")]);
}

#[test]
fn test_symlink_file() {
    let dir = Dir::tmp();
    dir.mkdirp("a");
    dir.touch("a/b");
    dir.symlink_file("a/b", "a/c");
    let p = dir.path();

    let res = mg_collect_no_err(p, ["a/*"]);
    assert_eq!(res.sorted_paths(), vec![p.join("a/b"), p.join("a/c")]);
    assert!(!res.sorted_ents()[0].path_is_symlink());
    assert!(res.sorted_ents()[0].file_type().is_file());
    assert!(res.sorted_ents()[1].path_is_symlink());
    assert!(!res.sorted_ents()[1].file_type().is_file());

    let res = mg_collect_custom(p, ["a/*"], |b| b.follow_links(true));
    assert_eq!(res.sorted_paths(), vec![p.join("a/b"), p.join("a/c")]);
    assert!(!res.sorted_ents()[0].path_is_symlink());
    assert!(res.sorted_ents()[0].file_type().is_file());
    assert!(res.sorted_ents()[1].path_is_symlink());
    assert!(res.sorted_ents()[1].file_type().is_file());
}

#[test]
fn test_invalid_glob() {
    let dir = Dir::tmp();
    dir.mkdirp("a/x");
    dir.touch("a/x/b");
    let p = dir.path();

    let b = MultiGlobBuilder::new(p.join("a"), ["x/*", "y/{", "z/["]);

    let err = b.build().err().unwrap();
    assert_eq!(err.glob().unwrap(), "y/{");

    let (walker, errors) = b.build_skip_invalid();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].glob().unwrap(), "y/{");
    assert_eq!(errors[1].glob().unwrap(), "z/[");
    assert_eq!(
        walker.map(|e| e.unwrap().path().to_owned()).collect::<Vec<_>>(),
        vec![p.join("a/x/b")]
    );
}

#[test]
fn test_bigger_walk() {
    let dir = Dir::tmp();
    dir.mkdirp("a/b/c");
    dir.touch("a/b/c/x.doc");
    dir.touch("a/b/c/y.doc");
    dir.touch("a/b/c/z1.txt");
    dir.touch("a/b/c/z2.txt");
    dir.touch("a/b/c/z3.txt");
    dir.mkdirp("x/y/b/c/d");
    dir.touch("x/y/b/c/d/a1.doc");
    dir.touch("x/y/b/c/d/a2.doc");
    dir.touch("x/y/b/c/d/a3.doc");
    let p = dir.path();

    let res = mg_collect_no_err(
        p.join("a"),
        [
            ".",
            "..",
            "../x/**/*[13].d*",
            "../**/*/c",
            "b/*",
            &p.join("a/b/*/z{1,3}.*").display().to_string(),
            "b/c/../*/y*",
        ],
    );
    assert_eq!(
        res.sorted_paths(),
        vec![
            p.join("a"),
            p.join("a/.."),
            p.join("a/../a/b/c"),
            p.join("a/../x/y/b/c"),
            p.join("a/../x/y/b/c/d/a1.doc"),
            p.join("a/../x/y/b/c/d/a3.doc"),
            p.join("a/b/c"), // NOTE: it's the same path as before but we can't tell that unless we query metadata
            p.join("a/b/c/../c/y.doc"),
            p.join("a/b/c/z1.txt"),
            p.join("a/b/c/z3.txt"),
        ]
    );
}

#[test]
fn test_canonicalized() {
    let dir = Dir::tmp();
    dir.mkdirp("a/b/c");
    dir.touch("a/b/c/d");
    let p = dir.path();

    let patterns = [
        &p.join("a/b").display().to_string(),
        &p.join("a/b/c").display().to_string(),
        "a/*/c",
        "a/*",
        "../a/b",
        "../a/b/c",
        "../*/b",
        "a/*/../b/c",
    ];
    let res = mg_collect_no_err(p.join("a"), patterns);
    assert_eq!(
        res.sorted_paths(),
        vec![p.join("a/../a/b"), p.join("a/../a/b/c"), p.join("a/b"), p.join("a/b/c"),]
    );
    let res = mg_collect_custom(p.join("a"), patterns, |b| b.canonicalize());
    assert_eq!(res.sorted_paths(), vec![p.join("a/b"), p.join("a/b/c")]);
}
