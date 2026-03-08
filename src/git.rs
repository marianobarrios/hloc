#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct CommitId(git2::Oid);

impl CommitId {
    pub fn from_oid(oid: git2::Oid) -> Self {
        Self(oid)
    }

    pub fn to_object<'r>(self, repo: &'r git2::Repository) -> git2::Commit<'r> {
        repo.find_commit(self.0).unwrap()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct BlobId(git2::Oid);

impl BlobId {
    pub fn from_oid(oid: git2::Oid) -> Self {
        Self(oid)
    }

    pub fn to_object<'r>(self, repo: &'r git2::Repository) -> git2::Blob<'r> {
        repo.find_blob(self.0).unwrap()
    }
}
