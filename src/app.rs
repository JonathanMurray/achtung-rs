use crate::game::{Direction, FrameReport, Game, Player, DIRECTIONS, DOWN, LEFT, RIGHT, UP};
use crate::{game, TerminalUi};
use crossterm::event::Event::Key;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Color;
use crossterm::terminal;
use std::cmp::{max, min};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
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
    socket: Option<TcpStream>,
    ui: TerminalUi,
    ready_to_run_frame: bool,
    received_commands_for_next_frame: Vec<SetDirection>,
    players_controlled_by_keyboard: Vec<(KeyboardControls, usize)>,
    players_controlled_by_ai: Vec<usize>,
    player_controlled_by_socket: Option<usize>, // TODO combine with socket field somehow
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

        let players;
        let mut players_controlled_by_keyboard = vec![];
        let mut player_controlled_by_socket = None;
        let mut players_controlled_by_ai = vec![];

        match mode {
            GameMode::Host(_) => {
                players = vec![
                    Player::new("P1".to_string(), Color::Blue, west),
                    Player::new("P2".to_string(), Color::Green, east),
                ];
                players_controlled_by_keyboard.push((wasd_controls, 0));
                player_controlled_by_socket = Some(1);
            }
            GameMode::Client(_) => {
                players = vec![
                    Player::new("P1".to_string(), Color::Blue, west),
                    Player::new("P2".to_string(), Color::Green, east),
                ];
                player_controlled_by_socket = Some(0);
                players_controlled_by_keyboard.push((wasd_controls, 1));
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
            }
        };

        let socket = match mode {
            GameMode::Host(socket) => Some(socket),
            GameMode::Client(socket) => Some(socket),
            GameMode::Offline => None,
        };

        let offline = socket.is_none();

        let game = Game::new(size, players, frame);

        Ok(Self {
            game,
            socket,
            ui,
            ready_to_run_frame: offline,
            received_commands_for_next_frame: vec![],
            players_controlled_by_keyboard,
            players_controlled_by_ai,
            player_controlled_by_socket,
        })
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        let (sender, receiver) = mpsc::channel();
        Self::spawn_periodic_timer(sender.clone());
        Self::spawn_event_receiver(sender.clone());

        if let Some(socket) = &self.socket {
            let socket = socket.try_clone()?;
            Self::spawn_socket_reader(sender, socket.try_clone()?);

            let mut socket = socket.try_clone()?;
            assert_eq!(self.players_controlled_by_keyboard.len(), 1);
            let (_controls, player_i) = &self.players_controlled_by_keyboard[0];

            send_direction_command(
                &mut socket,
                self.game.frame,
                self.game.players[*player_i].direction,
            )?;
        }

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
                        for (controls, player_i) in &self.players_controlled_by_keyboard {
                            let player = &mut self.game.players[*player_i];
                            if !player.crashed {
                                if let Some(dir) = controls.handle(code) {
                                    player.direction = dir;
                                    if let Some(socket) = &mut self.socket {
                                        send_direction_command(socket, self.game.frame, dir)?;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                },

                ThreadMessage::Network(network_event) => match network_event {
                    NetworkEvent::Received(packet) => match packet {
                        NetworkPacket::Command(cmd) => {
                            if cmd.frame_modulo == SetDirection::modulo(self.game.frame - 1) {
                                // Too late. That frame has already been run.
                            } else if cmd.frame_modulo == SetDirection::modulo(self.game.frame) {
                                self.ready_to_run_frame = true;
                                let player_i = self.player_controlled_by_socket.unwrap();
                                self.game.players[player_i].direction = cmd.direction;
                            } else if cmd.frame_modulo == SetDirection::modulo(self.game.frame + 1)
                            {
                                self.received_commands_for_next_frame.push(cmd);
                            } else {
                                panic!("Received network packet with unexpected frame modulo: {:?}. Our frame: {}", packet, self.game.frame);
                            }
                        }
                        NetworkPacket::GoodBye => {
                            self.ui.set_banner(Color::Yellow, "They left!".to_string());
                            self.game.game_over = true;
                        }
                    },
                    NetworkEvent::Error(e) => {
                        panic!("{:?}", e);
                    }
                    NetworkEvent::RemoteDisconnected => {
                        self.ui
                            .set_banner(Color::Yellow, "Disconnected!".to_string());
                        self.game.game_over = true;
                    }
                },

                ThreadMessage::Tick => {
                    if self.ready_to_run_frame && !self.game.game_over {
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
                                    self.ui
                                        .set_banner(Color::Yellow, "Everyone crashed!".to_string());
                                }
                            }
                        }

                        for i in 0..self.players_controlled_by_ai.len() {
                            let player_i = self.players_controlled_by_ai[i];
                            if !self.game.players[player_i].crashed {
                                self.run_player_ai(player_i)
                            }
                        }

                        // If online, wait for others' input before next frame
                        self.ready_to_run_frame = self.socket.is_none();
                        self.ui.set_frame(self.game.frame);

                        if let Some(command) = self.received_commands_for_next_frame.last() {
                            assert_eq!(SetDirection::modulo(self.game.frame), command.frame_modulo);
                            let player_i = self.player_controlled_by_socket.unwrap();
                            self.game.players[player_i].direction = command.direction;
                            self.received_commands_for_next_frame.clear();
                            self.ready_to_run_frame = true;
                        }

                        if let Some(socket) = &mut self.socket {
                            assert_eq!(self.players_controlled_by_keyboard.len(), 1);
                            let player_i = self.players_controlled_by_keyboard[0].1;

                            // Send out a direction message for the next frame.
                            // This tells the remote that we're ready for it
                            // TODO: what if remote is further ahead and they immediately
                            // execute the next frame, meaning that we never got a chance
                            // to control our line?
                            send_direction_command(
                                socket,
                                self.game.frame,
                                self.game.players[player_i].direction,
                            )?;
                        }
                    }
                }
            }
        }

        if let Some(socket) = &mut self.socket {
            if let Err(error) = send_net_packet(socket, NetworkPacket::GoodBye) {
                match error.kind() {
                    ErrorKind::ConnectionReset => {}
                    _ => panic!("Failed to send goodbye: {:?}", error),
                }
            }
        }

        Ok(())
    }

    fn run_player_ai(&mut self, player_index: usize) {
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

    fn spawn_socket_reader(sender: Sender<ThreadMessage>, mut socket: TcpStream) {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let mut read_buf = [0; 1024];
            loop {
                match socket.read(&mut read_buf) {
                    Ok(n) => {
                        thread::sleep(Duration::from_millis(100)); //TODO

                        buf.extend_from_slice(&read_buf[..n]);

                        for byte in &buf {
                            let msg = match NetworkPacket::parse(*byte) {
                                Some(msg) => msg,
                                None => {
                                    let msg = ThreadMessage::Network(NetworkEvent::Error(format!(
                                        "Received bad byte: {:?}",
                                        byte
                                    )));
                                    if sender.send(msg).is_err() {
                                        // no receiver (i.e. main thread has exited)
                                    }
                                    return;
                                }
                            };

                            let they_left = matches!(msg, NetworkPacket::GoodBye);
                            let msg = ThreadMessage::Network(NetworkEvent::Received(msg));
                            if sender.send(msg).is_err() {
                                // no receiver (i.e. main thread has exited)
                                return;
                            }
                            if they_left {
                                return;
                            }
                        }
                        buf.clear();
                    }
                    Err(error) => {
                        let msg = match error.kind() {
                            ErrorKind::ConnectionReset => NetworkEvent::RemoteDisconnected,
                            _ => NetworkEvent::Error(format!("Socket error: {:?}", error)),
                        };
                        if sender.send(ThreadMessage::Network(msg)).is_err() {
                            // no receiver (i.e. main thread has exited)
                        }
                        return;
                    }
                }
            }
        });
    }

    fn spawn_periodic_timer(sender: Sender<ThreadMessage>) {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(150));
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

