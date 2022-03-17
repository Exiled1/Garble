//! Server entrypoint & command-line argument parsing
use garble::server::Server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    #[cfg(feature = "console-subscriber")]
    console_subscriber::init();

    // Parse-command line arguments
    let (addr, port) = garble::parse_command_line_args("0.0.0.0", garble::DEFAULT_PORT);
    let bind_addr = std::format!("{}:{}", addr, port);
    // Start the server
    let mut shutdown_handle = Server::start(&bind_addr).await?;
    println!("Listening on {bind_addr}");

    // Wait for a shutdown signal
    tokio::select! {
        _ = shutdown_handle.stopped() => {},
        _ = tokio::signal::ctrl_c() => {},
    };

    // Stop the server
    println!("Shutting down...");
    shutdown_handle.shutdown().await?;

    Ok(())
}
