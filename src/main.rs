use anyhow::Result;

use crossterm::cursor::{Hide, MoveTo};
use crossterm::event::Event::Key;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::QueueableCommand;
use std::cmp::{max, min};
use std::io::{self, Stdout, Write};
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::{panic, thread};

fn main() -> Result<()> {
    panic::set_hook(Box::new(move |panic_info| {
        let mut stdout = io::stdout();
        restore_terminal(&mut stdout);
        eprintln!("Panic: >{:?}<", panic_info);
    }));

    let mut app = App::new()?;
    app.run()?;

    Ok(())
}

struct App {
    size: (u16, u16),
    game_over: bool,
    stdout: Stdout,
    banner: String,
    human: Player,
    ai: Player,
}

impl App {
    pub fn new() -> Result<Self> {
        let mut stdout = io::stdout();
        claim_terminal(&mut stdout)?;
        let (terminal_w, terminal_h) = terminal::size()?;
        let w = min(30, terminal_w);
        let h = max(terminal_h, 10);

        Ok(Self {
            size: (w, h),
            game_over: false,
            stdout,
            banner: "ACHTUNG!".to_string(),
            human: Player::new((1, h / 2), (1, 0)),
            ai: Player::new((w - 2, h / 2), (-1, 0)),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let (sender, receiver) = mpsc::channel();
        Self::spawn_periodic_timer(sender.clone());
        Self::spawn_event_receiver(sender);

        loop {
            self.draw_border()?;

            self.stdout.queue(SetForegroundColor(Color::Blue))?;
            for point in &self.human.line {
                self.stdout.queue(MoveTo(point.0, point.1))?;
                self.stdout.write_all("X".as_bytes())?;
            }
            self.stdout.queue(SetForegroundColor(Color::Red))?;
            for point in &self.ai.line {
                self.stdout.queue(MoveTo(point.0, point.1))?;
                self.stdout.write_all("X".as_bytes())?;
            }
            self.stdout.queue(ResetColor)?;

            self.stdout.flush()?;

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
                    }) if key_direction(code).is_some() => {
                        self.human.direction = key_direction(code).unwrap();
                    }
                    _ => {}
                },

                Message::Tick => {
                    if self.game_over {
                        continue;
                    } else if !self.is_vacant(self.human.next_position()) {
                        self.game_over = true;
                        self.banner = "YOU LOST!".to_string();
                    } else if !self.is_vacant(self.ai.next_position()) {
                        self.game_over = true;
                        self.banner = "YOU WON!".to_string();
                    } else {
                        self.human.advance_one_step();
                        self.ai.advance_one_step();

                        let ai_head = self.ai.head();
                        if !self.is_vacant(translated(ai_head, self.ai.direction)) {
                            for dir in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
                                if self.is_vacant(translated(ai_head, dir)) {
                                    self.ai.direction = dir;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
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

    fn is_vacant(&self, point: (u16, u16)) -> bool {
        self.is_within_game_bounds(point) && !self.human.contains(point) && !self.ai.contains(point)
    }

    fn is_within_game_bounds(&self, point: (u16, u16)) -> bool {
        point.0 > 0 && point.1 > 0 && point.0 < self.size.0 - 1 && point.1 < self.size.1 - 1
    }

    fn draw_border(&mut self) -> Result<()> {
        let (w, h) = self.size;
        self.stdout.queue(MoveTo(0, 0))?;

        for y in 0..h {
            if y == 0 {
                let banner = &self.banner[0..min(w as usize - 6, self.banner.len())];
                self.stdout.write_all("+-- ".as_bytes())?;
                self.stdout.queue(SetForegroundColor(Color::Yellow))?;
                self.stdout.write_all(format!("{} ", banner).as_bytes())?;
                self.stdout.queue(ResetColor)?;
                self.stdout.write_all(
                    "-".repeat((w - 6 - banner.len() as u16) as usize)
                        .as_bytes(),
                )?;
                self.stdout.write_all("+".as_bytes())?;
            } else if y == h - 1 {
                self.stdout.write_all("+-- press q to exit ".as_bytes())?;
                self.stdout
                    .write_all("-".repeat((w - 21) as usize).as_bytes())?;
                self.stdout.write_all("+".as_bytes())?;
            } else {
                self.stdout.write_all("|".as_bytes())?;
                self.stdout
                    .write_all(" ".repeat((w - 2) as usize).as_bytes())?;
                self.stdout.write_all("|".as_bytes())?;
            }
            if y < h - 1 {
                self.stdout.write_all("\n".as_bytes())?;
            }
        }

        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        restore_terminal(&mut self.stdout);
    }
}

struct Player {
    line: Vec<(u16, u16)>,
    direction: (i32, i32),
}

impl Player {
    fn new(pos: (u16, u16), direction: (i32, i32)) -> Self {
        Self {
            line: vec![pos],
            direction,
        }
    }

    fn advance_one_step(&mut self) {
        self.line.push(self.next_position());
    }

    fn next_position(&self) -> (u16, u16) {
        translated(self.head(), self.direction)
    }

    fn head(&self) -> (u16, u16) {
        *self.line.last().unwrap()
    }

    fn contains(&self, point: (u16, u16)) -> bool {
        self.line.contains(&point)
    }
}

fn translated(pos: (u16, u16), direction: (i32, i32)) -> (u16, u16) {
    (
        (pos.0 as i32 + direction.0) as u16,
        (pos.1 as i32 + direction.1) as u16,
    )
}

enum Message {
    Event(Event),
    Tick,
}

fn key_direction(pressed_key_code: KeyCode) -> Option<(i32, i32)> {
    match pressed_key_code {
        KeyCode::Left => Some((-1, 0)),
        KeyCode::Right => Some((1, 0)),
        KeyCode::Up => Some((0, -1)),
        KeyCode::Down => Some((0, 1)),
        _ => None,
    }
}

fn claim_terminal(stdout: &mut Stdout) -> Result<()> {
    terminal::enable_raw_mode()?;
    stdout.queue(EnterAlternateScreen)?;
    stdout.queue(Hide)?;
    stdout.flush()?;
    Ok(())
}

fn restore_terminal(stdout: &mut Stdout) {
    stdout.queue(LeaveAlternateScreen).unwrap();
    stdout.flush().unwrap();
    terminal::disable_raw_mode().unwrap();
}
