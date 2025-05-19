use std::{
    collections::{BTreeMap, HashMap},
    fs, io, mem,
    path::{Path, PathBuf},
    sync::Arc,
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::{builder::MultiGlobOptions, util::is_glob_like, DirEntry, GlobError};

macro_rules! itry {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(err) => return Some(Err(From::from(err))),
        }
    };
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum WalkNodeType {
    #[default]
    Path,
    Glob,
    Walk,
}

#[derive(Default, Clone)]
struct WalkPlanNode {
    node_type: WalkNodeType,
    is_terminal: bool,
    patterns: BTreeMap<String, WalkPlanNode>,
}

impl WalkPlanNode {
    pub fn build(patterns: &[impl AsRef<str>]) -> Self {
        let mut root = Self::default();
        for pattern in patterns {
            let parts: Vec<_> = Path::new(pattern.as_ref())
                .components()
                .map(|c| c.as_os_str().to_str().unwrap())
                .collect();
            root.insert(&parts);
        }
        root.optimize();
        root
    }

    pub fn terminal() -> Self {
        Self { is_terminal: true, ..Self::default() }
    }

    pub fn insert(&mut self, parts: &[&str]) {
        let Some((&part, tail)) = parts.split_first() else {
            self.is_terminal = true;
            return;
        };
        let make_path = || parts.iter().collect::<PathBuf>().to_str().unwrap().to_owned();
        if self.node_type == WalkNodeType::Walk {
            self.patterns.insert(make_path(), Self::terminal());
            return;
        }
        let part = part.to_owned();
        if part.contains("**") {
            self.node_type = WalkNodeType::Walk;
            let mut patterns = Vec::new();
            self.collect(PathBuf::new(), &mut patterns);
            assert!(self.patterns.is_empty());
            for pattern in patterns {
                self.patterns.insert(pattern, Self::terminal());
            }
            self.patterns.insert(make_path(), Self::terminal());
        } else if is_glob_like(&part) {
            self.node_type = WalkNodeType::Glob;
            self.patterns.entry(part).or_default().insert(tail);
        } else {
            self.patterns.entry(part).or_default().insert(tail);
        }
    }

    pub fn collect(&mut self, path: PathBuf, out: &mut Vec<String>) {
        for (k, mut v) in mem::take(&mut self.patterns) {
            let path = path.join(k);
            if v.is_terminal {
                out.push(path.to_str().unwrap().to_owned());
            }
            v.collect(path, out);
        }
    }

    pub fn optimize(&mut self) {
        // squash pure-path component trees into pure-path nodes with multi-part paths
        // note: this code would only makes sense if we always resolved all symlinks for path components

        // for v in self.patterns.values_mut() {
        //     v.optimize();
        // }
        // if self.node_type != WalkNodeType::Path {
        //     return;
        // }
        // let mut patterns = BTreeMap::new();
        // for (k, mut v) in mem::take(&mut self.patterns) {
        //     if v.node_type == WalkNodeType::Path {
        //         if v.is_terminal {
        //             patterns.insert(k.clone(), Self::terminal());
        //         }
        //         for (pk, pv) in mem::take(&mut v.patterns) {
        //             patterns.insert(Path::new(&k).join(&pk).to_str().unwrap().to_owned(), pv);
        //         }
        //     } else {
        //         patterns.insert(k, v);
        //     }
        // }
        // self.patterns = patterns;
    }
}

#[derive(Clone)]
enum WalkNodeMatcher {
    Path { paths: Vec<String> },
    Walk { globset: GlobSet, recursive: bool },
}

#[derive(Clone)]
struct WalkPlanNodeCompiled {
    matcher: WalkNodeMatcher,
    is_terminal: bool,
    destinations: Vec<WalkPlanNodeCompiled>,
}

