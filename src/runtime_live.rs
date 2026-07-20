#[allow(dead_code)]
mod legacy {
    include!("runtime.rs");
}

pub use legacy::{
    connect_sse, ingest_file, ingest_reader, snapshot_from_journal, wrap_command, IngestStats,
};

use std::path::Path;

use anyhow::Result;

use crate::live::load_cached_snapshot;
use crate::model::AppSnapshot;

pub async fn load_snapshot(data_dir: &Path) -> Result<AppSnapshot> {
    load_cached_snapshot(data_dir).await
}
