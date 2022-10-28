use crate::game::{Direction, PlayerIndex, DOWN, LEFT, RIGHT, UP};
use anyhow::Result;
use crossterm::cursor::{Hide, MoveTo};
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::QueueableCommand;
use std::cmp::min;
use std::io::{self, Stdout, Write};

use crate::Point;

const SIDE_BAR_WIDTH: u16 = 16;
const BANNER_HEIGHT: u16 = 2;

pub struct TerminalUi {
    stdout: Stdout,
    ui_size: (u16, u16),
    game_size: (u16, u16),
    game_offset: (u16, u16),
    banner: String,
    banner_color: Color,
    player_labels: Vec<PlayerLabel>,
    frame: String,
}

impl TerminalUi {
    pub fn new(game_size: (u16, u16), frame: u32, player_labels: Vec<PlayerLabel>) -> Result<Self> {
        let mut stdout = io::stdout();
        claim_terminal(&mut stdout)?;

        let game_offset = (1, 3);
        Ok(Self {
            stdout,
            ui_size: (
                game_size.0 + game_offset.0 + 1 + SIDE_BAR_WIDTH,
                game_size.1 + game_offset.1 + 1,
            ),
            game_size,
            game_offset,
            banner: String::new(),
            banner_color: Color::White,
            player_labels,
            frame: format!("{}", frame),
        })
    }

    pub fn compute_game_size(ui_size: (u16, u16)) -> (u16, u16) {
        (
            ui_size.0 - 2 - SIDE_BAR_WIDTH,
            ui_size.1 - 2 - BANNER_HEIGHT,
        )
    }

    pub fn set_player_score(&mut self, player_i: PlayerIndex, score: u32) {
        self.player_labels[player_i].score = score;
    }

    pub fn set_player_crashed(&mut self, player_i: PlayerIndex, crashed: bool) {
        self.player_labels[player_i].crashed = crashed;
    }

    pub fn set_player_disconnected(&mut self, player_i: PlayerIndex, disconnected: bool) {
        self.player_labels[player_i].disconnected = disconnected;
    }

    pub fn draw_background(&mut self) -> Result<()> {
        let (ui_w, ui_h) = self.ui_size;
        let (game_w, _game_h) = self.game_size;

        for y in 0..ui_h {
            self.stdout.queue(MoveTo(0, y))?;
            if y == 0 {
                self.draw_horizontal_line(game_w)?;
            } else if y == 1 {
                self.draw("| ")?;
                let banner = &self.banner[0..min(self.banner.len(), game_w as usize - 2)];
                self.stdout.queue(SetForegroundColor(self.banner_color))?;
                self.stdout.write_all(format!("{} ", banner).as_bytes())?;
                self.stdout.queue(ResetColor)?;
                self.stdout.write_all(
                    " ".repeat((game_w - 2 - banner.len() as u16) as usize)
                        .as_bytes(),
                )?;
                self.stdout.write_all("|".as_bytes())?;
            } else if y == 2 || y == ui_h - 1 {
                self.draw_horizontal_line(game_w)?;
            } else {
                self.draw("|")?;
                self.draw(&" ".repeat(game_w as usize))?;
                self.draw("|")?;
            }
        }

        let side_bar_w = ui_w - game_w - 2;
        let side_bar_x = self.game_offset.0 + self.game_size.0;
        for y in 0..ui_h {
            self.stdout.queue(MoveTo(side_bar_x, y))?;
            if y % 2 == 0 {
                let next_label_index = (y / 2) as usize;
                if self.player_labels.len() >= next_label_index {
                    self.draw("+-")?;
                    self.draw_horizontal_line(side_bar_w - 1)?;
                }
            } else if let Some(label) = self.player_labels.get((y / 2) as usize) {
                let PlayerLabel {
                    name,
                    color,
                    score,
                    crashed,
                    disconnected,
                } = label;
                let main_text = if *disconnected { "(offline)" } else { name };
                let main_text = &main_text[0..min(main_text.len(), side_bar_w as usize - 5)];
                self.stdout.write_all("|".as_bytes())?;
                if *crashed {
                    self.stdout.queue(SetForegroundColor(Color::DarkRed))?;
                    self.stdout.write_all("!".as_bytes())?;
                    self.stdout.queue(ResetColor)?;
                } else {
                    self.stdout.write_all(" ".as_bytes())?;
                }
                self.stdout.write_all("| ".as_bytes())?;
                if *disconnected {
                    self.stdout.write_all(main_text.as_bytes())?;
                } else {
                    self.stdout.queue(SetForegroundColor(*color))?;
                    self.stdout.write_all(main_text.as_bytes())?;
                    self.stdout.queue(ResetColor)?;
                }

                let score = format!("{}", score);
                self.stdout.write_all(
                    " ".repeat(
                        (side_bar_w - 3 - main_text.len() as u16 - score.len() as u16) as usize,
                    )
                    .as_bytes(),
                )?;
                self.stdout.write_all(score.as_bytes())?;
                self.stdout.write_all(" |".as_bytes())?;
            }
        }

        Ok(())
    }

