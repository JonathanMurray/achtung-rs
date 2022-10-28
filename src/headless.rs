use crate::app::ThreadMessage;
use crate::game::{Game, Player, DOWN, LEFT, RIGHT, UP};
use crate::net::{NetworkEvent, Networking};
use crossterm::style::Color;
use std::io::{stdout, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;

pub fn run(socket: TcpStream) {
    let frame = 1;

    let remote_player_i = 0;
    let local_player_i = 1;
    let local_direction = LEFT;
    let (mut networking, size) = Networking::join(
        socket,
        local_player_i,
        remote_player_i,
        local_direction,
        frame,
    );

    println!("Game size: {:?}", size);

    let remote_player = Player::new("P1".to_string(), Color::Blue, ((1, size.1 / 2), RIGHT));
    let local_player = Player::new(
        "P2".to_string(),
        Color::Green,
        ((size.0 - 2, size.1 / 2), local_direction),
    );

    let players = vec![remote_player, local_player];

    let mut game = Game::new(size, players, frame);

    let (sender, receiver) = mpsc::channel();

    let mut frame = frame;

    networking.start_game(sender, false).unwrap();

    let mut input = String::new();
    let stdin = std::io::stdin();
    loop {
        match receiver.try_recv() {
            Ok(ThreadMessage::Network(event)) => match event {
                NetworkEvent::SetDirectionCommand(cmd) => {
                    println!("Received : {:?}", cmd);
                    game.players[cmd.player_i].direction = cmd.direction;
                }
                NetworkEvent::RemoteLeft { .. } => {
                    println!("They left!");
                    break;
                }
                NetworkEvent::ReceiveError(error) => {
                    panic!("Network error: {:?}", error);
                }
            },
            Ok(ThreadMessage::UserInput(event)) => {
                panic!("Headless didn't expect user input: {:?}", event)
            }
            Ok(ThreadMessage::Tick) => panic!("No tick in headless"),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                println!("They left!");
                break;
            }
        }

        print!("> ");
        stdout().flush().unwrap();
        input.clear();
        stdin.read_line(&mut input).unwrap();
        input.make_ascii_lowercase();

        let direction = if input.starts_with('w') {
            Some(UP)
        } else if input.starts_with('a') {
            Some(LEFT)
        } else if input.starts_with('s') {
            Some(DOWN)
        } else if input.starts_with('d') {
            Some(RIGHT)
        } else {
            None
        };

        let command = direction.and_then(|dir| networking.set_direction(dir).unwrap());

        if let Some(cmd) = command {
            game.players[cmd.player_i].direction = cmd.direction;
        }

        networking.commit_frame().unwrap();

        if let Some(report) = game.run_frame() {
            println!("GAME EVENT: {:?}", report);
        }

        frame += 1;
        let commands = networking.start_new_frame(frame).unwrap();
        for cmd in commands {
            game.players[cmd.player_i].direction = cmd.direction;
        }
    }

    networking.exit();
}
