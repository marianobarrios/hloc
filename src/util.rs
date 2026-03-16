use chrono::{DateTime, Utc};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, UNIX_EPOCH};
use unicode_segmentation::UnicodeSegmentation;

pub fn datetime_from_epoch_seconds(seconds: i64) -> DateTime<Utc> {
    let epoch: DateTime<Utc> = UNIX_EPOCH.into();
    epoch + Duration::from_secs(seconds as u64)
}

pub trait MutexExt<T> {
    /// `Mutex::lock` returns a result that is almost always unwrapped right away without any
    /// extra treatment. This method does that internally, to avoid polluting client code.
    fn lock_or_panic(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_panic(&self) -> MutexGuard<'_, T> {
        self.lock().expect("lock should not be poisoned")
    }
}

pub trait OsStrExt {
    fn to_str_or_panic(&self) -> &str;
}

impl OsStrExt for OsStr {
    fn to_str_or_panic(&self) -> &str {
        self.to_str().expect("path should be valid UTF-8")
    }
}

pub trait PathExt {
    fn to_str_or_panic(&self) -> &str;
}

impl PathExt for Path {
    fn to_str_or_panic(&self) -> &str {
        self.to_str().expect("path should be valid UTF-8")
    }
}

pub fn merge_options<T>(a: Option<T>, b: Option<T>, merge_fn: fn(T, T) -> T) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(merge_fn(a, b)),
        (opt_a, opt_b) => opt_a.or(opt_b),
    }
}

pub fn truncate_beginning(string: &str, max_graphemes: usize, ellipsis: &str) -> String {
    let truncated: String = string
        .graphemes(true)
        .rev() // start from the end of the string
        .take(max_graphemes)
        .collect::<Vec<_>>()
        .into_iter()
        .rev() // restore original order
        .collect();
    if truncated.len() < string.len() { ellipsis.to_owned() + &truncated } else { truncated }
}

/// Returns the longest common ancestor directory of all the given paths.
///
/// Walks the path component by component, keeping only the prefix that is identical across all
/// inputs. For example, `/a/b/c` and `/a/b/d` share the prefix `/a/b`.
pub fn longest_common_subpath<T>(dirs: &[T]) -> PathBuf
where
    T: AsRef<Path>,
{
    assert!(!dirs.is_empty());
    // Start with all components of the first path as the candidate common prefix.
    let mut common: Vec<_> = dirs[0].as_ref().components().collect();
    for dir in &dirs[1..] {
        // Shorten the candidate to the matching prefix with this path.
        common = common
            .iter()
            .zip(dir.as_ref().components())
            .take_while(|&(a, b)| *a == b)
            .map(|(&a, _)| a)
            .collect();
    }
    common.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_graphemes() {
        assert_eq!(truncate_beginning("abc", 0, "..."), "...");
        assert_eq!(truncate_beginning("abc", 2, "..."), "...bc");
        assert_eq!(truncate_beginning("abc", 3, "..."), "abc");
        assert_eq!(truncate_beginning("abcd", 3, "..."), "...bcd");
    }

    #[test]
    fn test_longest_common_subpath() {
        assert_eq!(longest_common_subpath(&["/a/b/c"]), PathBuf::from("/a/b/c"));
        assert_eq!(longest_common_subpath(&["/a/b", "/a/b"]), PathBuf::from("/a/b"));
        assert_eq!(longest_common_subpath(&["/a/b/c", "/a/b/d"]), PathBuf::from("/a/b"));
        assert_eq!(longest_common_subpath(&["/a/b", "/c/d"]), PathBuf::from("/"));
        assert_eq!(longest_common_subpath(&["/a/b", "/a/b/c"]), PathBuf::from("/a/b"));
        assert_eq!(longest_common_subpath(&["/a/b/c", "/a/b/d", "/a/b/e"]), PathBuf::from("/a/b"));
    }
}
