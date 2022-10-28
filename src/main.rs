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
use user_interface::TerminalUi;

pub type Point = (u16, u16);

fn main() -> Result<()> {
    const HOST_ADDRESS: &str = "localhost:8000";

    let args: Vec<String> = env::args().collect();
    let mode = match args.get(1).map(|s| &s[..]) {
        Some("host") => {
            let listener = TcpListener::bind(HOST_ADDRESS)?;
            let local_addr = listener.local_addr()?;
            print!("Waiting for client ({:?}) ... ", local_addr);
            io::stdout().flush()?;
            let (socket, address) = listener.accept()?;
            println!("SUCCESS: {:?}", address);
            GameMode::Host(socket)
        }
        Some("client") => {
            print!("Connecting ... ");
            io::stdout().flush()?;
            let socket = TcpStream::connect(HOST_ADDRESS)?;
            println!("SUCCESS: {:?}", socket);
            GameMode::Client(socket)
        }
        Some("headless") => {
            print!("Connecting ... ");
            io::stdout().flush()?;
            let socket = TcpStream::connect(HOST_ADDRESS)?;
            println!("SUCCESS: {:?}", socket);
            headless::run(socket);
            return Ok(());
        }
        Some(other) => panic!("Invalid game mode: {}", other),
        None => GameMode::Offline,
    };

    setup_panic_handler();

    let slow_io = matches!(mode, GameMode::Host(_));
    let mut app = App::new(mode).expect("Creating app");
    app.run(slow_io).expect("Running app");

    Ok(())
}

fn setup_panic_handler() {
    panic::set_hook(Box::new(move |panic_info| {
        let mut stdout = io::stdout();
        user_interface::restore_terminal(&mut stdout);
        eprintln!("Panic: >{:?}<", panic_info);
    }));
}
