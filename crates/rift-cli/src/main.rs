// rift-cli binary entry point. Logic lives in lib.rs so integration
// tests can drive it without spawning subprocesses.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rift_cli::run().await
}
