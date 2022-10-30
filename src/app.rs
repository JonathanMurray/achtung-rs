use crate::game::{
    self, Direction, FrameEvent, Game, Player, PlayerIndex, DIRECTIONS, DOWN, LEFT, RIGHT, UP,
};
use crate::net::{NetResult, NetworkEvent, Networking, Outcome};
use crate::user_interface::TerminalUi;
use crate::Point;
use crossterm::event::Event::Key;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use tui::style::Color;

#[derive(Debug)]
pub enum GameMode {
    Host(TcpStream, String),
    Client(TcpStream, String),
    Offline,
}

pub enum StartPosition {
    North,
    West,
    South,
    East,
}

impl StartPosition {
    fn resolve(&self, size: (u16, u16)) -> (Point, Direction) {
        match self {
            StartPosition::North => (((size.0 / 2) as i32, 0), DOWN),
            StartPosition::West => ((0, (size.1 / 2) as i32), RIGHT),
            StartPosition::South => (((size.0 / 2) as i32, (size.1 - 1) as i32), UP),
            StartPosition::East => (((size.0 - 1) as i32, (size.1 / 2) as i32), LEFT),
        }
    }

    fn direction(&self) -> Direction {
        match self {
            StartPosition::North => DOWN,
            StartPosition::West => RIGHT,
            StartPosition::South => UP,
            StartPosition::East => LEFT,
        }
    }
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
        let suggested_game_size = (35, 15);
        let game_size;

        let arrow_controls =
            KeyboardControls::new([KeyCode::Up, KeyCode::Left, KeyCode::Down, KeyCode::Right]);
        let wasd_controls = KeyboardControls::new([
            KeyCode::Char('w'),
            KeyCode::Char('a'),
            KeyCode::Char('s'),
            KeyCode::Char('d'),
        ]);

        let frame = 1;

        let networking;
        let players;
        let mut players_controlled_by_keyboard = vec![];
        let mut players_controlled_by_ai = vec![];

        match mode {
            GameMode::Host(socket, local_name) => {
                game_size = suggested_game_size;
                let local_player = Player::new(
                    local_name.clone(),
                    Color::Blue,
                    StartPosition::West.resolve(game_size),
                );

                let local_player_i = 0;
                let remote_player_i = 1;
                players_controlled_by_keyboard.push((wasd_controls, local_player_i));
                let (n, game_info) = Networking::host(
                    socket,
                    local_player_i,
                    remote_player_i,
                    local_player.direction,
                    frame,
                    game_size,
                    local_name,
                );
                networking = Some(n);

                let remote_player = Player::new(
                    game_info.remote_player_name,
                    Color::Green,
                    StartPosition::East.resolve(game_size),
                );
                players = vec![local_player, remote_player];
            }
            GameMode::Client(socket, local_name) => {
                let remote_player_i = 0;
                let local_player_i = 1;
                players_controlled_by_keyboard.push((wasd_controls, local_player_i));
                let local_start_pos = StartPosition::East;
                let (n, game_info) = Networking::join(
                    socket,
                    local_player_i,
                    remote_player_i,
                    local_start_pos.direction(),
                    frame,
                    local_name.clone(),
                );
                networking = Some(n);
                game_size = game_info.size;
                let remote_start_pos = StartPosition::West;

                players = vec![
                    Player::new(
                        game_info.remote_player_name,
                        Color::Blue,
                        remote_start_pos.resolve(game_size),
                    ),
                    Player::new(local_name, Color::Green, local_start_pos.resolve(game_size)),
                ];
            }
            GameMode::Offline => {
                game_size = suggested_game_size;
                players = vec![
                    Player::new(
                        "Mario".to_string(),
                        Color::Blue,
                        StartPosition::West.resolve(game_size),
                    ),
                    Player::new(
                        "Bowser".to_string(),
                        Color::Green,
                        StartPosition::East.resolve(game_size),
                    ),
                    Player::new(
                        "AI 1".to_string(),
                        Color::Magenta,
                        StartPosition::North.resolve(game_size),
                    ),
                    Player::new(
                        "AI 2".to_string(),
                        Color::Cyan,
                        StartPosition::South.resolve(game_size),
                    ),
                ];
                players_controlled_by_keyboard.push((wasd_controls, 0));
                players_controlled_by_keyboard.push((arrow_controls, 1));
                players_controlled_by_ai.push(2);
                players_controlled_by_ai.push(3);
                networking = None;
            }
        };

