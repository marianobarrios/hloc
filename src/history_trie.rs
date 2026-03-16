//! Fork detection via a commit-history trie.
//!
//! When two repositories share a common Git history (one is a fork of the other), counting both
//! would double-count the shared commits. This module solves that by inserting every repository's
//! commit sequence into a trie keyed on commit IDs, then doing a single BFS pass to assign each
//! shared commit to exactly one repository — the one deemed most "original" according to its
//! [`fork_priority`](crate::config::RepoConfig::fork_priority).
//!
//! # How the trie encodes histories
//!
//! Each edge in the trie corresponds to one commit. A path from the root to a node therefore
//! represents a commit sequence. Every node records which repositories pass through it. A special
//! [`CommitRef::EoH`] (End-of-History) sentinel is appended at the end of each repository's
//! sequence so that we can distinguish where each history terminates, even when two repositories
//! share all of their commits up to the very last one.

use crate::git::CommitId;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A trie whose keys are sequences of Git commit IDs.
///
/// Each inserted repository occupies a root-to-leaf path in the trie. Shared prefixes between
/// paths represent shared commit history (forks).
#[derive(Debug)]
pub struct HistoryTrie {
    all_repos: HashMap<PathBuf, Rc<Repo>>,
    root: TrieNode,
}

impl HistoryTrie {
    pub fn new() -> Self {
        Self { all_repos: HashMap::new(), root: TrieNode::default() }
    }
}

/// A single node in the [`HistoryTrie`].
///
/// `repos` lists every repository whose sampled history passes through this node (i.e. has the
/// corresponding commit). `children` maps the next commit (or `EoH`) to the child node.
#[derive(Debug, Default)]
struct TrieNode {
    repos: Vec<Rc<Repo>>,
    children: HashMap<CommitRef, TrieNode>,
}

/// A repository entry stored inside [`TrieNode`], carrying the information needed to rank
/// repositories when their histories share a common prefix.
#[derive(Debug, PartialEq, Eq)]
struct Repo {
    path: PathBuf,
    priority: i32,
}

impl Ord for Repo {
    /// A lower ordering means that the repository is considered the original in a fork.
    /// Using the supplied value, and breaking ties with the alphabetical ordering of the name, for
    /// stability.
    fn cmp(&self, other: &Self) -> Ordering {
        (self.priority, &self.path).cmp(&(other.priority, &other.path))
    }
}

impl PartialOrd for Repo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A reference to a position in a repository's commit sequence.
#[derive(Debug, Copy, Clone, Hash)]
// Our custom `PartialEq` implementation is consistent with the generated `Hash` one
#[allow(clippy::derived_hash_with_manual_eq)]
enum CommitRef {
    /// Normal commit
    GitCommit(CommitId),

    /// End-of-history pseudo-commit.
    ///
    /// Used as a sentinel appended after the last real commit of each repository. Two `EoH`
    /// values are intentionally never equal to each other so that each repository gets its own
    /// distinct leaf node, even when their full histories are identical.
    EoH,
}

impl PartialEq<Self> for CommitRef {
    /// [`CommitRef::GitCommit`] variants are compared normally; [`CommitRef::EoH`] variants are never equal among them.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CommitRef::GitCommit(left), CommitRef::GitCommit(right)) => left.eq(right),
            _ => false,
        }
    }
}

impl Eq for CommitRef {}

impl HistoryTrie {
    /// Inserts a repository's sampled commit sequence into the trie.
    ///
    /// Each commit in `commits` creates (or navigates to) a child node, and the repository is
    /// recorded at every node it passes through. An [`CommitRef::EoH`] sentinel is appended after
    /// the last commit to mark where this repository's history ends.
    pub fn insert(&mut self, repo_path: &Path, priority: i32, commits: &[CommitId]) {
        assert!(!commits.is_empty());
        let repo = self
            .all_repos
            .entry(repo_path.to_owned())
            .or_insert_with(|| Rc::new(Repo { path: repo_path.to_owned(), priority }));

        let mut current_node = &mut self.root;

        for &commit in commits {
            // Navigate to the child node, creating it if it doesn't exist.
            let node = current_node.children.entry(CommitRef::GitCommit(commit)).or_default();
            node.repos.push(repo.clone());
            current_node = node;
        }

        let mut node = TrieNode::default();
        node.repos.push(repo.clone());
        let existing = current_node.children.insert(CommitRef::EoH, node);
        assert!(existing.is_none());
    }

