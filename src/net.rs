use crate::app::ThreadMessage;
use crate::game::{Direction, PlayerIndex, DOWN, LEFT, RIGHT, UP};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Networking {
    socket: TcpStream,
    session: Arc<Mutex<Session>>,
}

impl Networking {
    pub fn host(
        mut socket: TcpStream,
        local_player: PlayerIndex,
        remote_player: PlayerIndex,
        player_direction: Direction,
        frame: u32,
        game_size: (u16, u16),
        local_player_name: String,
    ) -> (Self, GameInfo) {
        ChooseGameSizePacket(game_size).write(&mut socket);
        ChooseNamePacket(local_player_name).write(&mut socket);

        let remote_player_name = ChooseNamePacket::read(&mut socket).0;

        let game_info = GameInfo {
            size: game_size,
            remote_player_name,
        };

        let session = Arc::new(Mutex::new(Session::new(
            local_player,
            remote_player,
            player_direction,
            frame,
        )));

        (Self { socket, session }, game_info)
    }

    pub fn join(
        mut socket: TcpStream,
        local_player: PlayerIndex,
        remote_player: PlayerIndex,
        player_direction: Direction,
        frame: u32,
        local_player_name: String,
    ) -> (Self, GameInfo) {
        ChooseNamePacket(local_player_name).write(&mut socket);

        let game_size = ChooseGameSizePacket::read(&mut socket).0;
        let remote_player_name = ChooseNamePacket::read(&mut socket).0;

        let game_info = GameInfo {
            size: game_size,
            remote_player_name,
        };

        let session = Arc::new(Mutex::new(Session::new(
            local_player,
            remote_player,
            player_direction,
            frame,
        )));

        (Self { socket, session }, game_info)
    }

    pub fn start_game(&mut self, sender: Sender<ThreadMessage>) -> NetResult<Vec<Outcome>> {
        self.spawn_socket_reader(sender)?;

        let outgoing_packet = self.session.lock().unwrap().start_game();
        let mut outcomes = vec![];
        self.send_packet(outgoing_packet.0, &mut outcomes)?;
        Ok(outcomes)
    }

    pub fn remote_player_index(&self) -> PlayerIndex {
        self.session.lock().unwrap().remote_player
    }

    pub fn start_new_frame(&mut self, frame: u32) -> NetResult<Vec<Outcome>> {
        let (outgoing_packet, mut outcomes) = self.session.lock().unwrap().start_new_frame(frame);
        self.send_packet(outgoing_packet.0, &mut outcomes)?;
        Ok(outcomes)
    }

    pub fn set_direction(&mut self, direction: Direction) -> NetResult<Vec<Outcome>> {
        let (outgoing_packet, mut outcomes) = self.session.lock().unwrap().set_direction(direction);
        if let Some(outgoing_packet) = outgoing_packet {
            self.send_packet(outgoing_packet.0, &mut outcomes)?;
        }
        Ok(outcomes)
    }

    pub fn commit_frame(&mut self) -> NetResult<Vec<Outcome>> {
        let (outgoing_packet, mut outcomes) = self.session.lock().unwrap().commit_frame();
        if let Some(outgoing_packet) = outgoing_packet {
            self.send_packet(outgoing_packet.0, &mut outcomes)?;
        }
        Ok(outcomes)
    }

    pub fn take_buffered_outcomes(&mut self) -> Vec<Outcome> {
        let mut session = self.session.lock().unwrap();
        std::mem::take(&mut session.buffered_outcomes)
    }

    pub fn exit(&mut self) {
        let mut outcomes = vec![];
        match self.send_packet(SessionPacket::GoodBye, &mut outcomes) {
            Ok(()) => {}
            Err(error) => panic!("Failed to send goodbye: {:?}", error),
        }
    }

