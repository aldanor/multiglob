use std::{
    collections::BTreeMap,
    path::{Component, Components, Path, PathBuf},
};

/// Check if a component of the path looks like it may be a glob pattern.
///
/// Note: this function is being used when splitting a glob pattern into a long possible
/// base and the glob remainder (scanning through components until we hit the first component
/// for which this function returns true). It is acceptable for this function to return
/// false positives (e.g. patterns like 'foo[bar' or 'foo{bar') in which case correctness
/// will not be affected but efficiency might be (because we'll traverse more than we should),
/// however it should not return false negatives.
pub fn is_glob_like(part: Component) -> bool {
    matches!(part, Component::Normal(_))
        && part.as_os_str().to_str().is_some_and(crate::util::is_glob_like)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GlobParts {
    base: PathBuf,
    pattern: PathBuf,
}

/// Split a glob into longest possible base + shortest possible glob pattern.
fn split_glob(pattern: impl AsRef<str>) -> GlobParts {
    let pattern: &Path = pattern.as_ref().as_ref();

    let mut glob = GlobParts::default();
    let mut globbing = false;
    let mut last = None;

    for part in pattern.components() {
        if let Some(last) = last {
            if last != Component::CurDir {
                if globbing {
                    glob.pattern.push(last);
                } else {
                    glob.base.push(last);
                }
            }
        }
        if !globbing {
            globbing = is_glob_like(part);
        }
        // we don't know if this part is the last one, defer handling it by one iteration
        last = Some(part);
    }

    if let Some(last) = last {
        // defer handling the last component to prevent draining entire pattern into base
        if globbing || matches!(last, Component::Normal(_)) {
            glob.pattern.push(last);
        } else {
            glob.base.push(last);
        }
    }
    glob
}

/// Classic trie with edges being path components and values being glob patterns.
#[derive(Default, Debug)]
struct Trie<'a> {
    children: BTreeMap<Component<'a>, Trie<'a>>,
    patterns: Vec<&'a Path>,
}

impl<'a> Trie<'a> {
    fn insert(&mut self, mut components: Components<'a>, pattern: &'a Path) {
        if let Some(part) = components.next() {
            self.children.entry(part).or_default().insert(components, pattern);
        } else {
            self.patterns.push(pattern);
        }
    }

    /// Iteratively collects groups of patterns from the Trie (no recursion).
    pub fn collect_groups(&self) -> Vec<(PathBuf, Vec<PathBuf>)> {
        /// Defines the current processing mode for a node on the stack.
        enum ModeState {
            /// Evaluate if the current node should start a new group or delegate to children.
            CollectGroups,
            /// Collect patterns for the group currently at the top of `active_group_stack`.
            /// The path accumulates the relative path for patterns within the current group.
            CollectPatterns(PathBuf),
            /// Finalize the group at the top of `active_group_stack` and add it to `out_groups`.
            FinalizeGroup,
        }

        // The main stack for iterative traversal. Each item includes:
        // - A reference to the Trie node to process.
        // - The current path context:
        //   - `CollectGroups` => the prefix that will become the group key if this node is a pivot.
        //   - `CollectPatterns` => the base path for forming new subgroup keys if a non-normal child is found.
        //   - `FinalizeGroup` => this path context is the key of the group being finalized.
        // - The `ModeState` indicating what action to perform.
        let mut stack = vec![(self, PathBuf::new(), ModeState::CollectGroups)];

        // The final list of (group_key, patterns_list) tuples that will be returned.
        let mut out_groups: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();

        // A stack to manage groups that are currently being built.
        // When a pivot node is found (in `CollectGroups`), a new group (group_key, empty_pattern_list)
        // is pushed here. `CollectPatterns` adds patterns to the group at the top of this stack.
        // `FinalizeGroup` moves the top group from this stack to `out_groups`.
        let mut active_group_stack: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();

        while let Some((node, path_context, mode)) = stack.pop() {
            match mode {
                ModeState::CollectGroups => {
                    if node.patterns.is_empty() {
                        // This node is not a pivot (no patterns directly in it).
                        // Child nodes might form their own independent groups.
                        // Push children to the stack to be processed for grouping.
                        // Iterate children in reverse order because the stack is LIFO,
                        // ensuring they are processed in their natural BTreeMap order.
                        for (part, child_node) in node.children.iter().rev() {
                            stack.push((
                                child_node,
                                path_context.join(part),
                                ModeState::CollectGroups,
                            ));
                        }
                    } else {
                        // This node is a pivot point because it contains patterns.
                        // A new group must be formed here with `path_context` as its key.

                        // Add a new group (with an empty pattern list for now) to the active_group_stack.
                        active_group_stack.push((path_context.clone(), Vec::new()));

                        // Schedule the finalization of this new group. This will happen after
                        // all its patterns (and patterns from normal descendants) are collected.
                        stack.push((
                            node, // node itself doesn't matter here
                            path_context.clone(),
                            ModeState::FinalizeGroup,
                        ));

                        // Schedule the collection of patterns for this new group.
                        stack.push((
                            node,
                            path_context,
                            ModeState::CollectPatterns(PathBuf::new()),
                        ));
                    }
                }

                ModeState::CollectPatterns(pattern_prefix) => {
                    // This state assumes a group is active on `active_group_stack`.
                    let active_group = active_group_stack.last_mut().unwrap();

                    // Add all patterns from `node` to this active group. Each pattern is prefixed with the
                    // pattern prefix which represents the path from the group's pivot node down to `node`
                    // via normal components.
                    for pattern in &node.patterns {
                        active_group.1.push(pattern_prefix.join(pattern));
                    }

                    // Process children of `node`.
                    for (part, child_node) in node.children.iter().rev() {
                        let child_path_context = path_context.join(part);
                        if let Component::Normal(_) = part {
                            // If the child is connected by a "Normal" component, continue collecting
                            // patterns for the *current* active group; extend pattern prefix and path context.
                            stack.push((
                                child_node,
                                child_path_context,
                                ModeState::CollectPatterns(pattern_prefix.join(part)),
                            ));
                        } else {
                            // If the child is connected by a non-Normal component, it signifies the start
                            // of a *new*, independent group collection. Push a separate task for this node.
                            stack.push((child_node, child_path_context, ModeState::CollectGroups));
                        }
                    }
                }

                ModeState::FinalizeGroup => {
                    // The group at the top of `active_group_stack` has had all its patterns collected.
                    // Move it to the `out_groups`.
                    out_groups.push(active_group_stack.pop().unwrap());
                }
            }
        }

        out_groups
    }
}

