use anyhow::Result;

pub fn run(_path: String, _reason: Option<String>) -> Result<()> {
    // TODO: implement lock via gRPC
    println!("lock: not yet implemented (requires server)");
    Ok(())
}
