use crate::game::{Player, PlayerIndex, DOWN, LEFT, RIGHT, UP};
use crate::{game, Point};
use backtrace::Backtrace;
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, ClearType};
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use std::cmp::min;
use std::io::Stdout;
use std::io::Write;
use std::{io, panic};
use tui::backend::CrosstermBackend;
use tui::buffer::Buffer;
use tui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Widget};
use tui::Terminal;

pub struct TerminalUi {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    game_size: (u16, u16),
    players: Vec<Player>,
    banner_text: String,
    banner_color: Color,
}

impl TerminalUi {
    pub fn new(game_size: (u16, u16), players: Vec<Player>) -> Self {
        let stdout = io::stdout();
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout)).unwrap();

        setup_panic_handler();
        claim_terminal(&mut terminal);

        Self {
            terminal,
            game_size,
            players,
            banner_text: Default::default(),
            banner_color: Color::White,
        }
    }

    pub fn set_player_line(&mut self, player_i: PlayerIndex, line: &[Point]) {
        self.players[player_i].line.clear();
        self.players[player_i].line.extend_from_slice(line);
    }

    pub fn set_player_direction(&mut self, player_i: PlayerIndex, direction: game::Direction) {
        self.players[player_i].direction = direction;
    }

    pub fn set_player_crashed(&mut self, player_i: PlayerIndex, crashed: bool) {
        self.players[player_i].crashed = crashed;
    }

    pub fn set_player_score(&mut self, player_i: PlayerIndex, score: u32) {
        self.players[player_i].score = score;
    }

    pub fn set_banner(&mut self, color: Color, text: &str) {
        self.banner_text.clear();
        self.banner_text.push_str(text);
        self.banner_color = color;
    }

    pub fn draw(&mut self) -> anyhow::Result<()> {
        self.terminal
            .draw(|frame| {
                let game_container = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " Achtung ({}x{})",
                        self.game_size.0, self.game_size.1
                    ))
                    .border_type(BorderType::Rounded);

                let game_container_padding = Margin {
                    vertical: 1,
                    horizontal: 1,
                };
                let desired_banner_height = 2;
                let desired_game_container_size = (
                    self.game_size.0 + game_container_padding.horizontal * 2,
                    self.game_size.1 + game_container_padding.vertical * 2 + desired_banner_height,
                );

                let horizontal_rects = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(
                        [
                            Constraint::Length(desired_game_container_size.0),
                            Constraint::Min(0),
                        ]
                        .as_ref(),
                    )
                    .split(frame.size());

                let mut game_container_rect = horizontal_rects[0];
                game_container_rect.height =
                    min(desired_game_container_size.1, game_container_rect.height);

                let mut sidebar_rect = horizontal_rects[1];
                sidebar_rect.width = min(sidebar_rect.width, 20);
                sidebar_rect.height = min(sidebar_rect.height, (self.players.len() + 2) as u16);

                let game_container_sub_rects = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(
                        [
                            Constraint::Length(desired_banner_height),
                            Constraint::Min(0),
                        ]
                        .as_ref(),
                    )
                    .split(game_container_rect.inner(&game_container_padding));

                let banner = Paragraph::new(&self.banner_text[..])
                    .style(
                        Style::default()
                            .fg(self.banner_color)
                            .add_modifier(Modifier::BOLD),
                    )
                    .alignment(Alignment::Center);
                let banner_container = Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default())
                    .border_type(BorderType::Double);
                let banner_container_rect = game_container_sub_rects[0];
                let mut banner_rect = banner_container_rect;
                banner_rect.height = min(banner_rect.height, 1);

                let game = GameWidget(&self.players);
                let game_rect = game_container_sub_rects[1];

                let sidebar_items: Vec<ListItem> = self
                    .players
                    .iter()
                    .map(|p| {
                        let name_part =
                            format!("{}| {}", if p.crashed { "!" } else { " " }, p.name);
                        let score_part = format!("{} ", p.score);
                        let spaces = " ".repeat(
                            (sidebar_rect.width as usize)
                                .saturating_sub(2 + name_part.len() + score_part.len()),
                        );
                        ListItem::new(format!("{}{}{}", name_part, spaces, score_part))
                            .style(Style::default().fg(p.color))
                    })
                    .collect();
                let sidebar = List::new(sidebar_items).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded),
                );

                frame.render_widget(game_container, game_container_rect);
                frame.render_widget(banner_container, banner_container_rect);
                frame.render_widget(banner, banner_rect);

                frame.render_widget(game, game_rect);
                frame.render_widget(sidebar, sidebar_rect);
            })
            .unwrap();

        Ok(())
    }
}

struct GameWidget<'a>(&'a [Player]);

impl Widget for GameWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for player in self.0 {
            for i in 0..player.line.len() {
                let point = player.line[i];
                let x = (area.x as i32 + point.0) as u16;
                let y = (area.y as i32 + point.1) as u16;
                if x <= area.right() && y <= area.bottom() {
                    let cell = buf.get_mut(x, y);
                    cell.fg = player.color;
                    let symbol = if i < player.line.len() - 1 {
                        "#"
                    } else {
                        match player.direction {
                            UP => "^",
                            LEFT => "<",
                            DOWN => "v",
                            RIGHT => ">",
                            bad_direction => panic!("Invalid direction: {:?}", bad_direction),
                        }
                    };
                    cell.set_symbol(symbol);
                }
            }
        }
        for player_line in self.0 {
            if player_line.crashed {
                let point = player_line.line.last().unwrap();
                let x = (area.x as i32 + point.0) as u16;
                let y = (area.y as i32 + point.1) as u16;
                if x <= area.right() && y <= area.bottom() {
                    let cell = buf.get_mut(x, y);
                    cell.fg = Color::Red;
                    cell.set_symbol("X");
                }
            }
        }
    }
}

impl Drop for TerminalUi {
    fn drop(&mut self) {
        restore_terminal(&mut self.terminal);
    }
}

fn claim_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    enable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), EnterAlternateScreen).unwrap();
    terminal.hide_cursor().unwrap();
}

pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen,).unwrap();
    terminal.show_cursor().unwrap();
}

fn setup_panic_handler() {
    panic::set_hook(Box::new(move |panic_info| {
        io::stdout().flush().unwrap();
        execute!(io::stdout(), crossterm::terminal::Clear(ClearType::All)).unwrap();
        execute!(io::stdout(), LeaveAlternateScreen).unwrap();
        disable_raw_mode().unwrap();

        println!("Panic backtrace: >{:?}<", Backtrace::new());
        println!("Panic: >{:?}<", panic_info);
    }));
}
