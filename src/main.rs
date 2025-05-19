use std::env;

use multiglob::MultiGlobBuilder;

fn main() {
    env_logger::builder().filter_module("multiglob", log::LevelFilter::Trace).init();
    let mut args = env::args().skip(1);
    let base = args.next().unwrap();
    println!("base: {base:?}");
    let patterns = args.collect::<Vec<_>>();
    println!("patterns: {patterns:?}");
    for entry in MultiGlobBuilder::new(base, patterns).build().unwrap() {
        let entry = entry.unwrap();
        println!("{} {:?}", entry.path().to_string_lossy(), entry.file_type());
        println!("{:?}", entry.path().components().collect::<Vec<_>>());
    }

    let walk = walkdir::WalkDir::new("tmp/asym").follow_links(false).follow_root_links(false);
    for entry in walk {
        let entry = entry.unwrap();
        let ty = entry.file_type();
        println!(
            "{} is_dir={} is_sym={} path_is_symlink={}",
            entry.path().display(),
            ty.is_dir(),
            ty.is_symlink(),
            entry.path_is_symlink()
        );
    }
}
