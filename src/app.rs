use crate::game::{
    Direction, FrameReport, Game, Player, PlayerIndex, DIRECTIONS, DOWN, LEFT, RIGHT, UP,
};
use crate::net::{NetError, NetworkEvent, NetworkPacket, Networking};
use crate::{game, TerminalUi};
use crossterm::event::Event::Key;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Color;
use crossterm::terminal;
use std::cmp::{max, min};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub enum GameMode {
    Host(TcpStream),
    Client(TcpStream),
    Offline,
}

pub struct App {
    game: Game,
    ui: TerminalUi,
    networking: Option<Networking>,
    players_controlled_by_keyboard: Vec<(KeyboardControls, PlayerIndex)>,
    players_controlled_by_ai: Vec<PlayerIndex>,
}

impl App {
    pub fn new(mode: GameMode) -> anyhow::Result<Self> {
        let (terminal_w, terminal_h) = terminal::size()?;
        let w = min(25, terminal_w);
        let h = max(terminal_h, 10);
        let size = (w, h);
        let frame = 1;
        let mut ui = TerminalUi::new(size, frame)?;

        ui.set_banner(Color::Yellow, format!("Achtung! {:?}", size));

        let arrow_controls =
            KeyboardControls::new([KeyCode::Up, KeyCode::Left, KeyCode::Down, KeyCode::Right]);
        let wasd_controls = KeyboardControls::new([
            KeyCode::Char('w'),
            KeyCode::Char('a'),
            KeyCode::Char('s'),
            KeyCode::Char('d'),
        ]);
        let east = ((w - 2, h / 2), LEFT);
        let west = ((1, h / 2), RIGHT);
        let top = ((w / 2, 1), DOWN);
        let bot = ((w / 2, h - 2), UP);

        let networking;
        let players;
        let mut players_controlled_by_keyboard = vec![];
        let mut players_controlled_by_ai = vec![];

        match mode {
            GameMode::Host(socket) => {
                players = vec![
                    Player::new("P1".to_string(), Color::Blue, west),
                    Player::new("P2".to_string(), Color::Green, east),
                ];
                players_controlled_by_keyboard.push((wasd_controls, 0));
                networking = Some(Networking::new(socket, 0, 1));
            }
            GameMode::Client(socket) => {
                players = vec![
                    Player::new("P1".to_string(), Color::Blue, west),
                    Player::new("P2".to_string(), Color::Green, east),
                ];
                players_controlled_by_keyboard.push((wasd_controls, 1));
                networking = Some(Networking::new(socket, 1, 0));
            }
            GameMode::Offline => {
                players = vec![
                    Player::new("P1".to_string(), Color::Blue, west),
                    Player::new("P2".to_string(), Color::Green, east),
                    Player::new("AI 1".to_string(), Color::Blue, top),
                    Player::new("AI 2".to_string(), Color::Cyan, bot),
                ];
                players_controlled_by_keyboard.push((wasd_controls, 0));
                players_controlled_by_keyboard.push((arrow_controls, 1));
                players_controlled_by_ai.push(2);
                players_controlled_by_ai.push(3);
                networking = None;
            }
        };

        let game = Game::new(size, players, frame);

        Ok(Self {
            game,
            networking,
            ui,
            players_controlled_by_keyboard,
            players_controlled_by_ai,
        })
    }