/// Given a collection of globs, cluster them into (base, globs) groups so that:
/// - base doesn't contain any glob symbols
/// - each directory would only be walked at most once
/// - base of each group is the longest common prefix of globs in the group
pub(crate) fn cluster_globs(patterns: &[impl AsRef<str>]) -> Vec<(PathBuf, Vec<String>)> {
    // pub(crate) fn cluster_globs(patterns: &[impl AsRef<str>]) -> Vec<(PathBuf, Vec<String>)> {
    // split all globs into base/pattern
    let globs: Vec<_> = patterns.iter().map(split_glob).collect();

    // construct a path trie out of all split globs
    let mut trie = Trie::default();
    for glob in &globs {
        trie.insert(glob.base.components(), &glob.pattern);
    }

    // run LCP-style aggregation of patterns in the trie into groups
    let groups = trie.collect_groups();

    // finally, convert resulting patterns to strings
    groups
        .into_iter()
        .map(|(base, patterns)| {
            (
                base,
                patterns
                    .iter()
                    // NOTE: this unwrap is ok because input patterns are valid utf-8
                    .map(|p| p.to_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{cluster_globs, split_glob, GlobParts};

    use crate::tests::util::windowsify;

    #[test]
    fn test_split_glob() {
        #[track_caller]
        fn check(input: &str, base: &str, pattern: &str, both: bool) {
            let result = split_glob(input);
            let expected = GlobParts { base: base.into(), pattern: pattern.into() };
            assert_eq!(result, expected, "(1): {input:?} != {base:?} + {pattern:?}");

            if both {
                let result = split_glob(windowsify(input));
                let expected = GlobParts {
                    base: windowsify(base).into(),
                    pattern: windowsify(pattern).into(),
                };
                assert_eq!(result, expected, "(2): {input:?} != {base:?} + {pattern:?}");
            }
        }

        check("", "", "", true);
        check("a", "", "a", true);
        check("a/b", "a", "b", true);
        check("a/b/", "a", "b", true);
        check("a/.//b/", "a", "b", true);
        check("./a/b/c", "a/b", "c", true);
        check("c/d/*", "c/d", "*", true);
        check("c/d/*/../*", "c/d", "*/../*", true);
        check("a/?b/c", "a", "?b/c", true);
        check("/a/b/*", "/a/b", "*", true);
        check("../x/*", "../x", "*", true);
        check("a/{b,c}/d", "a", "{b,c}/d", true);
        check("a/[bc]/d", "a", "[bc]/d", true);
        check("*", "", "*", true);
        check("*/*", "", "*/*", true);
        check("..", "..", "", true);
        check("/", "/", "", true);
        check("/foo/?", "/foo", "?", true);
        check("/foo/bar/*", "/foo/bar", "*", true);

        if cfg!(windows) {
            check(r"C:\a/b\c", r"C:\a\b", r"c", false);
            check(r"C:\a/b\c/*\d/e", r"C:\a\b\c", r"*\d\e", false);
            check(r"C:\*", r"C:\", r"*", false);
            check(r"\\a\b\c\d", r"\\a\b\c", r"d", false);
            check(r"\\a\b\c/*\d/e", r"\\a\b\c", r"*\d\e", false);
            check(r"\\a\b\*", r"\\a\b", r"*", false);
            check(r"/a\b\c", r"\a\b", r"c", false);
            check(r"/a\b/c\*/d\e", r"\a\b\c", r"*\d\e", false);
            check(r"/a/*", r"\a", r"*", false);
            check(r"./a/*", r"a", r"*", false);
        }
    }

    #[test]
    fn test_cluster_globs() {
        #[track_caller]
        fn check(input: &[&str], expected: &[(&str, &[&str])]) {
            let input = input.iter().map(windowsify).collect::<Vec<_>>();

            let mut result_sorted = cluster_globs(&input);
            for (_, patterns) in &mut result_sorted {
                patterns.sort_unstable();
            }
            result_sorted.sort_unstable();

            let mut expected_sorted = Vec::new();
            for (base, patterns) in expected {
                let mut patterns_sorted = Vec::new();
                for pattern in *patterns {
                    patterns_sorted.push(windowsify(pattern));
                }
                patterns_sorted.sort_unstable();
                expected_sorted.push((windowsify(base).into(), patterns_sorted));
            }
            expected_sorted.sort_unstable();

            assert_eq!(
                result_sorted, expected_sorted,
                "{input:?} != {expected_sorted:?} (got: {result_sorted:?})"
            );
        }

        check(&["a/b/*", "a/c/*"], &[("a/b", &["*"]), ("a/c", &["*"])]);
        check(&["./a/b/*", "a/c/*"], &[("a/b", &["*"]), ("a/c", &["*"])]);
        check(&["/a/b/*", "/a/c/*"], &[("/a/b", &["*"]), ("/a/c", &["*"])]);
        check(&["../a/b/*", "../a/c/*"], &[("../a/b", &["*"]), ("../a/c", &["*"])]);
        check(&["x/*", "y/*"], &[("x", &["*"]), ("y", &["*"])]);
        check(&[], &[]);
        check(&["./*", "a/*", "../foo/*.png"], &[("", &["*", "a/*"]), ("../foo", &["*.png"])]);
        check(
            &["?", "/foo/?", "/foo/bar/*", "../bar/*.png", "../bar/../baz/*.jpg"],
            &[
                ("", &["?"]),
                ("/foo", &["?", "bar/*"]),
                ("../bar", &["*.png"]),
                ("../bar/../baz", &["*.jpg"]),
            ],
        );
        check(&["/abs/path/*"], &[("/abs/path", &["*"])]);
        check(&["/abs/*", "rel/*"], &[("/abs", &["*"]), ("rel", &["*"])]);
        check(&["a/{b,c}/*", "a/d?/*"], &[("a", &["{b,c}/*", "d?/*"])]);
        check(
            &[
                "../shared/a/[abc].png",
                "../shared/a/b/*",
                "../shared/b/c/?x/d",
                "docs/important/*.{doc,xls}",
                "docs/important/very/*",
            ],
            &[
                ("../shared/a", &["[abc].png", "b/*"]),
                ("../shared/b/c", &["?x/d"]),
                ("docs/important", &["*.{doc,xls}", "very/*"]),
            ],
        );
        check(&["file.txt"], &[("", &["file.txt"])]);
        check(&["/"], &[("/", &[""])]);
        check(&[".."], &[("..", &[""])]);
        check(&["file1.txt", "file2.txt"], &[("", &["file1.txt", "file2.txt"])]);
        check(&["a/file1.txt", "a/file2.txt"], &[("a", &["file1.txt", "file2.txt"])]);
        check(
            &["*", "a/b/*", "a/../c/*.jpg", "a/../c/*.png", "/a/*", "/b/*"],
            &[
                ("", &["*", "a/b/*"]),
                ("a/../c", &["*.jpg", "*.png"]),
                ("/a", &["*"]),
                ("/b", &["*"]),
            ],
        );

        if cfg!(windows) {
            check(
                &[
                    r"\\foo\bar\shared/a/[abc].png",
                    r"\\foo\bar\shared/a/b/*",
                    r"\\foo\bar/shared/b/c/?x/d",
                    r"D:\docs\important/*.{doc,xls}",
                    r"D:\docs/important/very/*",
                ],
                &[
                    (r"\\foo\bar\shared\a", &["[abc].png", r"b\*"]),
                    (r"\\foo\bar\shared\b\c", &[r"?x\d"]),
                    (r"D:\docs\important", &["*.{doc,xls}", r"very\*"]),
                ],
            );
        }
    }
}
