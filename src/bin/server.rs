use garble::server::Server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    #[cfg(feature = "console-subscriber")]
    console_subscriber::init();

    // Parse-command line arguments
    let mut args = std::env::args().fuse();
    let (addr, port) = match (args.next(), args.next(), args.next()) {
        (_, None, None) => (String::new(), String::new()),
        (_, Some(s), None) => s
            .split_once(':')
            .map(|(a, p)| (a.to_string(), p.to_string()))
            .unwrap_or((s, String::new())),
        (s, _, _) => {
            eprintln!("usage: {} [addr][:port]", s.as_deref().unwrap_or("server"));
            std::process::exit(1)
        }
    };
    let bind_addr = std::format!(
        "{}:{}",
        if !addr.is_empty() { &addr } else { "0.0.0.0" },
        if !port.is_empty() { &port } else { "8080" }
    );

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
