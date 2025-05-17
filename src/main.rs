use std::env;

use multiglob::builder::MultiGlobBuilder;

fn main() {
    let mut args = env::args().skip(1);
    let base = args.next().unwrap();
    println!("base: {base:?}");
    let patterns = args.collect::<Vec<_>>();
    println!("patterns: {patterns:?}");
    for entry in MultiGlobBuilder::new(base, patterns).build().unwrap() {
        let entry = entry.unwrap();
        let suffix = if entry.is_dir() { "/" } else { "" };
        println!("{}{suffix}", entry.to_string_lossy());
    }
}
