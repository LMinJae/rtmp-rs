use std::collections::VecDeque;
use std::time::SystemTime;
use bytes::{Buf, BufMut, BytesMut};

#[derive(Debug)]
pub enum HandshakeError {
    UnsupportedVersion,
    EchoMismatch,
    Done,
}

enum State {
    Uninitialized,
    VersionSent,
    AckSent,
    Done
}

pub struct Handshake {
    state: State,
    buffer: VecDeque<u8>,
    time: u32,
    p1: Vec<u8>,
}

impl Handshake {
    pub fn new() -> Self {
        Handshake {
            state: State::Uninitialized,
            buffer: VecDeque::with_capacity(1537),
            time: 0,
            p1: Vec::<u8>::with_capacity(1528),
        }
    }

    pub fn consume(&mut self, buf: &[u8]) -> Result<Vec<u8>, HandshakeError> {
        self.buffer.extend(buf);

        match self.state {
            State::Uninitialized => self.generate_p0_p1(),
            State::VersionSent => self.stage_p0_p1(),
            State::AckSent => self.stage_p2(),
            State::Done => Err(HandshakeError::Done),
        }
    }

    fn generate_p0_p1(&mut self) -> Result<Vec<u8>, HandshakeError> {
        let mut buf = Vec::<u8>::with_capacity(1537);

        // Phase-0
        // - Version
        buf.put_u8(0x03);

        // Phase-1
        // - Time: may be 0
        if let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            self.time = now.as_millis() as u32;
        }
        buf.put_u32(self.time);
        // - Zero: MUST be all 0s
        buf.put_u32(0x00000000);
        // - Random data
        let mut rng = wyrng::WyRng::new(self.time as u64);
        for _ in 0..191 {
            self.p1.put_slice(&rng.generate().to_ne_bytes()[..]);
        }

        buf.extend(self.p1.iter());

        self.state = State::VersionSent;

        Ok(buf)
    }

    fn stage_p0_p1(&mut self) -> Result<Vec<u8>, HandshakeError> {
        if 1537 != self.buffer.len() {
            return Ok(Vec::<u8>::new())
        }

        if let Some(version) = self.buffer.pop_front() {
            if version < 3 {
                return Err(HandshakeError::UnsupportedVersion)
            }
        }

        let mut time: u32 = 0;
        for x in self.buffer.drain(..4) {
            time = (time << 8) | (x as u32);
        }
        // skip zero
        self.buffer.drain(..4);

        let mut time2: u32 = 0;
        if let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            time2 = now.as_millis() as u32;
        }

        let mut buf = Vec::<u8>::with_capacity(1536);

        // Time
        buf.put_u32(time);
        // Time2
        buf.put_u32(time2);

        // Echo:  random data field sent by the peer in S1 (for C2) or S2 (for C1)
        for n in self.buffer.drain(..1528) {
            buf.put_u8(n)
        }

        self.state = State::AckSent;

        Ok(buf)
    }

    fn stage_p2(&mut self) -> Result<Vec<u8>, HandshakeError> {
        if 1536 == self.buffer.len() {
            return Ok(Vec::<u8>::new())
        }

        // Time
        let time = self.buffer.get_u32();
        // Time2
        let time2 = self.buffer.get_u32();

        for (i, n) in self.buffer.drain(..1528).enumerate() {
            if n != self.p1[i] {
                return Err(HandshakeError::EchoMismatch);
            }
        }

        self.state = State::Done;

        Err(HandshakeError::Done)
    }
}
