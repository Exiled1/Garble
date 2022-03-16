use std::env;
use garble::client::ChatClient;
use garble::server::Server;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    println!("{:?}", args);
    let mut host: String = String::new();
    let mut port: Option<usize> = None;

    if args.len() == 2 || args.len() == 3 {
        host = args[1].to_owned();
        port = args.get(2)
            .and_then(|a| a.to_owned().parse::<usize>().ok())
            .filter(|b| b.to_owned() <= 65535);
    } else {
        eprintln!("Error: wrong number of arguments. Requires hostname and optional port number");
        std::process::exit(1);
    }
    let port = port.unwrap_or(3000);

    println!("{}", host);
    println!("{}", port);

    let chat_client = ChatClient::new(port, host); // make a new oop 

    chat_client.serverconnect().await?;

    loop {
        tokio::select! {
            _ = chat_client.send_message() => {
                println!("Send message!");
            }
            msg = chat_client.recieve_message() => {
                println!("{msg}");
            }
            _ = chat_client.terminate_connection() => {
                println!("Terminating connection");
                break;
            }
        }
    }
    
}
