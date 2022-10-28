use crate::app::ThreadMessage;
use crate::game::{Direction, PlayerIndex, DOWN, LEFT, RIGHT, UP};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

pub struct Networking {
    pub ready_to_run_frame: bool,
    socket: TcpStream,
    received_commands_for_next_frame: Vec<SetDirection>,
    player_controlled_by_socket: PlayerIndex,
}

impl Networking {
    pub fn new(socket: TcpStream, player_controlled_by_socket: PlayerIndex) -> Self {
        Self {
            socket,
            ready_to_run_frame: false,
            received_commands_for_next_frame: vec![],
            player_controlled_by_socket,
        }
    }

    pub fn handle_received_command(
        &mut self,
        cmd: SetDirection,
        current_frame: u32,
    ) -> Option<(PlayerIndex, Direction)> {
        if cmd.frame_modulo == SetDirection::modulo(current_frame - 1) {
            // Too late. That frame has already been run.
        } else if cmd.frame_modulo == SetDirection::modulo(current_frame) {
            self.ready_to_run_frame = true;
            let player_i = self.player_controlled_by_socket;
            return Some((player_i, cmd.direction));
        } else if cmd.frame_modulo == SetDirection::modulo(current_frame + 1) {
            self.received_commands_for_next_frame.push(cmd);
        } else {
            panic!(
                "Received command with unexpected frame modulo: {:?}. Our frame: {}",
                cmd, current_frame
            );
        }
        None
    }

    pub fn on_start_of_frame(
        &mut self,
        frame: u32,
        local_player_direction: Direction,
    ) -> anyhow::Result<Option<(PlayerIndex, Direction)>> {
        // Send out a direction message for the next frame.
        // This tells the remote that we're ready for it
        // TODO: what if remote is further ahead and they immediately
        // execute the next frame, meaning that we never got a chance
        // to control our line?
        self.send_direction_command(frame, local_player_direction)?;

        self.ready_to_run_frame = false;
        let maybe_command = self
            .received_commands_for_next_frame
            .last()
            .copied()
            .map(|command| {
                assert_eq!(SetDirection::modulo(frame), command.frame_modulo);
                self.received_commands_for_next_frame.clear();
                self.ready_to_run_frame = true;
                (self.player_controlled_by_socket, command.direction)
            });
        Ok(maybe_command)
    }

    pub fn on_exit(&mut self) {
        if let Err(error) = self.send_net_packet(NetworkPacket::GoodBye) {
            match error.kind() {
                ErrorKind::ConnectionReset => {}
                _ => panic!("Failed to send goodbye: {:?}", error),
            }
        }
    }

    pub fn send_direction_command(
        &mut self,
        frame: u32,
        direction: Direction,
    ) -> std::io::Result<()> {
        self.socket
            .write_all(&[NetworkPacket::Command(SetDirection::new(frame, direction)).serialize()])
    }

    pub fn send_net_packet(&mut self, packet: NetworkPacket) -> std::io::Result<()> {
        self.socket.write_all(&[packet.serialize()])
    }

    pub fn spawn_socket_reader(
        &mut self,
        sender: Sender<ThreadMessage>,
        slow_io: bool,
    ) -> anyhow::Result<()> {
        let socket = self.socket.try_clone()?;
        thread::spawn(move || run_socket_reader(socket, sender, slow_io));
        Ok(())
    }
}

fn run_socket_reader(mut socket: TcpStream, sender: Sender<ThreadMessage>, slow_io: bool) {
    let mut buf = Vec::new();
    let mut read_buf = [0; 1024];
    loop {
        match socket.read(&mut read_buf) {
            Ok(n) => {
                if slow_io {
                    thread::sleep(Duration::from_millis(300)); //TODO
                }

                buf.extend_from_slice(&read_buf[..n]);

                for byte in &buf {
                    let msg = match NetworkPacket::parse(*byte) {
                        Some(msg) => msg,
                        None => {
                            let msg = ThreadMessage::Network(NetworkEvent::Error(format!(
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
                    _ => NetworkEvent::Error(format!("Socket error: {:?}", error)),
                };
                if sender.send(ThreadMessage::Network(msg)).is_err() {
                    // no receiver (i.e. main thread has exited)
                }
                return;
            }
        }
    }
}

#[derive(Debug)]
pub enum NetworkEvent {
    Received(NetworkPacket),
    Error(String),
    RemoteDisconnected,
}

#[derive(Debug)]
pub enum NetworkPacket {
    Command(SetDirection),
    GoodBye,
}

#[derive(Debug, Copy, Clone)]
pub struct SetDirection {
    frame_modulo: u8,
    direction: Direction,
}

impl SetDirection {
    fn new(frame: u32, direction: Direction) -> Self {
        Self {
            frame_modulo: Self::modulo(frame),
            direction,
        }
    }

    fn modulo(frame: u32) -> u8 {
        (frame % 64) as u8
    }
}

impl NetworkPacket {
    // 00000000 = GoodBye
    // ______dd = direction
    //       00 = UP
    //       01 = LEFT
    //       10 = DOWN
    //       11 = RIGHT
    // ffffff__ = FRAME % 64

    fn parse(byte: u8) -> Option<Self> {
        if byte == 0 {
            return Some(NetworkPacket::GoodBye);
        }

        let frame_modulo = (byte & 0b1111_1100) >> 2;
        let direction = match byte & 0b11 {
            0b00 => UP,
            0b01 => LEFT,
            0b10 => DOWN,
            0b11 => RIGHT,
            _ => return None,
        };
        Some(NetworkPacket::Command(SetDirection {
            frame_modulo,
            direction,
        }))
    }

    fn serialize(&self) -> u8 {
        match self {
            NetworkPacket::Command(SetDirection {
                frame_modulo,
                direction,
            }) => {
                let direction_part = match *direction {
                    UP => 0b00,
                    LEFT => 0b01,
                    DOWN => 0b10,
                    RIGHT => 0b11,
                    _ => panic!("Invalid direction: {:?}", direction),
                };
                (frame_modulo << 2) | direction_part
            }
            NetworkPacket::GoodBye => 0,
        }
    }
}
