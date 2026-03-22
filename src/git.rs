use std::fmt::Debug;

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct CommitId(git2::Oid);

impl CommitId {
    #[cfg(test)]
    pub fn from_hex_string(hex_string: &str) -> Self {
        git2::Oid::from_str(hex_string)
            .unwrap_or_else(|_| panic!("hex string {} should be valid", hex_string))
            .into()
    }

    pub fn into_object(self, repo: &git2::Repository) -> git2::Commit<'_> {
        repo.find_commit(self.0).unwrap()
    }
}

impl From<git2::Oid> for CommitId {
    fn from(oid: git2::Oid) -> Self {
        Self(oid)
    }
}

impl Debug for CommitId {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let chars: String = self.0.to_string().chars().take(8).collect();
        write!(fmt, "{chars}")
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct BlobId(git2::Oid);

impl BlobId {
    pub fn into_object(self, repo: &git2::Repository) -> git2::Blob<'_> {
        repo.find_blob(self.0).unwrap()
    }
}

impl From<git2::Oid> for BlobId {
    fn from(oid: git2::Oid) -> Self {
        Self(oid)
    }
}
