// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge artifacts list|download`.

use anyhow::Result;
use forge_core::workspace::Workspace;
use forge_proto::forge::*;
use std::path::PathBuf;

fn server_url() -> Result<String> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let config = ws.config()?;
    Ok(config
        .default_remote_url()
        .ok_or_else(|| {
            anyhow::anyhow!("No remote configured. Use: forge remote add origin <url>")
        })?
        .to_string())
}

pub fn list(run_id: i64, json_out: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let url = server_url()?;
        let mut client = crate::client::connect_forge(&url).await?;
        let resp = client
            .list_artifacts(ListArtifactsRequest { run_id })
            .await?
            .into_inner();
        if json_out {
            println!("{}", serde_json::to_string_pretty(&resp.artifacts)?);
        } else if resp.artifacts.is_empty() {
            println!("No artifacts for run {}.", run_id);
        } else {
            println!("{:<8} {:<32} {}", "ID", "NAME", "SIZE");
            println!("{}", "-".repeat(56));
            for a in &resp.artifacts {
                println!("{:<8} {:<32} {}", a.id, a.name, human_size(a.size_bytes));
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub fn download(artifact_id: i64, out: Option<PathBuf>) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let url = server_url()?;
        let mut client = crate::client::connect_forge(&url).await?;
        let mut stream = client
            .download_artifact(DownloadArtifactRequest { artifact_id })
            .await?
            .into_inner();

        let out_path = out.unwrap_or_else(|| PathBuf::from(format!("artifact-{artifact_id}.bin")));
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::File::create(&out_path).await?;
        use tokio_stream::StreamExt as _;
        let mut total: u64 = 0;
        while let Some(item) = stream.next().await {
            let chunk = item?;
            file.write_all(&chunk.data).await?;
            total += chunk.data.len() as u64;
        }
        file.flush().await?;
        println!("Saved {} ({}) to {}", artifact_id, human_size(total as i64), out_path.display());
        Ok::<(), anyhow::Error>(())
    })
}

fn human_size(n: i64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, units[i])
}
