use std::cmp::min;
use std::collections::HashMap;

use bytes::{Buf, BufMut, BytesMut};

use crate::message;

#[derive(Debug)]
pub enum ChunkError {
    UnknownChunkStream,
    UnknownEvent(message::UserControlEvent),
    WrongInput,
}

#[derive(Debug, PartialEq)]
enum State {
    Uninitialized,
    BasicHeader {
        fmt: u8,
        cs_id: u32,
    },
    ChunkMessageHeader {
        cs_id: u32,
    }
}

const DEFAULT_CHUNK_SIZE: i32 = 128;

pub struct Chunk {
    state: State,
    in_chunk_size: i32,
    out_chunk_size: i32,
    window_size: u32,
    limit_type: LimitType,
    rd_buf: BytesMut,
    wr_buf: BytesMut,
    cs_headers: HashMap<u32, message::MessagePacket>,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            state: State::Uninitialized,
            in_chunk_size: DEFAULT_CHUNK_SIZE,
            out_chunk_size: DEFAULT_CHUNK_SIZE,
            window_size: 2500000,
            limit_type: LimitType::Dynamic,
            rd_buf: BytesMut::with_capacity(128),
            wr_buf: BytesMut::with_capacity(128),
            cs_headers: HashMap::new(),
        }
    }

    pub fn buffering(&mut self, buf: &[u8]) {
        self.rd_buf.extend(buf)
    }

    pub fn poll(&mut self) -> Result<Option<message::Message>, ChunkError> {
        loop {
            if let State::Uninitialized = self.state {
                if 0 == self.rd_buf.len() {
                    return Ok(None)
                }

                self.parse_basic_header()
            }

            if let State::BasicHeader { fmt, cs_id } = self.state {
                /*  */ if 0 == fmt && 11 > self.rd_buf.len() {
                    return Ok(None)
                } else if 1 == fmt &&  7 > self.rd_buf.len() {
                    return Ok(None)
                } else if 2 == fmt &&  3 > self.rd_buf.len() {
                    return Ok(None)
                }

                macro_rules! read_u24 {
                    () => {
                        {
                            let mut v = 0_u32;
                            for x in self.rd_buf.split_to(3) {
                                v = v << 8 | (x as u32)
                            }
                            v
                        }
                    };
                }

                if 0b11 != fmt {
                    if let Some(msg) = self.cs_headers.get_mut(&cs_id) {
                        let timestamp = read_u24!();
                        if message::TIMESTAMP_MAX > timestamp {
                            if 0b00 == fmt {
                                msg.header.timestamp = timestamp;
                            } else {
                                msg.header.timestamp_delta = Some(timestamp);
                            }
                        }

                        if 0b10 != fmt {
                            let length = read_u24!();

                            msg.header.length = length;
                            if length as usize > msg.payload.capacity() {
                                msg.payload.reserve((length as usize) - msg.payload.capacity())
                            }
                        }

                        if 0b10 != fmt {
                            let type_id = self.rd_buf.get_u8();

                            msg.header.type_id = type_id;
                        }

                        if 0b00 == fmt {
                            let stream_id = self.rd_buf.get_u32();

                            msg.header.stream_id = stream_id;
                        }

                        if message::TIMESTAMP_MAX <= timestamp {
                            if 0b00 == fmt {
                                msg.header.timestamp = self.rd_buf.get_u32()
                            } else {
                                msg.header.timestamp_delta = Some(self.rd_buf.get_u32())
                            }
                        }

                        if let Some(d) = msg.header.timestamp_delta {
                            msg.header.timestamp += d;
                        }
                    } else if 0x00 == fmt {
                        let header = message::Header {
                            timestamp: read_u24!(),
                            length: read_u24!(),
                            type_id: self.rd_buf.get_u8(),
                            stream_id: self.rd_buf.get_u32(),
                            timestamp_delta: None,
                        };

                        self.cs_headers.insert(cs_id, message::MessagePacket {
                            header,
                            payload: BytesMut::with_capacity(header.length as usize),
                        });
                    } else {
                        return Err(ChunkError::UnknownChunkStream)
                    }
                }
                self.state = State::ChunkMessageHeader { cs_id }
            }
            if let State::ChunkMessageHeader { cs_id } = self.state {
                if let Some(msg) = self.cs_headers.get_mut(&cs_id) {
                    let length = msg.header.length as usize;

                    let left = min(self.in_chunk_size as usize, length - msg.payload.len());
                    if left > self.rd_buf.len() {
                        return Ok(None)
                    }
                    msg.payload.put(self.rd_buf.split_to(left));

                    if msg.payload.len() != length {
                        return Ok(None)
                    }
                }
                let msg =  self.cs_headers.get(&cs_id).unwrap().clone();

                let rst = self.process_message(cs_id, msg.header.clone(), msg.payload.clone());

                self.state = State::Uninitialized;
                return rst
            }
        }
    }

    fn process_message(&mut self, cs_id: u32, header: message::Header, payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        match header.type_id {
            message::msg_type::SET_CHUNK_SIZE => self.handle_set_chunk_size(payload),
            message::msg_type::ABORT => self.handle_abort(payload),
            message::msg_type::ACK => self.handle_ack(payload),
            message::msg_type::WINDOW_ACK_SIZE => self.handle_window_ack_size(payload),
            message::msg_type::SET_PEER_BANDWIDTH => self.handle_set_peer_bandwidth(payload),
            message::msg_type::USER_CONTROL => self.handle_user_control(payload),
            message::msg_type::COMMAND_AMF0 => {
                let mut reader = payload.reader();
                let mut rst = Vec::<amf::amf0::Value>::new();
                while let Ok(v) = amf::amf0::decoder::from_bytes(&mut reader) {
                    rst.append(&mut vec!(v));
                }

                Ok(Some(message::Message::Command { cs_id, payload: amf::Value::Amf0Value(amf::amf0::Value::StrictArray(rst)) }))
            }
            message::msg_type::COMMAND_AMF3 => {
                let mut reader = payload.reader();
                let mut rst = Vec::<amf::amf3::Value>::new();
                while let Ok(v) = amf::amf3::decoder::from_bytes(&mut reader) {
                    rst.append(&mut vec!(v));
                }

                Ok(Some(message::Message::Command { cs_id, payload: amf::Value::Amf3Value(amf::amf3::Value::Array(rst)) }))
            }
            message::msg_type::DATA_AMF0 => {
                let mut reader = payload.reader();
                let mut rst = Vec::<amf::amf0::Value>::new();
                while let Ok(v) = amf::amf0::decoder::from_bytes(&mut reader) {
                    rst.append(&mut vec!(v));
                }

                Ok(Some(message::Message::Data { cs_id, payload: amf::Value::Amf0Value(amf::amf0::Value::StrictArray(rst)) }))
            }
            message::msg_type::DATA_AMF3 => {
                let mut reader = payload.reader();
                let mut rst = Vec::<amf::amf3::Value>::new();
                while let Ok(v) = amf::amf3::decoder::from_bytes(&mut reader) {
                    rst.append(&mut vec!(v));
                }

                Ok(Some(message::Message::Data { cs_id, payload: amf::Value::Amf3Value(amf::amf3::Value::Array(rst)) }))
            }
            message::msg_type::SHARED_OBJECT_AMF0 => {
                let mut reader = payload.reader();
                let mut rst = Vec::<amf::amf0::Value>::new();
                while let Ok(v) = amf::amf0::decoder::from_bytes(&mut reader) {
                    rst.append(&mut vec!(v));
                }

                Ok(Some(message::Message::SharedObject{ cs_id, payload: amf::Value::Amf0Value(amf::amf0::Value::StrictArray(rst)) }))
            }
            message::msg_type::SHARED_OBJECT_AMF3 => {
                let mut reader = payload.reader();
                let mut rst = Vec::<amf::amf3::Value>::new();
                while let Ok(v) = amf::amf3::decoder::from_bytes(&mut reader) {
                    rst.append(&mut vec!(v));
                }

                Ok(Some(message::Message::SharedObject{ cs_id, payload: amf::Value::Amf3Value(amf::amf3::Value::Array(rst)) }))
            }
            message::msg_type::AUDIO => {
                Ok(Some(message::Message::Audio { cs_id, payload }))
            }
            message::msg_type::VIDEO => {
                Ok(Some(message::Message::Video { cs_id, payload }))
            }
            message::msg_type::AGGREGATE => self.handle_aggregate(cs_id, payload),
            id => Ok(Some(message::Message::Unknown { id, payload}))
        }
    }

    pub fn push(&mut self, _cs_id: u32, _msg: message::Message) {
        // TODO
        unimplemented!()
    }

    fn parse_basic_header(&mut self) {
        let n = self.rd_buf.get_u8();
        self.state = State::BasicHeader {
            fmt: n >> 6,
            cs_id: {
                let mut i = 0b00111111 & n as u32;
                if 0b111111 == n || 0 == n {
                    i = (self.rd_buf.get_u8() as u32) + 64
                }
                if 0b111111 == n {
                    i |= (self.rd_buf.get_u8() as u32) << 8;
                }
                i
            },
        };
    }

    fn handle_set_chunk_size(&mut self, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let chunk_size = payload.get_i32();
        if 1 > chunk_size {
            return Err(ChunkError::WrongInput)
        }

        self.in_chunk_size = chunk_size;
        if chunk_size as usize > self.rd_buf.capacity() {
            self.rd_buf.reserve((chunk_size as usize) - self.rd_buf.capacity())
        }

        Ok(Some(message::Message::SetChunkSize { chunk_size }))
    }

    fn handle_abort(&mut self, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let chunk_stream_id = payload.get_u32();

        if let Some(msg) = self.cs_headers.get_mut(&chunk_stream_id) {
            msg.payload.clear()
        }

        Ok(Some(message::Message::Abort { chunk_stream_id }))
    }

    fn handle_ack(&mut self, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let _sequence_number = payload.get_u32();

        eprintln!("Receive: Ack");
        Ok(None)
    }

    fn handle_window_ack_size(&mut self, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let ack_window_size = payload.get_u32();

        self.window_size = ack_window_size;

        eprintln!("Receive: Server BW = {:}", ack_window_size);
        Ok(None)
    }

    fn handle_set_peer_bandwidth(&mut self, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let ack_window_size = payload.get_u32();
        let limit_type = {
            match payload.get_u8() {
                0 => LimitType::Hard,
                1 => LimitType::Soft,
                2 => LimitType::Dynamic,
                n => LimitType::Unknown(n)
            }
        };

        if self.window_size != ack_window_size {
            self.push(2, message::Message::WindowAckSize {
                ack_window_size
            });
        }
        self.window_size = ack_window_size;
        self.limit_type = limit_type;

        eprintln!("Receive: Client BW = {:}, {:?}", ack_window_size, limit_type);
        Ok(None)
    }

    fn handle_user_control(&mut self, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let event_type = payload.get_u16();
        match event_type {
            message::user_control_event::STREAM_BEGIN => {
                let stream_id = payload.get_u32();

                eprintln!("Stream Begin {:}", stream_id);
                Ok(Some(message::Message::UserControl(message::UserControlEvent::StreamBegin { stream_id })))
            }
            message::user_control_event::STREAM_EOF => {
                let stream_id = payload.get_u32();

                eprintln!("Stream EOF {:}", stream_id);
                Ok(Some(message::Message::UserControl(message::UserControlEvent::StreamEoF { stream_id })))
            }
            message::user_control_event::STREAM_DRY => {
                let stream_id = payload.get_u32();

                eprintln!("Stream Dry {:}", stream_id);
                Ok(Some(message::Message::UserControl(message::UserControlEvent::StreamDry { stream_id })))
            }
            message::user_control_event::SET_BUFFER_LENGTH => {
                let stream_id = payload.get_u32();
                let length = payload.get_u32();

                Ok(Some(message::Message::UserControl(message::UserControlEvent::SetBufferLength { stream_id, length })))
            }
            message::user_control_event::STREAM_IS_RECORDED => {
                let stream_id = payload.get_u32();

                eprintln!("Stream isRecorded {:}", stream_id);
                Ok(Some(message::Message::UserControl(message::UserControlEvent::StreamIsRecorded { stream_id })))
            }
            message::user_control_event::PING_REQUEST => {
                let timestamp = payload.get_u32();

                self.push(2, message::Message::UserControl(message::UserControlEvent::PingResponse { timestamp }));

                eprintln!("Ping {:}", timestamp);
                Ok(Some(message::Message::UserControl(message::UserControlEvent::PingRequest { timestamp })))
            }
            message::user_control_event::PING_RESPONSE => {
                let timestamp = payload.get_u32();

                Ok(Some(message::Message::UserControl(message::UserControlEvent::PingResponse { timestamp })))
            }
            id =>  Err(ChunkError::UnknownEvent(message::UserControlEvent::Unknown {id, payload}))
        }
    }

    fn handle_aggregate(&mut self, cs_id: u32, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let mut rst = Vec::<message::Message>::new();
        while 0 < payload.len() {
            let sub_h = message::Header {
                type_id: payload.get_u8(),
                length: {
                    let mut v = 0_u32;
                    for _ in 0..3 {
                        v = v << 8 | (payload.get_u8() as u32)
                    }
                    v
                },
                timestamp: payload.get_u32(),
                stream_id: {
                    let mut v = 0_u32;
                    for _ in 0..3 {
                        v = v << 8 | (payload.get_u8() as u32)
                    }
                    v
                },
                timestamp_delta: None,
            };

            let mut data = BytesMut::with_capacity(sub_h.length as usize);
            data.put(payload.split_to(sub_h.length as usize));

            let _back_pointer = payload.get_u32();

            match self.process_message(cs_id, sub_h, data) {
                Ok(Some(m)) => {
                    rst.append(&mut vec!(m));
                },
                Ok(None) => {}
                Err(e) => return Err(e)
            }
        }

        Ok(Some(message::Message::Aggregate(rst)))
    }
}

#[derive(Debug, Copy, Clone)]
pub enum LimitType {
    Hard,
    Soft,
    Dynamic,
    Unknown(u8)
}