    /// Returns the de-duplicated commit list for every repository.
    ///
    /// Performs a BFS over the trie. Each queue entry carries the commit path accumulated from
    /// the root to the current node. At branching points the children are sorted by priority:
    ///
    /// - The first child inherits the full accumulated path (keeps all shared commits).
    /// - Remaining children receive only their own diverging commit as the start of a fresh path,
    ///   effectively stripping the shared prefix from their count.
    ///
    /// When an [`CommitRef::EoH`] node is reached, the accumulated path is recorded as the final
    /// commit list for all repositories that terminate there.
    pub fn get_all_sequences_iterative(&self) -> HashMap<PathBuf, Vec<CommitId>> {
        let mut results = HashMap::new();

        // Each queue entry holds the path to the current node and a reference to it.
        let mut queue: VecDeque<(Vec<CommitRef>, &TrieNode)> = VecDeque::new();

        // Initial queue population
        queue.push_back((vec![], &self.root));

        while let Some((path, node)) = queue.pop_front() {
            if let Some(CommitRef::EoH) = path.last() {
                let converted_path = Self::extract_path(&path);
                for repo in &node.repos {
                    results.insert(repo.path.to_owned(), converted_path.clone());
                }
            }

            let mut children: Vec<_> = node.children.iter().collect();

            // priority is applied here
            children.sort_by_key(|&(_, node)| node.repos.iter().min().unwrap());

            if let Some(&(&commit, node)) = children.first() {
                // First (chosen) child, add complete path to queue
                let mut child_path = path.clone();
                child_path.push(commit);
                queue.push_back((child_path, node));

                // Rest of the children, add only the last commit to queue, avoiding duplicated commits
                for &(&commit, node) in children.iter().skip(1) {
                    queue.push_back((vec![commit], node));
                }
            }
        }

        results
    }

    /// Strips the trailing [`CommitRef::EoH`] sentinel and converts the remaining
    /// [`CommitRef::GitCommit`] entries into plain [`CommitId`]s.
    fn extract_path(path: &[CommitRef]) -> Vec<CommitId> {
        let mut result = Vec::new();
        let mut it = path.iter();
        while let Some(CommitRef::GitCommit(commit)) = it.next() {
            result.push(*commit);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::git::CommitId;
    use crate::history_trie::HistoryTrie;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[test]
    fn test() {
        let commit1 = CommitId::from_hex_string("1").unwrap();
        let commit2 = CommitId::from_hex_string("2").unwrap();
        let commit3 = CommitId::from_hex_string("3").unwrap();
        let commit4 = CommitId::from_hex_string("4").unwrap();
        let commit5 = CommitId::from_hex_string("5").unwrap();

        let repo1 = PathBuf::from("repo1");
        let repo2 = PathBuf::from("repo2");

        // Unrelated histories
        do_test(
            &HashMap::from_iter([
                ((repo1.as_path(), 0), vec![commit1, commit2, commit3]),
                ((repo2.as_path(), 1), vec![commit4, commit5]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit4, commit5]),
            ]),
        );

        // Fork is a strict superset
        do_test(
            &HashMap::from_iter([
                ((repo1.as_path(), 0), vec![commit1, commit2, commit3]),
                ((repo2.as_path(), 1), vec![commit1, commit2, commit3, commit4]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit4]),
            ]),
        );

        // "Fork" is a strict subset
        do_test(
            &HashMap::from_iter([
                ((repo1.as_path(), 0), vec![commit1, commit2, commit3, commit4]),
                ((repo2.as_path(), 1), vec![commit1, commit2, commit3]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3, commit4]),
                (repo2.clone(), vec![]),
            ]),
        );

        // Identical histories
        do_test(
            &HashMap::from_iter([
                ((repo1.as_path(), 0), vec![commit1, commit2, commit3]),
                ((repo2.as_path(), 1), vec![commit1, commit2, commit3]),
            ]),
            &HashMap::from_iter([(repo1.clone(), vec![commit1, commit2, commit3]), (repo2.clone(), vec![])]),
        );

        // Diverging histories - Shorter is kept
        do_test(
            &HashMap::from_iter([
                ((repo1.as_path(), 0), vec![commit1, commit2, commit3]),
                ((repo2.as_path(), 1), vec![commit1, commit2, commit4, commit5]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit4, commit5]),
            ]),
        );

        // Diverging histories - Longer is kept
        do_test(
            &HashMap::from_iter([
                ((repo1.as_path(), 0), vec![commit1, commit2, commit4, commit5]),
                ((repo2.as_path(), 1), vec![commit1, commit2, commit3]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit4, commit5]),
                (repo2.clone(), vec![commit3]),
            ]),
        );
    }

    fn do_test(repos: &HashMap<(&Path, i32), Vec<CommitId>>, expected: &HashMap<PathBuf, Vec<CommitId>>) {
        let mut trie = HistoryTrie::new();
        for ((repo, priority), commits) in repos {
            trie.insert(repo, *priority, &commits);
        }
        assert_eq!(&trie.get_all_sequences_iterative(), expected);
    }
}
