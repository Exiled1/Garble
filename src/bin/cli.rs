use std::env;

fn main() {
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
}
