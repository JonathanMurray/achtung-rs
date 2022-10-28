use anyhow::Result;
use crossterm::cursor::{Hide, MoveTo};
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::QueueableCommand;
use std::cmp::min;
use std::io::{self, Stdout, Write};

use crate::Point;

pub struct TerminalUi {
    stdout: Stdout,
    size: (u16, u16),
    banner: String,
    banner_color: Color,
}

impl TerminalUi {
    pub fn new(size: (u16, u16)) -> Result<Self> {
        let mut stdout = io::stdout();
        claim_terminal(&mut stdout)?;

        Ok(Self {
            stdout,
            size,
            banner: String::new(),
            banner_color: Color::White,
        })
    }

    pub fn draw_background(&mut self) -> Result<()> {
        let (w, h) = self.size;
        self.stdout.queue(MoveTo(0, 0))?;

        for y in 0..h {
            if y == 0 {
                let banner = &self.banner[0..min(w as usize - 6, self.banner.len())];
                self.stdout.write_all("+-- ".as_bytes())?;
                self.stdout.queue(SetForegroundColor(self.banner_color))?;
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

    pub fn draw_colored_line(&mut self, color: Color, line: &[Point]) -> Result<()> {
        self.stdout.queue(SetForegroundColor(color))?;
        for point in line {
            self.draw_point(*point)?;
        }
        Ok(())
    }

    fn draw_point(&mut self, point: Point) -> Result<()> {
        self.stdout.queue(MoveTo(point.0, point.1))?;
        self.stdout.write_all("#".as_bytes())?;
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
}

impl Drop for TerminalUi {
    fn drop(&mut self) {
        restore_terminal(&mut self.stdout);
    }
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