    fn draw_horizontal_line(&mut self, w: u16) -> Result<()> {
        self.draw("+")?;
        self.draw(&"-".repeat(w as usize))?;
        self.draw("+")?;
        Ok(())
    }

    fn draw(&mut self, text: &str) -> Result<()> {
        self.stdout.write_all(text.as_bytes())?;
        Ok(())
    }

    pub fn draw_player_line(
        &mut self,
        color: Color,
        line: &[Point],
        direction: Direction,
    ) -> Result<()> {
        self.stdout.queue(SetForegroundColor(color))?;
        for i in 0..line.len() {
            let char = if i < line.len() - 1 {
                '#'
            } else {
                match direction {
                    UP => '^',
                    LEFT => '<',
                    DOWN => 'v',
                    RIGHT => '>',
                    _ => panic!("Invalid direction: {:?}", direction),
                }
            };
            self.draw_game_point(line[i], char as u8)?;
        }
        Ok(())
    }

    pub fn draw_crash(&mut self, point: Point) -> Result<()> {
        self.stdout.queue(SetForegroundColor(Color::DarkRed))?;
        self.draw_game_point(point, b'@')
    }

    fn draw_game_point(&mut self, point: Point, char: u8) -> Result<()> {
        self.draw_point(
            (
                (point.0 + self.game_offset.0 as i32) as u16,
                (point.1 + self.game_offset.1 as i32) as u16,
            ),
            char,
        )
    }

    fn draw_point(&mut self, point: (u16, u16), char: u8) -> Result<()> {
        self.stdout.queue(MoveTo(point.0, point.1))?;
        self.stdout.write_all(&[char])?;
        Ok(())
    }

    pub fn set_banner(&mut self, color: Color, banner: String) {
        self.banner_color = color;
        self.banner = banner;
    }

    pub fn flush(&mut self) -> Result<()> {
        self.stdout.queue(ResetColor)?;
        self.stdout.flush()?;
        Ok(())
    }

    pub fn set_frame(&mut self, frame: u32) {
        self.frame = format!("{}", frame);
    }
}

impl Drop for TerminalUi {
    fn drop(&mut self) {
        restore_terminal(&mut self.stdout);
    }
}

#[derive(Debug, Clone)]
pub struct PlayerLabel {
    pub name: String,
    pub color: Color,
    pub score: u32,
    pub crashed: bool,
    pub disconnected: bool,
}

fn claim_terminal(stdout: &mut Stdout) -> Result<()> {
    terminal::enable_raw_mode()?;
    stdout.queue(EnterAlternateScreen)?;
    stdout.queue(Hide)?;
    stdout.flush()?;
    Ok(())
}

pub fn restore_terminal(stdout: &mut Stdout) {
    stdout.queue(LeaveAlternateScreen).unwrap();
    stdout.queue(ResetColor).unwrap();
    stdout.flush().unwrap();
    terminal::disable_raw_mode().unwrap();
}
