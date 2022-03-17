pub mod client;
pub mod server;

pub mod crypto;
pub mod message;

pub const DEFAULT_PORT: usize = 4440;

pub fn parse_command_line_args(default_host: &str, default_port: usize) -> (String, usize) {
    let default_port = default_port.to_string();
    let mut args = std::env::args().fuse();
    let (addr, port) = match (args.next(), args.next(), args.next()) {
        (_, None, None) => (String::new(), String::new()),
        (_, Some(s), None) => s
            .split_once(':')
            .map(|(a, p)| (a.to_string(), p.to_string()))
            .unwrap_or((s, String::new())),
        (s, _, _) => usage(s.as_deref()),
    };
    let addr = if !addr.is_empty() {
        &addr
    } else {
        default_host
    };
    let port = if !port.is_empty() {
        &port
    } else {
        &default_port
    };

    if let Ok(port) = port.parse() {
        (addr.to_string(), port)
    } else {
        usage(std::env::args().next().as_deref())
    }
}

fn usage(binary: Option<&str>) -> ! {
    eprintln!("usage: {} [addr][:port]", binary.unwrap_or("garble"));
    std::process::exit(1)
}
