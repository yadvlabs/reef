//! Step logs and artifacts live on the shared filesystem under `<data_dir>/actions`, so every
//! host serves the same files.

use reef_core::{Error, Result};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
pub struct Files {
    root: PathBuf,
}

impl Files {
    pub fn new(data_dir: &Path) -> Files {
        Files { root: data_dir.join("actions") }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn log_rel(run: i64, job: i64, log: i64) -> String {
        format!("logs/{run}/{job}/{log}.log")
    }

    pub fn artifact_rel(run: i64, artifact: i64) -> String {
        format!("artifacts/{run}/{artifact}.zip")
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub async fn append(&self, rel: &str, data: &[u8]) -> Result<()> {
        let p = self.abs(rel);
        if let Some(d) = p.parent() {
            tokio::fs::create_dir_all(d).await.map_err(Error::internal)?;
        }
        let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(&p).await.map_err(Error::internal)?;
        f.write_all(data).await.map_err(Error::internal)?;
        Ok(())
    }

    pub async fn write(&self, rel: &str, data: &[u8]) -> Result<()> {
        let p = self.abs(rel);
        if let Some(d) = p.parent() {
            tokio::fs::create_dir_all(d).await.map_err(Error::internal)?;
        }
        tokio::fs::write(&p, data).await.map_err(Error::internal)?;
        Ok(())
    }

    pub async fn read(&self, rel: &str) -> Result<Vec<u8>> {
        match tokio::fs::read(self.abs(rel)).await {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(Error::internal(e)),
        }
    }

    pub async fn remove(&self, rel: &str) -> Result<()> {
        match tokio::fs::remove_file(self.abs(rel)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::internal(e)),
        }
    }

    pub async fn remove_run(&self, run: i64) {
        let _ = tokio::fs::remove_dir_all(self.root.join(format!("logs/{run}"))).await;
        let _ = tokio::fs::remove_dir_all(self.root.join(format!("artifacts/{run}"))).await;
    }
}
