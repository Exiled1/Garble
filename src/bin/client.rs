//! Client entry point & user interface

use anyhow::Result;
use futures::StreamExt;
use std::io::{self, Write};
use tokio::select;

use garble::client::{ChatClient, ChatConnector, ConnectError, Readline};

#[tokio::main]
async fn main() -> Result<()> {
    let (host, port) = garble::parse_command_line_args("localhost", garble::DEFAULT_PORT);

    // Connect & register with the server
    let mut connector = ChatConnector::new(&host, port).await?;

    println!(
        "Your key fingerprint is:\n{}",
        garble::crypto::hash_pub_key(&connector.keypair)
    );
    let mut chat_client: ChatClient = loop {
        print!("Enter the key fingerprint of the user you want to chat with: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.is_empty() {
            // EOF
            return Ok(());
        }

        // Request a connection with the desired client
        println!("Waiting for connection...");
        match connector.request_connection(input.trim().to_string()).await {
            Ok(client) => break client,
            Err(ConnectError::ConnectRefused {
                connector: c,
                reason,
            }) => {
                println!("Connection refused: {reason}");
                connector = c;
                continue;
            }
            Err(e) => anyhow::bail!(e),
        }
    };

    println!("Connected!");
    let mut readline = Readline::new("> ".to_string()).fuse();

    // Forever...
    loop {
        select! {
            // Did the user input a message?
            msg = readline.next() => {
                match msg {
                    Some(msg) => {
                        // Write the message, and send it over the network.
                        readline.get_mut().println(format_args!("> {msg}"));
                        _ = chat_client.outbound.send(msg);
                    }
                    // The user hit Ctrl+D, stop.
                    None => break
                }
            }
            // Or did we recieve something over the network?
            msg = chat_client.inbound.recv() => {
                match msg {
                    // It's a message, write it.
                    Some(Ok(msg)) => readline.get_mut().println(format_args!("< {msg}")),

                    // It's an error, write it.
                    Some(Err(e)) => readline.get_mut().println(format_args!("{e}")),

                    // We've disconnected, stop.
                    None => break
                }
            }
        }
    }

    Ok(())
}
