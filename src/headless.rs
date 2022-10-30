use crate::app::ThreadMessage;
use crate::game::{Game, Player, DOWN, LEFT, RIGHT, UP};
use crate::net::{NetworkEvent, Networking, Outcome};
use std::io::{stdout, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use tui::style::Color;

pub fn run(socket: TcpStream) {
    let frame = 1;

    let remote_player_i = 0;
    let local_player_i = 1;
    let local_direction = LEFT;
    let local_player_name = "Headless client".to_string();
    let (mut networking, game_info) = Networking::join(
        socket,
        local_player_i,
        remote_player_i,
        local_direction,
        frame,
        local_player_name.clone(),
    );

    println!("Game info: {:?}", game_info);

    let size = game_info.size;
    let remote_player = Player::new(
        game_info.remote_player_name,
        Color::Blue,
        ((1, (size.1 / 2) as i32), RIGHT),
    );
    let local_player = Player::new(
        local_player_name,
        Color::Green,
        (((size.0 - 2) as i32, (size.1 / 2) as i32), local_direction),
    );

    let players = vec![remote_player, local_player];

    let mut game = Game::new(size, players, frame);

    let (sender, receiver) = mpsc::channel();

    networking.start_game(sender).unwrap();

    let mut input = String::new();
    let stdin = std::io::stdin();
    while !game.game_over {
        println!("~~ frame {} ~~", game.frame);
        match receiver.try_recv() {
            Ok(ThreadMessage::Network(event)) => match event {
                NetworkEvent::BufferedOutcomes => {
                    let outcomes = networking.take_buffered_outcomes();
                    execute_outcomes(&mut game, &mut networking, outcomes);
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
                panic!("Network reader thread died")
            }
        }

        print!("> ");
        stdout().flush().unwrap();
        input.clear();
        stdin.read_line(&mut input).unwrap();
        input.make_ascii_lowercase();

        let input_direction = if input.starts_with('w') {
            Some(UP)
        } else if input.starts_with('a') {
            Some(LEFT)
        } else if input.starts_with('s') {
            Some(DOWN)
        } else if input.starts_with('d') {
            Some(RIGHT)
        } else if input.starts_with('q') {
            break;
        } else {
            None
        };

        if let Some(direction) = input_direction {
            let outcomes = networking.set_direction(direction).unwrap();
            execute_outcomes(&mut game, &mut networking, outcomes);
        }

        println!("Committing frame.");
        let outcomes = networking.commit_frame().unwrap();
        execute_outcomes(&mut game, &mut networking, outcomes);
    }

    println!("Game over. Press enter to exit.");
    let mut input = String::new();
    stdin.read_line(&mut input).unwrap();

    networking.exit();
}

fn execute_outcomes(game: &mut Game, networking: &mut Networking, outcomes: Vec<Outcome>) {
    for outcome in outcomes {
        println!("  outcome: {:?}", outcome);
        match outcome {
            Outcome::PlayerControl(control) => {
                game.players[control.player_i].direction = control.direction;
            }
            Outcome::RunFrame => {
                println!("  Running frame {}", game.frame);

                let frame_events = game.run_frame();
                if !frame_events.is_empty() {
                    println!("  Game events: {:?}", frame_events);
                }
                println!("  State: {:?}", game.players);

                let outcomes = networking.start_new_frame(game.frame).unwrap();
                execute_outcomes(game, networking, outcomes);
            }
            Outcome::RemoteLeft { .. } => {
                println!("  They left!");
                game.game_over = true;
            }
        }
    }
}