    fn send_packet(&mut self, packet: SessionPacket, outcomes: &mut Vec<Outcome>) -> NetResult<()> {
        if let Err(io_error) = self.socket.write_all(&[packet.serialize()]) {
            match io_error.kind() {
                ErrorKind::ConnectionReset => {
                    outcomes.push(Outcome::RemoteLeft { politely: false })
                }
                _ => return Err(io_error),
            }
        }

        Ok(())
    }

    pub fn spawn_socket_reader(&mut self, sender: Sender<ThreadMessage>) -> NetResult<()> {
        let socket = self.socket.try_clone()?;
        let session = Arc::clone(&self.session);
        thread::spawn(move || run_socket_reader(socket, sender, session));
        Ok(())
    }
}

#[derive(Debug)]
pub struct GameInfo {
    pub size: (u16, u16),
    pub remote_player_name: String,
}

struct Session {
    player: PlayerIndex,
    remote_player: PlayerIndex,
    player_direction: Direction,
    frame: u32,
    queued_command_from_remote: Option<Direction>,
    has_remote_committed_frame: bool,
    has_remote_committed_next_frame: bool,
    has_committed_frame: bool,
    queued_command: Option<Direction>,
    buffered_outcomes: Vec<Outcome>,
}

impl Session {
    fn new(
        local_player: PlayerIndex,
        remote_player: PlayerIndex,
        player_direction: Direction,
        frame: u32,
    ) -> Self {
        Self {
            player: local_player,
            remote_player,
            player_direction,
            frame,
            queued_command_from_remote: None,
            has_remote_committed_frame: false,
            has_remote_committed_next_frame: false,
            has_committed_frame: false,
            queued_command: None,
            buffered_outcomes: Vec::new(),
        }
    }

    fn start_game(&mut self) -> OutgoingPacket {
        OutgoingPacket(SessionPacket::SetDirection(SetDirectionPacket::new(
            self.frame,
            self.player_direction,
        )))
    }

    fn start_new_frame(&mut self, frame: u32) -> (OutgoingPacket, Vec<Outcome>) {
        self.frame = frame;
        self.has_committed_frame = false;
        self.has_remote_committed_frame = false;

        if let Some(dir) = self.queued_command.take() {
            self.player_direction = dir;
            self.buffered_outcomes
                .push(Outcome::PlayerControl(PlayerControlOutcome::new(
                    self.player,
                    dir,
                )));
        }

        if let Some(dir) = self.queued_command_from_remote.take() {
            self.buffered_outcomes
                .push(Outcome::PlayerControl(PlayerControlOutcome::new(
                    self.remote_player,
                    dir,
                )));
        }

        if self.has_remote_committed_next_frame {
            self.has_remote_committed_frame = true;
            self.has_remote_committed_next_frame = false;
        }

        let outgoing_packet = OutgoingPacket(SessionPacket::SetDirection(SetDirectionPacket::new(
            self.frame,
            self.player_direction,
        )));
        (outgoing_packet, std::mem::take(&mut self.buffered_outcomes))
    }

    fn set_direction(&mut self, direction: Direction) -> (Option<OutgoingPacket>, Vec<Outcome>) {
        let outgoing_packet = if self.has_committed_frame {
            self.queued_command = Some(direction);
            None
        } else {
            self.player_direction = direction;
            self.buffered_outcomes
                .push(Outcome::PlayerControl(PlayerControlOutcome::new(
                    self.player,
                    direction,
                )));
            Some(OutgoingPacket(SessionPacket::SetDirection(
                SetDirectionPacket::new(self.frame, direction),
            )))
        };

        (outgoing_packet, std::mem::take(&mut self.buffered_outcomes))
    }

    fn commit_frame(&mut self) -> (Option<OutgoingPacket>, Vec<Outcome>) {
        let outgoing_packet = if !self.has_committed_frame {
            self.has_committed_frame = true;

            if self.has_remote_committed_frame {
                self.buffered_outcomes.push(Outcome::RunFrame);
            }
            let outgoing_packet = OutgoingPacket(SessionPacket::CommitFrame(
                CommitFramePacket::new(self.frame),
            ));
            Some(outgoing_packet)
        } else {
            None
        };
        (outgoing_packet, std::mem::take(&mut self.buffered_outcomes))
    }

