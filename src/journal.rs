use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::model::TelemetryEvent;

#[derive(Clone, Debug)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn append(&self, event: &TelemetryEvent) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create journal directory {}", parent.display()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("open journal {}", self.path.display()))?;
        let mut encoded = serde_json::to_vec(event).context("serialize telemetry event")?;
        encoded.push(b'\n');
        file.write_all(&encoded)
            .await
            .with_context(|| format!("append journal {}", self.path.display()))?;
        file.flush().await.context("flush telemetry journal")?;
        Ok(())
    }

    pub async fn append_all<'a, I>(&self, events: I) -> Result<usize>
    where
        I: IntoIterator<Item = &'a TelemetryEvent>,
    {
        let mut count = 0;
        for event in events {
            self.append(event).await?;
            count += 1;
        }
        Ok(count)
    }

    pub async fn read_all(&self) -> Result<Vec<TelemetryEvent>> {
        let file = match fs::File::open(&self.path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| format!("open journal {}", self.path.display()))
            }
        };
        let mut lines = BufReader::new(file).lines();
        let mut events = Vec::new();
        while let Some(line) = lines.next_line().await.context("read journal line")? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<TelemetryEvent>(line) {
                events.push(event);
            }
        }
        Ok(events)
    }
}
