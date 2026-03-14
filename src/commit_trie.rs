use crate::git::CommitId;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct HistoryTrie {
    root: TrieNode,
}

impl HistoryTrie {
    pub fn new() -> Self {
        Self { root: TrieNode::default() }
    }
}

#[derive(Debug, Default)]
struct TrieNode {
    repos: Vec<Repo>,
    children: HashMap<CommitRef, TrieNode>,
}

#[derive(Debug, PartialEq, Eq)]
struct Repo {
    path: PathBuf,
    priority: i32,
}

impl Ord for Repo {
    /// A lower ordering means that the repository is considered the original in a fork.
    /// Using the supplied value, and breaking ties with the alphabetical soring of the name, for
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

#[derive(Debug, Copy, Clone, Hash)]
// Our custom `PartialEq` implementation is consistent with the generated `Hash` one
#[allow(clippy::derived_hash_with_manual_eq)]
enum CommitRef {
    /// Normal commit
    GitCommit(CommitId),

    /// End-of-history pseudo-commit
    EoH,
}

impl PartialEq<Self> for CommitRef {
    /// [`CommitRef::GitCommit`] variants are compared normally, [`CommitRef::EoH`] variants are never igual among them.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CommitRef::GitCommit(left), CommitRef::GitCommit(right)) => left.eq(right),
            _ => false,
        }
    }
}

impl Eq for CommitRef {}

impl HistoryTrie {
    pub fn insert(&mut self, repo: &Path, priority: i32, commits: &[CommitId]) {
        assert!(!commits.is_empty());
        let mut current_node = &mut self.root;

        for &commit in commits {
            // Navigate to the child node, creating it if it doesn't exist.
            let node = current_node.children.entry(CommitRef::GitCommit(commit)).or_default();
            node.repos.push(Repo { path: repo.to_owned(), priority });
            current_node = node;
        }

        let mut node = TrieNode::default();
        node.repos.push(Repo { path: repo.to_owned(), priority });
        let existing = current_node.children.insert(CommitRef::EoH, node);
        assert!(existing.is_none());
    }

    /// Iteratively collects and returns all sequences as a Vector of Vectors.
    /// Uses a queue to implement breadth-first traversal.
    pub fn get_all_sequences_iterative(&self) -> HashMap<PathBuf, Vec<CommitId>> {
        let mut results = HashMap::new();

        // Each queue entry holds the path to the current node and a reference to it.
        let mut queue: VecDeque<(Vec<CommitRef>, &TrieNode)> = VecDeque::new();

        // Initial queue population
        queue.push_back((vec![], &self.root));

        while let Some((path, node)) = queue.pop_front() {
            if let Some(CommitRef::EoH) = path.last() {
                let converted_path = Self::convert_path(&path);
                for repo in &node.repos {
                    results.insert(repo.path.to_owned(), converted_path.clone());
                }
            }

            let mut children: Vec<_> = node.children.iter().collect();
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

    fn convert_path(path: &[CommitRef]) -> Vec<CommitId> {
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
    use crate::commit_trie::HistoryTrie;
    use crate::git::CommitId;
    use std::collections::HashMap;
    use std::path::PathBuf;

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
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit4, commit5]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit4, commit5]),
            ]),
        );

        // Fork is a strict superset
        do_test(
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit1, commit2, commit3, commit4]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit4]),
            ]),
        );

        // "Fork" is a strict subset
        do_test(
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3, commit4]),
                (repo2.clone(), vec![commit1, commit2, commit3]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3, commit4]),
                (repo2.clone(), vec![]),
            ]),
        );

        // Identical histories
        do_test(
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit1, commit2, commit3]),
            ]),
            &HashMap::from_iter([(repo1.clone(), vec![commit1, commit2, commit3]), (repo2.clone(), vec![])]),
        );

        // Diverging histories - Shorter is kept
        do_test(
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit1, commit2, commit4, commit5]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit3]),
                (repo2.clone(), vec![commit4, commit5]),
            ]),
        );

        // Diverging histories - Longer is kept
        do_test(
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit4, commit5]),
                (repo2.clone(), vec![commit1, commit2, commit3]),
            ]),
            &HashMap::from_iter([
                (repo1.clone(), vec![commit1, commit2, commit4, commit5]),
                (repo2.clone(), vec![commit3]),
            ]),
        );
    }

    fn do_test(repos: &HashMap<PathBuf, Vec<CommitId>>, expected: &HashMap<PathBuf, Vec<CommitId>>) {
        let mut trie = HistoryTrie::new();
        for (repo, commits) in repos {
            trie.insert(repo.as_ref(), 0, &commits);
        }
        assert_eq!(&trie.get_all_sequences_iterative(), expected);
    }
}