    fn on_received_set_direction(&mut self, pkt: SetDirectionPacket) -> bool {
        if pkt.frame_modulo == SessionPacket::modulo(self.frame) {
            assert!(!self.has_remote_committed_frame);

            self.buffered_outcomes
                .push(Outcome::PlayerControl(PlayerControlOutcome::new(
                    self.remote_player,
                    pkt.direction,
                )));
            true
        } else if pkt.frame_modulo == SessionPacket::modulo(self.frame + 1) {
            assert!(!self.has_remote_committed_next_frame);
            self.queued_command_from_remote = Some(pkt.direction);
            false
        } else {
            panic!(
                "Received command with unexpected frame modulo: {:?}. Our frame: {}",
                pkt, self.frame
            );
        }
    }

    fn on_received_commit_frame(&mut self, pkt: CommitFramePacket) -> bool {
        if pkt.0 == SessionPacket::modulo(self.frame) {
            self.has_remote_committed_frame = true;
            if self.has_committed_frame {
                self.buffered_outcomes.push(Outcome::RunFrame);
                true
            } else {
                false
            }
        } else if pkt.0 == SessionPacket::modulo(self.frame + 1) {
            self.has_remote_committed_next_frame = true;
            false
        } else {
            panic!(
                "Received commit with unexpected frame modulo: {:?}. Our frame: {}",
                pkt, self.frame
            );
        }
    }

    fn on_received_good_bye(&mut self) {
        self.buffered_outcomes
            .push(Outcome::RemoteLeft { politely: true });
    }
}

struct OutgoingPacket(SessionPacket);

pub type NetResult<T> = Result<T, std::io::Error>;

