use std::collections::{HashMap, VecDeque};

use bytes::{Buf};

#[derive(Debug)]
pub enum ChunkError {
    UnknownType3,
}

enum State {
    Uninitialized,
    BasicHeader {
        fmt: u8,
        cs_id: u16,
    },
    ChunkMessageHeader {
        cs_id: u16,
    }
}

pub struct Chunk {
    state: State,
    buffer: VecDeque<u8>,
    cs_headers: HashMap<u16, HeaderMessage>,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            state: State::Uninitialized,
            buffer: VecDeque::with_capacity(128),
            cs_headers: HashMap::new(),
        }
    }

    pub fn buffering(&mut self, buf: &[u8]) {
        self.buffer.extend(buf)
    }

    pub fn poll(&mut self) -> Result<Option<Message>, ChunkError> {
        loop {
            match self.state {
                State::Uninitialized => {
                    if 0 == self.buffer.len() {
                        return Ok(None)
                    }

                    let n = self.buffer.get_u8();
                    self.state = State::BasicHeader {
                        fmt: n >> 6,
                        cs_id: {
                            match 0b00111111 & n {
                                0b111111 => {
                                    self.buffer.get_u16() + 64
                                },
                                0b000000 => {
                                    (self.buffer.get_u8() as u16) + 64
                                },
                                n => {
                                    n as u16
                                },
                            }
                        },
                    };
                },
                State::BasicHeader { fmt, cs_id } => {
                    /*  */ if 0 == fmt && 11 > self.buffer.len() {
                        return Ok(None)
                    } else if 1 == fmt && 7 > self.buffer.len() {
                        return Ok(None)
                    } else if 2 == fmt && 3 > self.buffer.len() {
                        return Ok(None)
                    }

                    if 0b11 == fmt {
                        self.state = State::ChunkMessageHeader {
                            cs_id,
                        }
                    } else {
                        let timestamp = {
                            let mut n = 0;
                            for x in self.buffer.drain(0..3) {
                                n = (n << 8) | (x as u32);
                            }
                            n
                        };

                        if 0b10 == fmt {
                            self.cs_headers.insert(cs_id, HeaderMessage {
                                timestamp,
                                length: None,
                                type_id: None,
                                stream_id: None
                            })
                        } else {
                            let length = {
                                let mut n = 0;
                                for x in self.buffer.drain(0..3) {
                                    n = (n << 8) | (x as u32);
                                }
                                n
                            };
                            let type_id = self.buffer.get_u8();

                            if 0b00 != fmt {
                                self.cs_headers.insert(cs_id, HeaderMessage {
                                    timestamp,
                                    length: Some(length),
                                    type_id: Some(type_id),
                                    stream_id: None,
                                })
                            } else {
                                self.cs_headers.insert(cs_id, HeaderMessage {
                                    timestamp,
                                    length: Some(length),
                                    type_id: Some(type_id),
                                    stream_id: Some(self.buffer.get_u32())
                                })
                            }
                        };

                        self.state = State::ChunkMessageHeader {
                            cs_id,
                        }
                    }
                },
                State::ChunkMessageHeader { cs_id } => {
                    let header_msg = if let Some(header_msg) = self.cs_headers.get(&cs_id) {
                        header_msg
                    } else {
                        return Err(ChunkError::UnknownType3)
                    };

                    let timestamp = header_msg.timestamp;
                    let stream_id = header_msg.stream_id;

                    if let Some(length) = header_msg.length {
                        if length as usize > self.buffer.len() {
                            return Ok(None)
                        }
                    }

                    match header_msg.type_id {
                        Some(message_types::SET_CHUNK_SIZE) => {
                            let chunk_size = self.buffer.get_i32();

                            self.state = State::Uninitialized;
                            self.cs_headers.remove(&cs_id);

                            if chunk_size as usize > self.buffer.capacity() {
                                self.buffer.reserve_exact((chunk_size as usize) - self.buffer.capacity());
                            }
                            return Ok(Some(Message {
                                cs_id,
                                timestamp,
                                stream_id,
                                msg: MessageData::SetChunkSize { chunk_size },
                            }))
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

pub struct Message {
    pub cs_id: u16,
    pub timestamp: u32,
    pub stream_id: Option<u32>,
    pub msg: MessageData,
}

pub enum MessageData {
    SetChunkSize {
        chunk_size: i32,
    },
    Unknown,
}

// 5.3.1.2. Chunk Message Header
pub struct HeaderMessage {
    pub timestamp: u32,
    pub length: Option<u32>,
    pub type_id: Option<u8>,
    pub stream_id: Option<u32>,
}

mod message_types {
    // 5.4.1. Set Chunk Size (1)
    pub const SET_CHUNK_SIZE                : u8 =  1;
}
