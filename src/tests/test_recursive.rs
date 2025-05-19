use rstest::rstest;
use walkdir::WalkDir;

use crate::{DirEntry, MultiGlobBuilder, MultiGlobWalker};

use super::util::{Dir, RecursiveResults, Result};

#[ctor::ctor]
fn init() {
    env_logger::builder().filter_module("multiglob", log::LevelFilter::Trace).init();
}

#[track_caller]
fn assert_ent_eq(a: &DirEntry, b: &DirEntry) {
    let cmp = |d: &DirEntry| {
        let md = d.metadata().ok();
        (
            d.path().to_path_buf(),
            d.file_type(),
            d.path_is_symlink(),
            md.as_ref().map(|md| md.file_type()),
            md.as_ref().and_then(|md| md.modified().ok()),
        )
    };
    assert_eq!(cmp(a), cmp(b));
}

#[track_caller]
fn assert_mg_eq_wd(mg: MultiGlobWalker, wd: WalkDir) {
    let ents_mg = RecursiveResults::collect(mg);
    let ents_wd = RecursiveResults::collect(wd);
    ents_mg.assert_no_errors("mg");
    ents_wd.assert_no_errors("wd");
    assert_eq!(ents_mg.sorted_paths(), ents_wd.sorted_paths());
    for (mg, wd) in ents_mg.sorted_ents().into_iter().zip(ents_wd.sorted_ents()) {
        assert_ent_eq(&mg, &wd);
    }
}

#[rstest]
fn test_double_star_at_root(
    #[values("base", "base/asym")] base: &str,
    #[values(false, true)] follow_links: bool,
) -> Result<()> {
    let dir = Dir::tmp();
    dir.mkdirp("base");
    dir.mkdirp("a/b");
    dir.symlink_dir("a", "base/asym");
    dir.symlink_dir("a/b", "a/bsym");
    dir.touch("a/b/c");

    let base = dir.path().join(base);
    assert_mg_eq_wd(
        MultiGlobBuilder::new(&base, ["**"]).follow_links(follow_links).build().unwrap(),
        WalkDir::new(&base).follow_links(follow_links),
    );

    Ok(())
}

#[rstest]
fn test_double_star_at_path(
    #[values("x/y", "x/y/asym", "x/y/asym/b")] path: &str,
    #[values(false, true)] follow_links: bool,
) -> Result<()> {
    let dir = Dir::tmp();
    dir.mkdirp("base/x/y");
    dir.mkdirp("a/b");
    dir.symlink_dir("a", "base/x/y/asym");
    dir.symlink_dir("a/b", "a/bsym");
    dir.touch("a/b/c");

    let base = dir.path().join("base");
    assert_mg_eq_wd(
        MultiGlobBuilder::new(&base, [format!("{path}/**")])
            .follow_links(follow_links)
            .build()
            .unwrap(),
        WalkDir::new(&base.join(path)).follow_links(follow_links).follow_root_links(false),
    );

    Ok(())
}
