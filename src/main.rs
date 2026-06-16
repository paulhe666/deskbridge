#![allow(dead_code)]

mod client;
mod clipboard;
mod file_transfer;
mod input;
mod protocol;
mod server;
mod transport;

use std::env;
use std::process::ExitCode;

#[derive(Debug)]
enum Command {
    Server { bind: String, edge: server::Edge },
    Client { server: String },
}

fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let command = match parse_args(&args) {
        Ok(command) => command,
        Err(e) => {
            eprintln!("{e}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let result = match command {
        Command::Server { bind, edge } => server::run(server::ServerConfig { bind, edge }),
        Command::Client { server } => client::run(&server),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    match args.first().map(String::as_str) {
        Some("server") => {
            let bind = value_after(args, "--bind").unwrap_or_else(|| "0.0.0.0:24920".to_string());
            let edge = value_after(args, "--edge")
                .map(|value| server::Edge::parse(&value))
                .transpose()?
                .unwrap_or(server::Edge::Right);
            Ok(Command::Server { bind, edge })
        }
        Some("client") => {
            let server = value_after(args, "--server")
                .ok_or_else(|| "client mode requires --server HOST:PORT".to_string())?;
            Ok(Command::Client { server })
        }
        _ => Err("missing command: server or client".to_string()),
    }
}

fn value_after(args: &[String], key: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == key)
        .map(|pair| pair[1].clone())
}

fn print_usage() {
    eprintln!(
        "Usage:
  deskbridge server --bind 0.0.0.0:24920
  deskbridge server --bind 0.0.0.0:24920 --edge right
  deskbridge client --server WINDOWS_IP:24920

Goal:
  Windows server + macOS client keyboard/mouse sharing, text/image/file clipboard,
  and file clipboard transfer over a Deskflow-like independent protocol."
    );
}
