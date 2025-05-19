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
}
