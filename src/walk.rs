use std::{
    collections::BTreeMap,
    fs, io, mem,
    path::{Path, PathBuf},
    sync::Arc,
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::{util::is_glob_like, GlobError};

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
        for v in self.patterns.values_mut() {
            v.optimize();
        }
        if self.node_type != WalkNodeType::Path {
            return;
        }
        let mut patterns = BTreeMap::new();
        for (k, mut v) in mem::take(&mut self.patterns) {
            if v.node_type == WalkNodeType::Path {
                if v.is_terminal {
                    patterns.insert(k.clone(), Self::terminal());
                }
                for (pk, pv) in mem::take(&mut v.patterns) {
                    patterns.insert(Path::new(&k).join(&pk).to_str().unwrap().to_owned(), pv);
                }
            } else {
                patterns.insert(k, v);
            }
        }
        self.patterns = patterns;
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
    fn new(node: &WalkPlanNode, skip_invalid: bool) -> Result<Self, GlobError> {
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

struct NodeWalker {
    base: PathBuf,
    state: NodeWalkerState,
    destinations: Vec<WalkPlanNodeCompiled>,
    index_buf: Vec<usize>,
    walkdir_fn: WalkDirFn,
}

impl NodeWalker {
    pub fn new(node: WalkPlanNodeCompiled, base: PathBuf, walkdir_fn: WalkDirFn) -> Self {
        let state = match node.matcher {
            WalkNodeMatcher::Path { paths } => {
                let paths = paths.iter().map(|p| base.join(p)).collect();
                NodeWalkerState::Path { paths, index: 0 }
            }
            WalkNodeMatcher::Walk { globset, recursive } => {
                let max_depth = if recursive { usize::MAX } else { 1 };
                let walker = walkdir_fn(WalkDir::new(&base)).max_depth(max_depth).into_iter();
                NodeWalkerState::Walk { globset, walker }
            }
        };
        Self { base, state, destinations: node.destinations, index_buf: Vec::new(), walkdir_fn }
    }

    fn build_output(
        destinations: &[WalkPlanNodeCompiled],
        walkdir_fn: &WalkDirFn,
        path: &Path,
        index: impl IntoIterator<Item = usize>,
        is_dir: bool,
    ) -> Option<NodeWalkerOutput> {
        let mut out = NodeWalkerOutput::default();
        for i in index {
            let dst = &destinations[i];
            if dst.is_terminal && out.terminal.is_none() {
                out.terminal = Some(path.into());
            }
            if !dst.destinations.is_empty() && is_dir {
                out.nodes.push(NodeWalker::new(dst.clone(), path.into(), walkdir_fn.clone()));
            }
        }
        (out.terminal.is_some() || !out.nodes.is_empty()).then_some(out)
    }
}

#[derive(Default)]
struct NodeWalkerOutput {
    terminal: Option<PathBuf>, // TODO: mimic DirEntry
    nodes: Vec<NodeWalker>,
}

impl Iterator for NodeWalker {
    type Item = io::Result<NodeWalkerOutput>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match &mut self.state {
                NodeWalkerState::Path { paths, index } => {
                    if *index >= paths.len() {
                        return None;
                    }
                    let i = *index;
                    *index += 1;
                    let path = &paths[i];
                    let Ok(meta) = fs::metadata(path) else {
                        continue;
                    };
                    if let Some(out) = Self::build_output(
                        &self.destinations,
                        &self.walkdir_fn,
                        path,
                        [i],
                        meta.is_dir(),
                    ) {
                        return Some(Ok(out));
                    } else {
                        continue;
                    }
                }
                NodeWalkerState::Walk { walker, globset } => {
                    let entry = walker.next()?;
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(err) => return Some(Err(err.into())),
                    };
                    let is_dir = entry.file_type().is_dir(); // requires follow_links for this to be true
                    if let Ok(path) = entry.path().strip_prefix(&self.base) {
                        globset.matches_into(path, &mut self.index_buf);
                        // TODO: preserve DirEntry and its metadata
                        if let Some(out) = Self::build_output(
                            &self.destinations,
                            &self.walkdir_fn,
                            path,
                            self.index_buf.iter().copied(),
                            is_dir,
                        ) {
                            return Some(Ok(out));
                        } else {
                            continue;
                        }
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub struct MultiGlobWalker {
    walker: Option<NodeWalker>,
    stack: Vec<NodeWalker>,
}

impl MultiGlobWalker {
    pub fn add(
        &mut self,
        base: PathBuf,
        patterns: Vec<String>,
        walkdir_fn: WalkDirFn,
        skip_invalid: bool,
    ) -> Result<(), GlobError> {
        let plan = WalkPlanNode::build(&patterns);
        let node = WalkPlanNodeCompiled::new(&plan, skip_invalid)?;
        let walker = NodeWalker::new(node, base, walkdir_fn.clone());
        self.stack.push(walker);
        Ok(())
    }

    pub fn rev(self) -> Self {
        Self { stack: self.stack.into_iter().rev().collect(), ..self }
    }
}

impl Iterator for MultiGlobWalker {
    type Item = io::Result<PathBuf>; // TODO: wrap DirEntry

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.walker.is_none() {
                self.walker = self.stack.pop();
                if self.walker.is_none() {
                    return None;
                }
            }
            match self.walker.as_mut().unwrap().next() {
                None => self.walker = None,
                Some(Err(err)) => return Some(Err(err.into())),
                Some(Ok(mut res)) => {
                    self.stack.extend(res.nodes.drain(..));
                    if let Some(terminal) = res.terminal {
                        return Some(Ok(terminal)); // TODO: DirEntry
                    }
                }
            };
        }
    }
}
