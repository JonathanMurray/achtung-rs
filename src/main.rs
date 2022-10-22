use crossterm::Result;

use crossterm::cursor::{MoveTo, MoveToColumn, MoveToRow};
use crossterm::event::Event::Key;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::QueueableCommand;
use std::io::{self, Stdout, Write};

fn main() -> Result<()> {
    let mut app = App::new()?;
    app.run()?;

    Ok(())
}

struct App {
    stdout: Stdout,
}

impl App {
    pub fn new() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.queue(EnterAlternateScreen)?;
        stdout.flush()?;
        Ok(Self { stdout })
    }

    pub fn run(&mut self) -> Result<()> {
        self.stdout.queue(EnterAlternateScreen)?;
        self.stdout.flush()?;

        self.draw_border()?;
        self.stdout.flush()?;

        let mut input = String::new();

        loop {
            input.clear();
            let event = crossterm::event::read()?;

            match event {
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
                _ => {}
            }

            self.draw_border()?;
            self.stdout.flush()?;

            self.stdout.queue(MoveToRow(1))?;
            self.stdout.queue(MoveToColumn(2))?;
            self.stdout
                .write_all(format!("EVENT: {:?}", event).as_bytes())?;
            self.stdout.flush()?;
        }

        self.stdout.flush()?;

        Ok(())
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
        self.stdout.queue(LeaveAlternateScreen).unwrap();
        self.stdout.flush().unwrap();
        terminal::disable_raw_mode().unwrap();
    }
}
