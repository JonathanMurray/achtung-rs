use crate::app::ThreadMessage;
use crate::game::{Direction, PlayerIndex, DOWN, LEFT, RIGHT, UP};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

pub struct Networking {
    socket: TcpStream,
    player: PlayerIndex,
    remote_player: PlayerIndex,
    player_direction: Direction,
    frame: u32,
    queued_command_from_remote: Option<Direction>,
    has_remote_committed_frame: bool,
    has_remote_committed_next_frame: bool,
    has_committed_frame: bool,
    queued_command: Option<Direction>,
}

impl Networking {
    pub fn new(
        socket: TcpStream,
        local_player: PlayerIndex,
        remote_player: PlayerIndex,
        player_direction: Direction,
        frame: u32,
    ) -> Self {
        Self {
            socket,
            player: local_player,
            remote_player,
            player_direction,
            frame,
            queued_command_from_remote: None,
            has_remote_committed_frame: false,
            has_remote_committed_next_frame: false,
            has_committed_frame: false,
            queued_command: None,
        }
    }

    pub fn start_game(&mut self) -> NetResult<()> {
        self.send_direction_command(self.player_direction)
    }

    pub fn start_new_frame(&mut self, frame: u32) -> NetResult<Vec<SetDirectionCommand>> {
        let mut commands = vec![];

        self.frame = frame;
        self.has_committed_frame = false;
        self.has_remote_committed_frame = false;

        if let Some(dir) = self.queued_command.take() {
            self.player_direction = dir;
            commands.push(SetDirectionCommand {
                player_i: self.player,
                direction: dir,
            });
        }
        self.send_direction_command(self.player_direction)?;

        if let Some(dir) = self.queued_command_from_remote.take() {
            commands.push(SetDirectionCommand {
                player_i: self.remote_player,
                direction: dir,
            });
        }

        if self.has_remote_committed_next_frame {
            self.has_remote_committed_frame = true;
            self.has_remote_committed_next_frame = false;
        }

        Ok(commands)
    }

    pub fn set_direction(&mut self, direction: Direction) -> NetResult<bool> {
        if self.has_committed_frame {
            self.queued_command = Some(direction);
            Ok(false)
        } else {
            self.player_direction = direction;
            self.send_direction_command(direction)?;
            Ok(true)
        }
    }

    pub fn commit_frame(&mut self) -> NetResult<()> {
        if !self.has_committed_frame {
            self.send_net_packet(NetworkPacket::CommitFrame(NetworkPacket::modulo(
                self.frame,
            )))?;
        }
        self.has_committed_frame = true;
        Ok(())
    }

    pub fn handle_event(&mut self, event: NetworkEvent) -> EventOutcome {
        match event {
            NetworkEvent::Received(packet) => match packet {
                NetworkPacket::SetDirection(pkt) => {
                    if pkt.frame_modulo == NetworkPacket::modulo(self.frame) {
                        assert!(!self.has_remote_committed_frame);
                        return EventOutcome::SetDirection(SetDirectionCommand {
                            player_i: self.remote_player,
                            direction: pkt.direction,
                        });
                    } else if pkt.frame_modulo == NetworkPacket::modulo(self.frame + 1) {
                        assert!(!self.has_remote_committed_next_frame);
                        self.queued_command_from_remote = Some(pkt.direction);
                    } else {
                        panic!(
                            "Received command with unexpected frame modulo: {:?}. Our frame: {}",
                            pkt, self.frame
                        );
                    }
                }

                NetworkPacket::CommitFrame(frame_modulo) => {
                    if frame_modulo == NetworkPacket::modulo(self.frame) {
                        self.has_remote_committed_frame = true;
                    } else if frame_modulo == NetworkPacket::modulo(self.frame + 1) {
                        self.has_remote_committed_next_frame = true;
                    } else {
                        panic!(
                            "Received commit with unexpected frame modulo: {:?}. Our frame: {}",
                            frame_modulo, self.frame
                        );
                    }
                }

                NetworkPacket::GoodBye => {
                    return EventOutcome::TheyLeft { politely: true };
                }
            },

            NetworkEvent::ReceiveError(e) => {
                panic!("Failed to receive from socket: {:?}", e);
            }

            NetworkEvent::RemoteDisconnected => {
                return EventOutcome::TheyLeft { politely: false };
            }
        }
        EventOutcome::None
    }

    pub fn have_everyone_committed_frame(&self) -> bool {
        self.has_committed_frame && self.has_remote_committed_frame
    }

    pub fn exit(&mut self) {
        match self.send_net_packet(NetworkPacket::GoodBye) {
            Ok(_) => {}
            Err(NetError::Disconnected) => {}
            Err(error) => panic!("Failed to send goodbye: {:?}", error),
        }
    }

