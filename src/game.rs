use crate::Point;
use crossterm::style::Color;

pub type PlayerIndex = usize;
pub type Direction = (i32, i32);

pub const UP: Direction = (0, -1);
pub const LEFT: Direction = (-1, 0);
pub const DOWN: Direction = (0, 1);
pub const RIGHT: Direction = (1, 0);

pub const DIRECTIONS: [Direction; 4] = [UP, LEFT, DOWN, RIGHT];

pub struct Game {
    size: (u16, u16),
    pub game_over: bool,
    pub players: Vec<Player>,
    pub frame: u32,
}

impl Game {
    pub fn new(size: (u16, u16), players: Vec<Player>, frame: u32) -> Self {
        Self {
            size,
            game_over: false,
            players,
            frame,
        }
    }

    pub fn run_frame(&mut self) -> Option<FrameReport> {
        let mut report = None;
        for player in &mut self.players {
            if !player.crashed {
                player.advance_one_step();
            }
        }

        for i in 0..self.players.len() {
            if !self.players[i].crashed && self.is_player_crashing(i) {
                self.players[i].crashed = true;
                report = Some(FrameReport::PlayerCrashed(i));
            }
        }

        let mut survivors = self.players.iter().filter(|p| !p.crashed);
        if let Some(survivor) = survivors.next() {
            if survivors.next().is_none() {
                report = Some(FrameReport::PlayerWon(
                    survivor.color,
                    survivor.name.clone(),
                ));
                self.game_over = true;
            }
        } else {
            report = Some(FrameReport::EveryoneCrashed);
            self.game_over = true;
        }

        self.frame += 1;

        report
    }

    fn is_within_game_bounds(&self, point: Point) -> bool {
        point.0 > 0 && point.1 > 0 && point.0 < self.size.0 - 1 && point.1 < self.size.1 - 1
    }

    pub fn is_vacant(&self, point: Point) -> bool {
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

    fn is_player_crashing(&self, player_index: PlayerIndex) -> bool {
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
}

#[derive(Debug)]
pub enum FrameReport {
    PlayerCrashed(PlayerIndex),
    PlayerWon(Color, String),
    EveryoneCrashed,
}

pub struct Player {
    pub name: String,
    pub color: Color,
    pub line: Vec<Point>,
    pub direction: Direction,
    pub crashed: bool,
}

impl Player {
    pub fn new(name: String, color: Color, start_position: (Point, Direction)) -> Self {
        Self {
            name,
            color,
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

    pub fn head(&self) -> Point {
        *self.line.last().unwrap()
    }

    fn full_body(&self) -> &[Point] {
        &self.line[..]
    }

    fn tail(&self) -> &[Point] {
        &self.line[..self.line.len() - 1]
    }
}

pub fn translated(point: Point, direction: Direction) -> Point {
    (
        (point.0 as i32 + direction.0) as u16,
        (point.1 as i32 + direction.1) as u16,
    )
}
