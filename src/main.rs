mod user_interface;

use std::cmp::{max, min};
use std::collections::HashMap;
use std::io::{self};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::{panic, thread};

use anyhow::Result;
use crossterm::event::Event::Key;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::Color;
use crossterm::terminal::{self};

use user_interface::TerminalUi;

pub type Point = (u16, u16);
type Direction = (i32, i32);

const UP: Direction = (0, -1);
const LEFT: Direction = (-1, 0);
const DOWN: Direction = (0, 1);
const RIGHT: Direction = (1, 0);

const DIRECTIONS: [Direction; 4] = [UP, LEFT, DOWN, RIGHT];

fn main() -> Result<()> {
    panic::set_hook(Box::new(move |panic_info| {
        let mut stdout = io::stdout();
        user_interface::restore_terminal(&mut stdout);
        eprintln!("Panic: >{:?}<", panic_info);
    }));

    let mut app = App::new()?;
    app.run()?;

    Ok(())
}

struct App {
    size: (u16, u16),
    game_over: bool,
    ui: TerminalUi,
    players: Vec<Player>,
}

impl App {
    pub fn new() -> Result<Self> {
        let (terminal_w, terminal_h) = terminal::size()?;
        let w = min(25, terminal_w);
        let h = max(terminal_h, 10);
        let size = (w, h);
        let mut ui = TerminalUi::new(size)?;

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
        Ok(Self {
            size,
            game_over: false,
            ui,
            players: vec![
                Player::new(
                    "P1".to_string(),
                    Color::Blue,
                    PlayerControls::Keyboard(wasd_controls),
                    west,
                ),
                Player::new(
                    "P2".to_string(),
                    Color::Green,
                    PlayerControls::Keyboard(arrow_controls),
                    east,
                ),
                Player::new("AI 1".to_string(), Color::Red, PlayerControls::Ai, top),
                Player::new("AI 2".to_string(), Color::Cyan, PlayerControls::Ai, bot),
            ],
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let (sender, receiver) = mpsc::channel();
        Self::spawn_periodic_timer(sender.clone());
        Self::spawn_event_receiver(sender);

        loop {
            self.ui.draw_background()?;

            for player in &self.players {
                self.ui.draw_colored_line(player.color, &player.line)?;
            }

            self.ui.flush()?;

            match receiver.recv()? {
                Message::Event(event) => match event {
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
                        for player in &mut self.players {
                            if !player.crashed {
                                if let PlayerControls::Keyboard(controls) = &player.controls {
                                    if let Some(dir) = controls.handle(code) {
                                        player.direction = dir;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                },

                Message::Tick => {
                    if !self.game_over {
                        for player in &mut self.players {
                            if !player.crashed {
                                player.advance_one_step();
                            }
                        }

                        for i in 0..self.players.len() {
                            if !self.players[i].crashed && self.is_player_crashing(i) {
                                self.players[i].crashed = true;
                                self.ui.set_banner(
                                    Color::Yellow,
                                    format!("{} crashed!", self.players[i].name),
                                );
                            }
                        }

                        let mut survivors = self.players.iter().filter(|p| !p.crashed);
                        if let Some(survivor) = survivors.next() {
                            if survivors.next().is_none() {
                                self.ui
                                    .set_banner(survivor.color, format!("{} won!", survivor.name));
                                self.game_over = true;
                            }
                        } else {
                            self.ui
                                .set_banner(Color::Yellow, "Everyone crashed!".to_string());
                            self.game_over = true;
                        }

                        for i in 0..self.players.len() {
                            if !self.players[i].crashed {
                                if let PlayerControls::Ai = &self.players[i].controls {
                                    self.run_player_ai(i)
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn run_player_ai(&mut self, player_index: usize) {
        let ai_head = self.players[player_index].head();
        if !self.is_vacant(translated(ai_head, self.players[player_index].direction)) {
            for dir in DIRECTIONS {
                if self.is_vacant(translated(ai_head, dir)) {
                    self.players[player_index].direction = dir;
                }
            }
        }
    }

    fn spawn_periodic_timer(sender: Sender<Message>) {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(150));
            if sender.send(Message::Tick).is_err() {
                // no receiver (i.e. main thread has exited)
                break;
            }
        });
    }

    fn spawn_event_receiver(sender: Sender<Message>) {
        thread::spawn(move || loop {
            let event = crossterm::event::read().expect("Receiving event");
            if sender.send(Message::Event(event)).is_err() {
                // no receiver (i.e. main thread has exited)
                break;
            }
        });
    }

    fn is_player_crashing(&self, player_index: usize) -> bool {
        let head = self.players[player_index].head();
        if !self.is_within_game_bounds(head) {
            return true;
        }

        for (i, player) in self.players.iter().enumerate() {
            let obstacle = if i == player_index {
                // A player can not be crashing with its own head
                player.tail()
            } else {
                player.full_body()
            };
            if obstacle.contains(&head) {
                return true;
            }
        }
        false
    }

    fn is_vacant(&self, point: Point) -> bool {
        if !self.is_within_game_bounds(point) {
            return false;
        }
        for player in &self.players {
            if player.full_body().contains(&point) {
                return false;
            }
        }
        true
    }

    fn is_within_game_bounds(&self, point: Point) -> bool {
        point.0 > 0 && point.1 > 0 && point.0 < self.size.0 - 1 && point.1 < self.size.1 - 1
    }
}

struct Player {
    name: String,
    color: Color,
    controls: PlayerControls,
    line: Vec<Point>,
    direction: Direction,
    crashed: bool,
}

enum PlayerControls {
    Keyboard(KeyboardControls),
    Ai,
}

impl Player {
    fn new(
        name: String,
        color: Color,
        controls: PlayerControls,
        start_position: (Point, Direction),
    ) -> Self {
        Self {
            name,
            color,
            controls,
            line: vec![start_position.0],
            direction: start_position.1,
            crashed: false,
        }
    }

    fn advance_one_step(&mut self) {
        self.line.push(self.next_position());
    }

    fn next_position(&self) -> Point {
        translated(self.head(), self.direction)
    }

    fn head(&self) -> Point {
        *self.line.last().unwrap()
    }

    fn full_body(&self) -> &[Point] {
        &self.line[..]
    }

    fn tail(&self) -> &[Point] {
        &self.line[..self.line.len() - 1]
    }
}

fn translated(point: Point, direction: Direction) -> Point {
    (
        (point.0 as i32 + direction.0) as u16,
        (point.1 as i32 + direction.1) as u16,
    )
}

enum Message {
    Event(Event),
    Tick,
}

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