impl WalkPlanNodeCompiled {
    pub fn new(node: &WalkPlanNode, skip_invalid: bool) -> Result<Self, GlobError> {
        // TODO: when skip_invalid is enabled, it could return a list of globs that failed and errors
        let mut destinations = Vec::new();
        let matcher = if node.node_type == WalkNodeType::Path {
            destinations.extend(node.patterns.values().cloned());
            WalkNodeMatcher::Path { paths: node.patterns.keys().cloned().collect() }
        } else {
            let mut globset = GlobSetBuilder::new();
            for (k, v) in &node.patterns {
                let glob = match Glob::new(k) {
                    Ok(glob) => glob,
                    Err(_) if skip_invalid => continue,
                    Err(err) => return Err(err),
                };
                globset.add(glob);
                destinations.push(v.clone());
            }
            let globset = match globset.build() {
                Ok(globset) => globset,
                Err(_) if skip_invalid => {
                    destinations.clear();
                    GlobSet::empty()
                }
                Err(err) => return Err(err),
            };
            let recursive = node.node_type == WalkNodeType::Walk;
            WalkNodeMatcher::Walk { globset, recursive }
        };
        let destinations =
            destinations.iter().map(|d| Self::new(d, skip_invalid)).collect::<Result<_, _>>()?;
        Ok(Self { matcher, is_terminal: node.is_terminal, destinations })
    }
}

enum NodeWalkerState {
    Path { paths: Vec<PathBuf>, index: usize },
    Walk { globset: GlobSet, walker: walkdir::IntoIter },
}

type WalkDirFn = Arc<dyn Fn(WalkDir) -> WalkDir + Send + Sync + 'static>;

#[derive(Default)]
struct NodeWalkerOutput {
    terminal: Option<DirEntry>,
    nodes: Vec<NodeWalker>,
}

struct NodeWalker {
    base: PathBuf,
    state: NodeWalkerState,
    destinations: Vec<WalkPlanNodeCompiled>,
    index_buf: Vec<usize>,
    walkdir_fn: WalkDirFn,
    opts: MultiGlobOptions,
    follow: bool, // there were symlinks along the way and follow_links was on
}

impl NodeWalker {
    pub fn new(
        node: WalkPlanNodeCompiled,
        base: PathBuf,
        walkdir_fn: WalkDirFn,
        opts: MultiGlobOptions,
    ) -> Self {
        let state = match node.matcher {
            WalkNodeMatcher::Path { paths } => {
                let paths = paths.iter().map(|p| base.join(p)).collect();
                NodeWalkerState::Path { paths, index: 0 }
            }
            WalkNodeMatcher::Walk { globset, recursive } => {
                let max_depth = if recursive { usize::MAX } else { 1 };
                // TODO: add depth support from root, track current depth
                println!("new walker with base: {}", base.display());
                let walker = walkdir_fn(WalkDir::new(&base)).max_depth(max_depth).into_iter();
                NodeWalkerState::Walk { globset, walker }
            }
        };
        Self {
            base,
            state,
            destinations: node.destinations,
            index_buf: Vec::new(),
            walkdir_fn,
            opts,
            follow: false,
        }
    }
}

impl Iterator for NodeWalker {
    type Item = io::Result<NodeWalkerOutput>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut entry = None;
            self.index_buf.clear();

            match &mut self.state {
                NodeWalkerState::Path { paths, index } => {
                    if *index >= paths.len() {
                        return None;
                    }
                    let i = *index;
                    *index += 1;
                    let path = &paths[i];
                    let Ok(mut meta) = fs::symlink_metadata(&path) else { continue };
                    let follow = meta.is_symlink() && self.opts.follow_links;
                    if follow {
                        if let Ok(m) = fs::metadata(&path) {
                            meta = m;
                        } else {
                            continue;
                        }
                    }
                    entry = Some(DirEntry::from_meta(path.to_path_buf(), meta, follow));
                    self.index_buf.push(i);
                }
                NodeWalkerState::Walk { walker, globset } => {
                    let walk_entry = itry!(walker.next()?);
                    if let Ok(path) = walk_entry.path().strip_prefix(&self.base) {
                        globset.matches_into(path, &mut self.index_buf);
                        if !self.index_buf.is_empty() {
                            entry = Some(DirEntry::from_walk(walk_entry));
                        }
                    }
                }
            }

            let Some(entry) = entry else { continue };
            let mut out = NodeWalkerOutput::default();
            let path = entry.path().to_path_buf();
            let is_dir = entry.file_type().is_dir(); // will account for follow_links already
            let mut entry = Some(entry);
            for &i in &self.index_buf {
                let dst = &self.destinations[i];
                if dst.is_terminal && out.terminal.is_none() {
                    out.terminal = entry.take();
                }
                if !dst.destinations.is_empty() && is_dir {
                    out.nodes.push(NodeWalker::new(
                        dst.clone(),
                        path.clone(),
                        self.walkdir_fn.clone(),
                        self.opts.clone(),
                    ));
                }
            }
            if out.terminal.is_some() || !out.nodes.is_empty() {
                return Some(Ok(out));
            }
        }
    }
}

