mod app;
mod game;
mod headless;
mod net;
mod user_interface;

use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::{env, panic};

use anyhow::Result;
use app::App;

use crate::app::GameMode;

const DEFAULT_PORT: u32 = 8000;
pub type Point = (i32, i32);

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let mode = match args.get(1).map(|s| &s[..]) {
        Some("host") => {
            let address = args
                .get(2)
                .map(String::to_string)
                .unwrap_or_else(|| format!("localhost:{}", DEFAULT_PORT));
            let listener = TcpListener::bind(address)?;
            let local_addr = listener.local_addr()?;
            print!("Waiting for client ({:?}) ... ", local_addr);
            io::stdout().flush()?;
            let (socket, address) = listener.accept()?;
            println!("SUCCESS: {:?}", address);
            let name = args
                .get(3)
                .map(String::to_string)
                .unwrap_or_else(|| "Host".to_string());
            GameMode::Host(socket, name)
        }
        Some("client") => {
            let address = args
                .get(2)
                .map(String::to_string)
                .unwrap_or_else(|| format!("localhost:{}", DEFAULT_PORT));
            print!("Connecting to host on {:?} ... ", address);
            io::stdout().flush()?;
            let socket = TcpStream::connect(address)?;
            println!("SUCCESS: {:?}", socket);
            let name = args
                .get(3)
                .map(String::to_string)
                .unwrap_or_else(|| "Client".to_string());
            GameMode::Client(socket, name)
        }
        Some("headless") => {
            let address = args
                .get(2)
                .map(String::to_string)
                .unwrap_or_else(|| format!("localhost:{}", DEFAULT_PORT));
            print!("Connecting to host on {:?} ... ", address);
            io::stdout().flush()?;
            let socket = TcpStream::connect(address)?;
            println!("SUCCESS: {:?}", socket);
            headless::run(socket);
            return Ok(());
        }
        Some(other) => panic!("Invalid game mode: {}", other),
        None => GameMode::Offline,
    };

    let mut app = App::new(mode).expect("Creating app");
    app.run().expect("Running app");

    Ok(())
}