fn run_socket_reader(
    mut socket: TcpStream,
    sender: Sender<ThreadMessage>,
    session: Arc<Mutex<Session>>,
) {
    let mut buf = Vec::new();
    let mut read_buf = [0; 1024];
    loop {
        match socket.read(&mut read_buf) {
            Ok(n) => {
                buf.extend_from_slice(&read_buf[..n]);

                for byte in &buf {
                    let packet = match SessionPacket::parse(*byte) {
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

                    let mut session = session.lock().unwrap();

                    let mut remote_left = false;

                    let new_outcomes = match packet {
                        SessionPacket::SetDirection(pkt) => session.on_received_set_direction(pkt),
                        SessionPacket::CommitFrame(pkt) => session.on_received_commit_frame(pkt),
                        SessionPacket::GoodBye => {
                            remote_left = true;
                            session.on_received_good_bye();
                            true
                        }
                    };

                    if new_outcomes {
                        let event = NetworkEvent::BufferedOutcomes;
                        if sender.send(ThreadMessage::Network(event)).is_err() {
                            // no receiver (i.e. main thread has exited)
                            return;
                        }
                    }
                    if remote_left {
                        return;
                    }
                }
                buf.clear();
            }
            Err(error) => {
                let event = match error.kind() {
                    ErrorKind::ConnectionReset => {
                        let mut session = session.lock().unwrap();
                        session
                            .buffered_outcomes
                            .push(Outcome::RemoteLeft { politely: false });
                        NetworkEvent::BufferedOutcomes
                    }
                    _ => NetworkEvent::ReceiveError(format!("Failed to read: {:?}", error)),
                };
                if sender.send(ThreadMessage::Network(event)).is_err() {
                    // no receiver (i.e. main thread has exited)
                }
                return;
            }
        }
    }
}

#[derive(Debug)]
pub enum NetworkEvent {
    BufferedOutcomes,
    ReceiveError(String),
}

#[derive(Debug)]
pub enum Outcome {
    PlayerControl(PlayerControlOutcome),
    RunFrame,
    RemoteLeft { politely: bool },
}

#[derive(Debug, Copy, Clone)]
pub struct PlayerControlOutcome {
    pub player_i: PlayerIndex,
    pub direction: Direction,
}

impl PlayerControlOutcome {
    pub fn new(player_i: PlayerIndex, direction: Direction) -> Self {
        Self {
            player_i,
            direction,
        }
    }
}

#[derive(Debug, Clone)]
struct ChooseNamePacket(String);

impl ChooseNamePacket {
    fn read(reader: &mut dyn Read) -> Self {
        let mut len = [0];
        reader.read_exact(&mut len).unwrap();
        let len = u8::from_be_bytes(len);
        let mut name = vec![0; len as usize];
        reader.read_exact(&mut name).unwrap();
        let name = String::from_utf8(name).unwrap();
        Self(name)
    }

    fn write(&self, writer: &mut dyn Write) {
        let name = self.0.as_bytes();
        let len = name.len() as u8;
        writer.write_all(&[len]).unwrap();
        writer.write_all(name).unwrap();
    }
}

#[derive(Debug, Clone, Copy)]
struct ChooseGameSizePacket((u16, u16));

impl ChooseGameSizePacket {
    fn read(reader: &mut dyn Read) -> Self {
        let mut w_buf = [0; 2];
        reader.read_exact(&mut w_buf).unwrap();
        let mut h_buf = [0; 2];
        reader.read_exact(&mut h_buf).unwrap();
        let game_size = (u16::from_be_bytes(w_buf), u16::from_be_bytes(h_buf));
        Self(game_size)
    }

    fn write(&self, writer: &mut dyn Write) {
        let game_size = self.0;
        let w = game_size.0.to_be_bytes();
        let h = game_size.1.to_be_bytes();
        writer.write_all(&w).unwrap();
        writer.write_all(&h).unwrap();
    }
}

#[derive(Debug, Copy, Clone)]
enum SessionPacket {
    SetDirection(SetDirectionPacket),
    CommitFrame(CommitFramePacket),
    GoodBye,
}

#[derive(Debug, Copy, Clone)]
struct SetDirectionPacket {
    frame_modulo: u8,
    direction: Direction,
}

impl SetDirectionPacket {
    fn new(frame: u32, direction: Direction) -> Self {
        Self {
            frame_modulo: SessionPacket::modulo(frame),
            direction,
        }
    }
}

#[derive(Debug, Copy, Clone)]
struct CommitFramePacket(u8);

impl CommitFramePacket {
    fn new(frame: u32) -> Self {
        Self(SessionPacket::modulo(frame))
    }
}

impl SessionPacket {
    // 10000000 = GoodBye
    // 1fffff11 = CommitFrame(frame)
    // 0fffffdd = SetDirection(frame, direction)
    // 0     00 = UP
    // 0     01 = LEFT
    // 0     10 = DOWN
    // 0     11 = RIGHT
    // _fffff__ = FRAME % 32

    fn parse(byte: u8) -> Option<Self> {
        if byte == 0b_1000_0000 {
            return Some(SessionPacket::GoodBye);
        }

        let frame_modulo = (byte & 0b_0111_1100) >> 2;

        if (byte & 0b_1000_0000) != 0 {
            return Some(SessionPacket::CommitFrame(CommitFramePacket(frame_modulo)));
        }

        let direction = match byte & 0b_11 {
            0b_00 => UP,
            0b_01 => LEFT,
            0b_10 => DOWN,
            0b_11 => RIGHT,
            _ => return None,
        };
        Some(SessionPacket::SetDirection(SetDirectionPacket {
            frame_modulo,
            direction,
        }))
    }

    fn serialize(&self) -> u8 {
        match self {
            SessionPacket::GoodBye => 0b_1000_0000,
            SessionPacket::CommitFrame(CommitFramePacket(frame_modulo)) => {
                0b_1000_0011 | (frame_modulo << 2)
            }
            SessionPacket::SetDirection(SetDirectionPacket {
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