    pub fn run(&mut self, slow_io: bool) -> anyhow::Result<()> {
        let (sender, receiver) = mpsc::channel();
        Self::spawn_periodic_timer(sender.clone());
        Self::spawn_event_receiver(sender.clone());

        if let Some(networking) = &mut self.networking {
            assert_eq!(self.players_controlled_by_keyboard.len(), 1);
            let (_controls, player_i) = &self.players_controlled_by_keyboard[0];
            let frame = self.game.frame;
            let direction = self.game.players[*player_i].direction;
            networking
                .spawn_socket_reader(sender, slow_io)
                .expect("Spawning socket reader");
            let commands = networking
                .start_new_frame(frame, direction)
                .expect("Starting first frame");
            assert!(commands.is_empty());
        }

        let mut even_tick = true;

        loop {
            self.ui.draw_background()?;

            for player in &self.game.players {
                self.ui.draw_colored_line(player.color, &player.line)?;
            }

            self.ui.flush()?;

            match receiver.recv()? {
                ThreadMessage::UserInput(event) => match event {
                    Key(KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        kind: KeyEventKind::Press,
                        state: _,
                    }) => break,
                    Key(KeyEvent {
                        code: KeyCode::Char('q'),
                        ..
                    }) => break,
                    Key(KeyEvent {
                        code,
                        kind: KeyEventKind::Press,
                        ..
                    }) => {
                        for i in 0..self.players_controlled_by_keyboard.len() {
                            let (controls, player_i) = &self.players_controlled_by_keyboard[i];
                            let player_i = *player_i;
                            let player = &self.game.players[player_i];
                            if !player.crashed {
                                if let Some(dir) = controls.handle(code) {
                                    let mut execute_now = true;

                                    if let Some(networking) = &mut self.networking {
                                        match networking.set_direction(self.game.frame, dir) {
                                            Ok(x) => execute_now = x,
                                            Err(error) => self.handle_networking_error(error),
                                        }
                                    }

                                    if execute_now {
                                        self.game.players[player_i].direction = dir;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                },

                ThreadMessage::Network(network_event) => match network_event {
                    // TODO: Let networking module handle more of this
                    NetworkEvent::Received(packet) => match packet {
                        NetworkPacket::SetDirection(cmd) => {
                            let networking = self.networking.as_mut().unwrap();
                            if let Some((player_i, direction)) =
                                networking.receive_command(cmd, self.game.frame)
                            {
                                self.game.players[player_i].direction = direction;
                            }
                        }
                        NetworkPacket::CommitFrame(frame_modulo) => {
                            let networking = self.networking.as_mut().unwrap();
                            networking.receive_commit(frame_modulo, self.game.frame);
                        }
                        NetworkPacket::GoodBye => {
                            self.ui.set_banner(Color::Yellow, "They left!".to_string());
                            self.game.game_over = true;
                        }
                    },
                    NetworkEvent::Error(e) => {
                        panic!("NetworkEvent::Error({:?})", e);
                    }
                    NetworkEvent::RemoteDisconnected => {
                        self.ui
                            .set_banner(Color::Yellow, "Disconnected!".to_string());
                        self.game.game_over = true;
                    }
                },

                ThreadMessage::Tick => {
                    even_tick = !even_tick;

                    if even_tick {
                        if let Some(networking) = self.networking.as_mut() {
                            if !self.game.game_over {
                                if let Err(e) = networking.commit_frame(self.game.frame) {
                                    self.handle_networking_error(e);
                                }
                            }
                        }
                    } else {
                        let ready = self
                            .networking
                            .as_ref()
                            .map(|networking| networking.have_everyone_committed_frame())
                            .unwrap_or(true);
                        if ready && !self.game.game_over {
                            if let Some(report) = self.game.run_frame() {
                                match report {
                                    FrameReport::PlayerCrashed(i) => {
                                        self.ui.set_banner(
                                            Color::Yellow,
                                            format!("{} crashed!", self.game.players[i].name),
                                        );
                                    }
                                    FrameReport::PlayerWon(color, name) => {
                                        self.ui.set_banner(color, format!("{} won!", name));
                                    }
                                    FrameReport::EveryoneCrashed => {
                                        self.ui.set_banner(
                                            Color::Yellow,
                                            "Everyone crashed!".to_string(),
                                        );
                                    }
                                }
                            }

                            for i in 0..self.players_controlled_by_ai.len() {
                                let player_i = self.players_controlled_by_ai[i];
                                if !self.game.players[player_i].crashed {
                                    self.run_player_ai(player_i)
                                }
                            }

                            self.ui.set_frame(self.game.frame);

                            if let Some(networking) = &mut self.networking {
                                assert_eq!(self.players_controlled_by_keyboard.len(), 1);
                                let local_player_i = self.players_controlled_by_keyboard[0].1;
                                let local_direction = self.game.players[local_player_i].direction;

                                match networking.start_new_frame(self.game.frame, local_direction) {
                                    Ok(commands) => {
                                        for (player_i, direction) in commands {
                                            self.game.players[player_i].direction = direction;
                                        }
                                    }
                                    Err(error) => {
                                        self.handle_networking_error(error);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(networking) = &mut self.networking {
            networking.exit();
        }

        Ok(())
    }

    fn handle_networking_error(&mut self, error: NetError) {
        match error {
            NetError::Disconnected => {
                self.ui
                    .set_banner(Color::Yellow, "Oops, they disconnected!".to_string());
                self.game.game_over = true;
            }
            NetError::Other(error) => {
                panic!("Unexpected networking error: {:?}", error);
            }
        }
    }

    fn run_player_ai(&mut self, player_index: PlayerIndex) {
        let ai_head = self.game.players[player_index].head();
        if !self.game.is_vacant(game::translated(
            ai_head,
            self.game.players[player_index].direction,
        )) {
            for dir in DIRECTIONS {
                if self.game.is_vacant(game::translated(ai_head, dir)) {
                    self.game.players[player_index].direction = dir;
                }
            }
        }
    }

    fn spawn_periodic_timer(sender: Sender<ThreadMessage>) {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(75));
            if sender.send(ThreadMessage::Tick).is_err() {
                // no receiver (i.e. main thread has exited)
                break;
            }
        });
    }

    fn spawn_event_receiver(sender: Sender<ThreadMessage>) {
        thread::spawn(move || loop {
            let event = crossterm::event::read().expect("Receiving event");
            if sender.send(ThreadMessage::UserInput(event)).is_err() {
                // no receiver (i.e. main thread has exited)
                break;
            }
        });
    }
}

pub enum ThreadMessage {
    UserInput(Event),
    Network(NetworkEvent),
    Tick,
}

#[derive(Clone)]
struct KeyboardControls {
    map: HashMap<KeyCode, Direction>,
}

impl KeyboardControls {
    fn new(direction_keys: [KeyCode; 4]) -> Self {
        let mut map = HashMap::new();
        map.insert(direction_keys[0], UP);
        map.insert(direction_keys[1], LEFT);
        map.insert(direction_keys[2], DOWN);
        map.insert(direction_keys[3], RIGHT);
        Self { map }
    }

    fn handle(&self, pressed_key_code: KeyCode) -> Option<Direction> {
        self.map.get(&pressed_key_code).copied()
    }
}