    fn send_direction_command(&mut self, direction: Direction) -> NetResult<()> {
        self.socket
            .write_all(&[NetworkPacket::SetDirection(SetDirectionPacket::new(
                self.frame, direction,
            ))
            .serialize()])
            .map_err(NetError::from)
    }

    fn send_net_packet(&mut self, packet: NetworkPacket) -> NetResult<()> {
        self.socket
            .write_all(&[packet.serialize()])
            .map_err(NetError::from)
    }

    pub fn spawn_socket_reader(
        &mut self,
        sender: Sender<ThreadMessage>,
        slow_io: bool,
    ) -> NetResult<()> {
        let socket = self.socket.try_clone()?;
        thread::spawn(move || run_socket_reader(socket, sender, slow_io));
        Ok(())
    }
}

type NetResult<T> = Result<T, NetError>;

#[derive(Debug)]
pub enum NetError {
    Disconnected,
    Other(std::io::Error),
}

impl From<std::io::Error> for NetError {
    fn from(io_error: std::io::Error) -> Self {
        match io_error.kind() {
            ErrorKind::ConnectionReset => NetError::Disconnected,
            _ => NetError::Other(io_error),
        }
    }
}

impl Display for NetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for NetError {}

fn run_socket_reader(mut socket: TcpStream, sender: Sender<ThreadMessage>, slow_io: bool) {
    let mut buf = Vec::new();
    let mut read_buf = [0; 1024];
    loop {
        match socket.read(&mut read_buf) {
            Ok(n) => {
                if slow_io {
                    thread::sleep(Duration::from_millis(10)); //TODO
                }

                buf.extend_from_slice(&read_buf[..n]);

                for byte in &buf {
                    let msg = match NetworkPacket::parse(*byte) {
                        Some(msg) => msg,
                        None => {
                            let msg = ThreadMessage::Network(NetworkEvent::ReceiveError(format!(
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
                    _ => NetworkEvent::ReceiveError(format!("Failed to read: {:?}", error)),
                };
                if sender.send(ThreadMessage::Network(msg)).is_err() {
                    // no receiver (i.e. main thread has exited)
                }
                return;
            }
        }
    }
}

pub enum EventOutcome {
    SetDirection(SetDirectionCommand),
    TheyLeft { politely: bool },
    None,
}

pub struct SetDirectionCommand {
    pub player_i: PlayerIndex,
    pub direction: Direction,
}

#[derive(Debug)]
pub enum NetworkEvent {
    Received(NetworkPacket),
    ReceiveError(String),
    RemoteDisconnected,
}

#[derive(Debug)]
pub enum NetworkPacket {
    SetDirection(SetDirectionPacket),
    CommitFrame(u8),
    GoodBye,
}

#[derive(Debug, Copy, Clone)]
pub struct SetDirectionPacket {
    frame_modulo: u8,
    direction: Direction,
}

impl SetDirectionPacket {
    fn new(frame: u32, direction: Direction) -> Self {
        Self {
            frame_modulo: NetworkPacket::modulo(frame),
            direction,
        }
    }
}

impl NetworkPacket {
    // 10000000 = GoodBye
    // 1fffff__ = CommitFrame(frame)
    // 0fffffdd = SetDirection(frame, direction)
    // 0     00 = UP
    // 0     01 = LEFT
    // 0     10 = DOWN
    // 0     11 = RIGHT
    // _fffff__ = FRAME % 32

    fn parse(byte: u8) -> Option<Self> {
        if byte == 0b_1000_0000 {
            return Some(NetworkPacket::GoodBye);
        }

        let frame_modulo = (byte & 0b_0111_1100) >> 2;

        if (byte & 0b_1000_0000) != 0 {
            return Some(NetworkPacket::CommitFrame(frame_modulo));
        }

        let direction = match byte & 0b_11 {
            0b_00 => UP,
            0b_01 => LEFT,
            0b_10 => DOWN,
            0b_11 => RIGHT,
            _ => return None,
        };
        Some(NetworkPacket::SetDirection(SetDirectionPacket {
            frame_modulo,
            direction,
        }))
    }

    fn serialize(&self) -> u8 {
        match self {
            NetworkPacket::GoodBye => 0b_1000_0000,
            NetworkPacket::CommitFrame(frame_modulo) => 0b_1000_0000 | (frame_modulo << 2),
            NetworkPacket::SetDirection(SetDirectionPacket {
                frame_modulo,
                direction,
            }) => (frame_modulo << 2) | Self::direction_part(direction),
        }
    }

    fn direction_part(direction: &Direction) -> u8 {
        match *direction {
            UP => 0b_00,
            LEFT => 0b_01,
            DOWN => 0b_10,
            RIGHT => 0b_11,
            _ => panic!("Invalid direction: {:?}", direction),
        }
    }

    fn modulo(frame: u32) -> u8 {
        (frame % 32) as u8
    }
}