        let mut ui = TerminalUi::new(game_size, players.clone());
        ui.set_banner(Color::Yellow, "Go!");

        let game = Game::new(game_size, players, frame);

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
        Self::spawn_clock(sender.clone());
        Self::spawn_input_listener(sender.clone());

        if let Some(networking) = &mut self.networking {
            let result = networking.start_game(sender, slow_io);
            self.handle_net_result(result);
        }

        loop {
            self.ui.draw()?;

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
                                if let Some(direction) = controls.handle(code) {
                                    if let Some(networking) = &mut self.networking {
                                        let result = networking.set_direction(direction);
                                        self.handle_net_result(result);
                                    } else {
                                        self.game.players[player_i].direction = direction;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                },

                ThreadMessage::Network(event) => match event {
                    NetworkEvent::BufferedOutcomes => {
                        let networking = self.networking.as_mut().unwrap();
                        let outcomes = networking.take_buffered_outcomes();
                        self.execute_net_outcomes(outcomes);
                    }

                    NetworkEvent::ReceiveError(e) => {
                        panic!("Failed to receive from socket: {:?}", e);
                    }
                },

                ThreadMessage::Tick => {
                    if !self.game.game_over {
                        if let Some(networking) = self.networking.as_mut() {
                            let result = networking.commit_frame();
                            self.handle_net_result(result);
                        } else {
                            self.run_frame();
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

    fn handle_net_result(&mut self, result: NetResult<Vec<Outcome>>) {
        match result {
            Ok(outcomes) => {
                self.execute_net_outcomes(outcomes);
            }
            Err(error) => {
                panic!("Unexpected networking error: {:?}", error);
            }
        }
    }

    fn execute_net_outcomes(&mut self, outcomes: Vec<Outcome>) {
        for outcome in outcomes {
            match outcome {
                Outcome::PlayerControl(control) => {
                    self.game.players[control.player_i].direction = control.direction;
                }
                Outcome::RunFrame => {
                    self.run_frame();
                    let networking = self.networking.as_mut().unwrap();
                    let result = networking.start_new_frame(self.game.frame);
                    self.handle_net_result(result);
                }
                Outcome::RemoteLeft { politely } => {
                    let networking = self.networking.as_ref().unwrap();
                    let player_i = networking.remote_player_index();
                    let msg = if politely {
                        format!("{} left!", self.game.players[player_i].name)
                    } else {
                        "Disconnected!".to_string()
                    };
                    self.ui.set_banner(Color::Yellow, &msg);
                    self.game.game_over = true;
                }
            }
        }
    }

    fn run_frame(&mut self) {
        let frame_events = self.game.run_frame();

        for i in 0..self.game.players.len() {
            self.ui.set_player_line(i, &self.game.players[i].line);
            self.ui
                .set_player_direction(i, self.game.players[i].direction);
        }

        for event in frame_events {
            match event {
                FrameEvent::PlayerCrashed(i) => {
                    self.ui.set_banner(
                        Color::Yellow,
                        &format!("{} crashed!", self.game.players[i].name),
                    );
                    self.ui.set_player_crashed(i, true);
                }
                FrameEvent::PlayerWon(color, name) => {
                    self.ui.set_banner(color, &format!("{} won!", name));
                }
                FrameEvent::EveryoneCrashed => {
                    self.ui.set_banner(Color::Yellow, "Everyone crashed!");
                    for i in 0..self.game.players.len() {
                        self.ui.set_player_crashed(i, true);
                    }
                }
            }
        }

        for i in 0..self.game.players.len() {
            self.ui.set_player_score(i, self.game.players[i].score);
        }

        for i in 0..self.players_controlled_by_ai.len() {
            let player_i = self.players_controlled_by_ai[i];
            if !self.game.players[player_i].crashed {
                self.run_player_ai(player_i)
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
                    break;
                }
            }
        }
    }

    fn spawn_clock(sender: Sender<ThreadMessage>) {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(150));
            if sender.send(ThreadMessage::Tick).is_err() {
                // no receiver (i.e. main thread has exited)
                break;
            }
        });
    }

    fn spawn_input_listener(sender: Sender<ThreadMessage>) {
        thread::spawn(move || loop {
            let event = crossterm::event::read().expect("Receiving event");
            if sender.send(ThreadMessage::UserInput(event)).is_err() {
                // no receiver (i.e. main thread has exited)
                break;
            }
        });
    }
}

#[derive(Debug)]
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