pub struct MultiGlobWalker {
    base: PathBuf,
    opts: MultiGlobOptions,
    stack: Vec<NodeWalker>,
    first: bool,
}

impl MultiGlobWalker {
    pub(crate) fn new(base: PathBuf, opts: MultiGlobOptions) -> Self {
        Self { base, opts, stack: Vec::new(), first: true }
    }

    pub(crate) fn add(
        &mut self,
        base: PathBuf,
        patterns: Vec<String>,
        skip_invalid: bool,
    ) -> Result<(), GlobError> {
        let plan = WalkPlanNode::build(&patterns);
        let node = WalkPlanNodeCompiled::new(&plan, skip_invalid)?;
        let opts = self.opts.clone();
        let walkdir_fn = Arc::new(move |walkdir| opts.configure_walkdir(walkdir));
        let walker = NodeWalker::new(node, base, walkdir_fn, self.opts.clone());
        self.stack.push(walker);
        Ok(())
    }

    pub(crate) fn rev(self) -> Self {
        Self { stack: self.stack.into_iter().rev().collect(), ..self }
    }
}

impl Iterator for MultiGlobWalker {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.first {
            // on first iteration, filter out unreachable base dirs and mark their follow status
            let stack = mem::take(&mut self.stack);
            let follow = check_base_dirs(
                &self.base,
                stack.iter().map(|node| &node.base),
                self.opts.follow_links,
            );
            self.stack = stack
                .into_iter()
                .zip(follow)
                .filter_map(|(node, follow)| follow.map(|f| (node, f)))
                .map(|(mut node, follow)| {
                    node.follow = follow;
                    node
                })
                .collect();
            self.first = false;
        }
        while !self.stack.is_empty() {
            match self.stack.last_mut().unwrap().next() {
                None => _ = self.stack.pop(),
                Some(Err(err)) => return Some(Err(err.into())),
                Some(Ok(mut res)) => {
                    self.stack.extend(res.nodes.drain(..));
                    if let Some(terminal) = res.terminal {
                        return Some(Ok(terminal));
                    }
                }
            };
        }
        None
    }
}

fn check_base_dirs<P: AsRef<Path>>(
    base: &Path,
    paths: impl IntoIterator<Item = P>,
    follow_links: bool,
) -> Vec<Option<bool>> {
    // For a collection of starting directories, check if it's valid directory path if
    // follow_links is enabled, we follow symlinks (note: root link is always being followed).
    // For each base path, return None if target directory is not a directory or if there
    // were symlinks along the way which we couldn't follow (including the target directory
    // itself in case it's not the root one); otherwise, return whether symlinks had to
    // be expanded along the way (not including the base directory). We do it in one go here
    // to avoid duplicate fs::metadata calls.

    fn check_symlink_dir(path: &Path, follow_links: bool) -> Option<bool> {
        // check if path is a directory and whether we had to follow a symlink to resolve it
        let sym = fs::symlink_metadata(path).ok()?;
        if sym.is_dir() {
            Some(false)
        } else if sym.is_symlink() && follow_links {
            fs::metadata(path).ok()?.is_dir().then_some(true)
        } else {
            None
        }
    }

    // note: if follow_links=false, could special-case this because we don't have to check ancestors

    let mut out = Vec::new();
    let mut cache = HashMap::new();
    cache.insert(base.to_owned(), check_symlink_dir(&base, true).map(|_| false));
    'outer: for path in paths {
        let path = path.as_ref();
        let mut follow = false;
        for ancestor in path.ancestors() {
            // starting from the path itself and going upwards
            let Some(is_symlink_dir) = *cache
                .entry(ancestor.to_path_buf())
                .or_insert_with(|| check_symlink_dir(ancestor, follow_links))
            else {
                out.push(None);
                continue 'outer;
            };
            follow |= is_symlink_dir;
            if path == base {
                break;
            }
        }
        out.push(Some(follow));
    }
    out
}
