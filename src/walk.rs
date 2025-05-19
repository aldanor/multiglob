use std::{
    collections::BTreeMap,
    fs, io, mem,
    path::{Path, PathBuf},
    sync::Arc,
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use log::debug;
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
}

impl NodeWalker {
    pub fn new(
        node: WalkPlanNodeCompiled,
        base: PathBuf,
        is_root: bool,
        walkdir_fn: WalkDirFn,
        opts: MultiGlobOptions,
    ) -> Self {
        let state = match node.matcher {
            WalkNodeMatcher::Path { paths } => {
                debug!("creating new path node at {} with paths {paths:?}", base.display());
                let paths = paths.iter().map(|p| base.join(p)).collect();
                NodeWalkerState::Path { paths, index: 0 }
            }
            WalkNodeMatcher::Walk { globset, recursive } => {
                let max_depth = if recursive { usize::MAX } else { 1 };
                // TODO: add depth support from root, track current depth
                debug!("creating new walker at {}, recursive={recursive}", base.display());
                let walker = walkdir_fn(WalkDir::new(&base))
                    .max_depth(max_depth)
                    .follow_root_links(is_root)
                    .into_iter();
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
                    let Ok(mut meta) = fs::symlink_metadata(path) else { continue };
                    let follow = meta.is_symlink() && self.opts.follow_links;
                    if follow {
                        if let Ok(m) = fs::metadata(path) {
                            meta = m;
                        } else {
                            continue;
                        }
                    }
                    entry = Some(DirEntry::from_meta(path.to_path_buf(), meta, follow));
                    self.index_buf.push(i);
                }
                NodeWalkerState::Walk { walker, globset } => {
                    debug!("trying to walk...");
                    let walk_entry = itry!(walker.next()?);
                    debug!("walk entry candidate: {walk_entry:?}");
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
            let is_dir = entry.file_type().is_dir(); // will account for follow_links
                                                     // let follow = entry.path_is_symlink();

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
                        false,
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
    opts: MultiGlobOptions,
    stack: Vec<NodeWalker>,
}

impl MultiGlobWalker {
    pub(crate) fn new(opts: MultiGlobOptions) -> Self {
        Self { opts, stack: Vec::new() }
    }

    pub(crate) fn add(
        &mut self,
        base: PathBuf,
        is_root: bool,
        patterns: Vec<String>,
        skip_invalid: bool,
    ) -> Result<(), GlobError> {
        let plan = WalkPlanNode::build(&patterns);
        let node = WalkPlanNodeCompiled::new(&plan, skip_invalid)?;
        let opts = self.opts.clone();
        let walkdir_fn = Arc::new(move |walkdir| opts.configure_walkdir(walkdir));
        let walker = NodeWalker::new(node, base, is_root, walkdir_fn, self.opts.clone());
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
        while !self.stack.is_empty() {
            match self.stack.last_mut().unwrap().next() {
                None => _ = self.stack.pop(),
                Some(Err(err)) => return Some(Err(err)),
                Some(Ok(mut res)) => {
                    self.stack.append(&mut res.nodes);
                    if let Some(terminal) = res.terminal {
                        return Some(Ok(terminal));
                    }
                }
            };
        }
        None
    }
}
