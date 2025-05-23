mod builder;
mod cluster;
mod dir;
mod util;
mod walk;

#[cfg(test)]
mod tests;

pub use globset::Error as GlobError;

pub use crate::{builder::MultiGlobBuilder, dir::DirEntry, walk::MultiGlobWalker};
