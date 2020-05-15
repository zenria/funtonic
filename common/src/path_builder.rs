use std::path::{Path, PathBuf};

pub struct PathBuilder {
    inner: PathBuf,
}

impl AsRef<Path> for PathBuilder {
    fn as_ref(&self) -> &Path {
        self.inner.as_path()
    }
}

impl PathBuilder {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Self {
        Self {
            inner: PathBuf::from(path.as_ref()),
        }
    }

    pub fn push<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.inner.push(path.as_ref());
        self
    }

    pub fn build(self) -> PathBuf {
        self.inner
    }
}
