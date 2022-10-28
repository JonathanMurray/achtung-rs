use anyhow::Result;

use crossterm::cursor::{Hide, MoveTo, MoveToColumn, MoveToRow};
use crossterm::event::Event::Key;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::QueueableCommand;
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
    stdout: Stdout,
    unhandled_event: String,
    elapsed_ticks: u32,
    line: Vec<(u16, u16)>,
    direction: (i32, i32),
}

impl App {
    pub fn new() -> Result<Self> {
        let mut stdout = io::stdout();
        claim_terminal(&mut stdout)?;
        Ok(Self {
            stdout,
            unhandled_event: String::new(),
            elapsed_ticks: 0,
            line: vec![(2, 5)],
            direction: (1, 0),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let (sender, receiver) = mpsc::channel();
        Self::spawn_periodic_timer(sender.clone());
        Self::spawn_event_receiver(sender);

        loop {
            self.draw_border()?;

            self.stdout.queue(MoveToRow(1))?;
            self.stdout.queue(MoveToColumn(2))?;
            self.stdout
                .write_all(format!("Event: {:?}", self.unhandled_event).as_bytes())?;

            self.stdout.queue(MoveToRow(2))?;
            self.stdout.queue(MoveToColumn(2))?;
            self.stdout
                .write_all(format!("Elapsed: {:?}", self.elapsed_ticks).as_bytes())?;

            self.stdout.queue(SetForegroundColor(Color::Red))?;
            for point in &self.line {
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
                        self.direction = key_direction(code).unwrap()
                    }
                    event => self.unhandled_event = format!("Event: {:?}", event),
                },

                Message::Tick => {
                    self.elapsed_ticks += 1;
                    let head = self.line.last().unwrap();
                    self.line.push((
                        (head.0 as i32 + self.direction.0) as u16,
                        (head.1 as i32 + self.direction.1) as u16,
                    ));
                }
            }
        }

        Ok(())
    }

    fn spawn_periodic_timer(sender: Sender<Message>) {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(300));
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

    fn draw_border(&mut self) -> Result<()> {
        let (w, h) = terminal::size()?;
        self.stdout.queue(MoveTo(0, 0))?;

        for y in 0..h {
            if y == 0 {
                self.stdout.write_all("+".as_bytes())?;
                self.stdout
                    .write_all("-".repeat((w - 2) as usize).as_bytes())?;
                self.stdout.write_all("+".as_bytes())?;
            } else if y == h - 1 {
                self.stdout.write_all("+-- press q to exit  ".as_bytes())?;
                self.stdout
                    .write_all("-".repeat((w - 22) as usize).as_bytes())?;
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

enum Message {
    Event(Event),
    Tick,
}

fn key_direction(pressed_key_code: KeyCode) -> Option<(i32, i32)> {
    return match pressed_key_code {
        KeyCode::Left => Some((-1, 0)),
        KeyCode::Right => Some((1, 0)),
        KeyCode::Up => Some((0, -1)),
        KeyCode::Down => Some((0, 1)),
        _ => None,
    };
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