fn send_direction_command(
    socket: &mut TcpStream,
    frame: u32,
    direction: Direction,
) -> std::io::Result<()> {
    socket.write_all(&[NetworkPacket::Command(SetDirection::new(frame, direction)).serialize()])
}

fn send_net_packet(socket: &mut TcpStream, packet: NetworkPacket) -> std::io::Result<()> {
    socket.write_all(&[packet.serialize()])
}

enum ThreadMessage {
    UserInput(Event),
    Network(NetworkEvent),
    Tick,
}

#[derive(Debug)]
enum NetworkEvent {
    Received(NetworkPacket),
    Error(String),
    RemoteDisconnected,
}

#[derive(Debug)]
enum NetworkPacket {
    Command(SetDirection),
    GoodBye,
}

#[derive(Debug, Copy, Clone)]
struct SetDirection {
    frame_modulo: u8,
    direction: Direction,
}

impl SetDirection {
    fn new(frame: u32, direction: Direction) -> Self {
        Self {
            frame_modulo: Self::modulo(frame),
            direction,
        }
    }

    fn modulo(frame: u32) -> u8 {
        (frame % 64) as u8
    }
}

impl NetworkPacket {
    // 00000000 = GoodBye
    // ______dd = direction
    //       00 = UP
    //       01 = LEFT
    //       10 = DOWN
    //       11 = RIGHT
    // ffffff__ = FRAME % 64

    fn parse(byte: u8) -> Option<Self> {
        if byte == 0 {
            return Some(NetworkPacket::GoodBye);
        }

        let frame_modulo = (byte & 0b1111_1100) >> 2;
        let direction = match byte & 0b11 {
            0b00 => UP,
            0b01 => LEFT,
            0b10 => DOWN,
            0b11 => RIGHT,
            _ => return None,
        };
        Some(NetworkPacket::Command(SetDirection {
            frame_modulo,
            direction,
        }))
    }

    fn serialize(&self) -> u8 {
        match self {
            NetworkPacket::Command(SetDirection {
                frame_modulo,
                direction,
            }) => {
                let direction_part = match *direction {
                    UP => 0b00,
                    LEFT => 0b01,
                    DOWN => 0b10,
                    RIGHT => 0b11,
                    _ => panic!("Invalid direction: {:?}", direction),
                };
                (frame_modulo << 2) | direction_part
            }
            NetworkPacket::GoodBye => 0,
        }
    }
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
