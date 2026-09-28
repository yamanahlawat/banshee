use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    banshee::run().await
}
