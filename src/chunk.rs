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
    bytes_in: u32,
    bytes_in_sent: u32,
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
            cs_headers: HashMap::with_capacity(4),
            bytes_in: 0,
            bytes_in_sent: 0,
        }
    }

    pub fn get_write_buffer_length(&self) -> usize {
        self.wr_buf.len()
    }

    pub fn flush_write_buffer(&mut self) -> BytesMut {
        self.wr_buf.split()
    }

    pub fn get_bytes_in(&self) -> u32 {
        self.bytes_in
    }

    pub fn buffering(&mut self, buf: &[u8]) {
        self.rd_buf.extend(buf);
        self.bytes_in += buf.len() as u32;
        if self.bytes_in > self.bytes_in_sent + self.window_size / 10 {
            self.bytes_in_sent = self.bytes_in;
            self.push(2, message::Message::Ack { sequence_number: self.bytes_in })
        }
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
                                msg.header.timestamp_delta = timestamp;
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
                                msg.header.timestamp_delta = self.rd_buf.get_u32()
                            }
                        }
                    } else if 0x00 == fmt {
                        let header = message::Header {
                            timestamp: read_u24!(),
                            length: read_u24!(),
                            type_id: self.rd_buf.get_u8(),
                            stream_id: self.rd_buf.get_u32(),
                            timestamp_delta: 0,
                        };

                        self.cs_headers.insert(cs_id, message::MessagePacket::new(header));
                    } else {
                        return Err(ChunkError::UnknownChunkStream)
                    }
                }
                self.state = State::ChunkMessageHeader { cs_id }
            }
            if let State::ChunkMessageHeader { cs_id } = self.state {
                if let Some(msg) = self.cs_headers.get_mut(&cs_id) {
                    msg.header.timestamp += msg.header.timestamp_delta;

                    let length = msg.header.length as usize;

                    let left = min(self.in_chunk_size as usize - 1, length - msg.payload.len());
                    if left > self.rd_buf.len() {
                        return Ok(None)
                    }
                    msg.payload.put(self.rd_buf.split_to(left));

                    if msg.payload.len() != length {
                        self.state = State::Uninitialized;
                        return Ok(None)
                    }
                }

                let rst = {
                    let (header, payload) = {
                        let msg = self.cs_headers.get(&cs_id).unwrap().clone();
                        (msg.header.clone(), msg.payload.clone())
                    };
                    self.process_message(header, payload)
                };

                if let Some(msg) = self.cs_headers.get_mut(&cs_id) {
                    msg.payload.clear()
                }

                self.state = State::Uninitialized;
                return rst
            }
        }
    }

    fn process_message(&mut self, header: message::Header, mut payload: BytesMut) -> Result<Option<message::Message>, ChunkError> {
        let msg = match header.type_id {
            message::msg_type::SET_CHUNK_SIZE => {
                let chunk_size = payload.get_i32();

                message::Message::SetChunkSize { chunk_size }
            }
            message::msg_type::ABORT => {
                let chunk_stream_id = payload.get_u32();

                message::Message::Abort { chunk_stream_id }
            }

            message::msg_type::ACK => {
                let sequence_number = payload.get_u32();

                message::Message::Ack { sequence_number }
            }
            message::msg_type::WINDOW_ACK_SIZE => {
                let ack_window_size = payload.get_u32();

                message::Message::WindowAckSize { ack_window_size }
            }
            message::msg_type::SET_PEER_BANDWIDTH => {
                let ack_window_size = payload.get_u32();
                let limit_type = {
                    match payload.get_u8() {
                        0 => LimitType::Hard,
                        1 => LimitType::Soft,
                        2 => LimitType::Dynamic,
                        n => LimitType::Unknown(n)
                    }
                };

                message::Message::SetPeerBandwidth { ack_window_size, limit_type }
            }
            message::msg_type::USER_CONTROL => {
                let event_type = payload.get_u16();
                match event_type {
                    message::user_control_event::STREAM_BEGIN => {
                        let stream_id = payload.get_u32();

                        message::Message::UserControl(message::UserControlEvent::StreamBegin { stream_id })
                    }
                    message::user_control_event::STREAM_EOF => {
                        let stream_id = payload.get_u32();

                        message::Message::UserControl(message::UserControlEvent::StreamEoF { stream_id })
                    }
                    message::user_control_event::STREAM_DRY => {
                        let stream_id = payload.get_u32();

                        message::Message::UserControl(message::UserControlEvent::StreamDry { stream_id })
                    }
                    message::user_control_event::SET_BUFFER_LENGTH => {
                        let stream_id = payload.get_u32();
                        let length = payload.get_u32();

                        message::Message::UserControl(message::UserControlEvent::SetBufferLength { stream_id, length })
                    }
                    message::user_control_event::STREAM_IS_RECORDED => {
                        let stream_id = payload.get_u32();

                        message::Message::UserControl(message::UserControlEvent::StreamIsRecorded { stream_id })
                    }
                    message::user_control_event::PING_REQUEST => {
                        let timestamp = payload.get_u32();


                        message::Message::UserControl(message::UserControlEvent::PingRequest { timestamp })
                    }
                    message::user_control_event::PING_RESPONSE => {
                        let timestamp = payload.get_u32();

                        message::Message::UserControl(message::UserControlEvent::PingResponse { timestamp })
                    }
                    id =>  message::Message::UserControl(message::UserControlEvent::Unknown {id, payload})
                }
            }
            message::msg_type::COMMAND_AMF0 => {
                let rst = Chunk::parse_amf0_packet(payload);

                message::Message::Command { payload: rst }
            }
            message::msg_type::DATA_AMF0 => {
                let rst = Chunk::parse_amf0_packet(payload);

                message::Message::Data { payload: rst }
            }
            message::msg_type::AUDIO => {
                message::Message::Audio { control: payload.get_u8(), payload }
            }
            message::msg_type::VIDEO => {
                message::Message::Video { control: payload.get_u8(), payload }
            }
            message::msg_type::AGGREGATE => {
                let mut rst = Vec::<message::Message>::new();
                while 0 < payload.len() {
                    let mut sub = message::MessagePacket::new(message::Header {
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
                        timestamp_delta: 0,
                    });

                    sub.payload.put(payload.split_to(sub.header.length as usize));

                    let _back_pointer = payload.get_u32();

                    match self.process_message(sub.header, sub.payload) {
                        Ok(Some(m)) => {
                            rst.append(&mut vec!(m));
                        },
                        Ok(None) => {}
                        Err(e) => return Err(e)
                    }
                }

                message::Message::Aggregate(rst)
            }
            id => message::Message::Unknown { id, payload}
        };

        match msg {
            message::Message::SetChunkSize { chunk_size } => {
                if 1 > chunk_size {
                    return Err(ChunkError::WrongInput)
                }

                self.in_chunk_size = chunk_size + 1;
                if chunk_size as usize > self.rd_buf.capacity() {
                    self.rd_buf.reserve((chunk_size as usize) - self.rd_buf.capacity())
                }

                Ok(Some(message::Message::SetChunkSize { chunk_size }))
            }
            message::Message::Abort { chunk_stream_id } => {
                if let Some(msg) = self.cs_headers.get_mut(&chunk_stream_id) {
                    msg.payload.clear()
                }

                Ok(None)
            }
            message::Message::Ack { sequence_number } => {
                eprintln!("Receive: Ack @ {:}", sequence_number);
                Ok(None)
            }
            message::Message::WindowAckSize { ack_window_size } => {
                self.window_size = ack_window_size;

                eprintln!("Receive: Server BW = {:}", ack_window_size);
                Ok(None)
            }
            message::Message::SetPeerBandwidth { ack_window_size, limit_type } => {
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
            message::Message::UserControl(message::UserControlEvent::PingRequest { timestamp }) => {
                self.push(2, message::Message::UserControl(message::UserControlEvent::PingResponse { timestamp }));

                Ok(None)
            }
            _ => Ok(Some(msg))
        }
    }

    pub fn push(&mut self, cs_id: u32, msg: message::Message) {
        let (head, mut body) = match msg {
            message::Message::SetChunkSize { chunk_size } => {
                if 128 > chunk_size {
                    // Fail occur when short command message processing
                    return
                }

                self.out_chunk_size = chunk_size + 1;

                let mut buf = BytesMut::with_capacity(self.out_chunk_size as usize);

                // Basic header
                Chunk::write_basic_header(&mut buf, 0, cs_id);

                // Chunk Message Header - Type 0
                // timestamp + message length(0)
                buf.put_u32(0);
                // message length(1..2)
                buf.put_u16(4);
                // message type_id
                buf.put_u8(message::msg_type::SET_CHUNK_SIZE);
                // message stream id
                buf.put_u32(0);

                let head = buf.split();

                // Body
                buf.put_i32(chunk_size);

                (head, buf)
            }
            message::Message::Ack { sequence_number } => {
                let mut buf = BytesMut::with_capacity(self.out_chunk_size as usize);

                // Basic header
                Chunk::write_basic_header(&mut buf, 1, cs_id);

                // Chunk Message Header - Type 1
                // timestamp + message length(0)
                buf.put_u32(0);
                // message length(1..2)
                buf.put_u16(4);
                // message type_id
                buf.put_u8(message::msg_type::ACK);

                let head = buf.split();

                // Body
                buf.put_u32(sequence_number);

                (head, buf)
            }
            message::Message::WindowAckSize { ack_window_size } => {
                let mut buf = BytesMut::with_capacity(self.out_chunk_size as usize);

                // Basic header
                Chunk::write_basic_header(&mut buf, 0, cs_id);

                // Chunk Message Header - Type 0
                // timestamp + message length(0)
                buf.put_u32(0);
                // message length(1..2)
                buf.put_u16(4);
                // message type_id
                buf.put_u8(message::msg_type::WINDOW_ACK_SIZE);
                // message stream id
                buf.put_u32(0);

                let head = buf.split();

                // Body
                buf.put_u32(ack_window_size);

                (head, buf)
            }
            message::Message::SetPeerBandwidth { ack_window_size, limit_type } => {
                let mut buf = BytesMut::with_capacity(self.out_chunk_size as usize);

                // Basic header
                Chunk::write_basic_header(&mut buf, 0, cs_id);

                // Chunk Message Header - Type 0
                // timestamp + message length(0)
                buf.put_u32(0);
                // message length(1..2)
                buf.put_u16(5);
                // message type_id
                buf.put_u8(message::msg_type::SET_PEER_BANDWIDTH);
                // message stream id
                buf.put_u32(0);

                let head = buf.split();

                // Body
                buf.put_u32(ack_window_size);
                buf.put_u8(match limit_type {
                    LimitType::Hard => 0,
                    LimitType::Soft => 1,
                    _ => 2
                });

                (head, buf)
            }
            message::Message::Command { payload } => {
                let mut buf = BytesMut::with_capacity(self.out_chunk_size as usize);

                // Basic header
                Chunk::write_basic_header(&mut buf, 0, cs_id);

                // Chunk Message Header - Type 0
                // timestamp
                buf.put_u16(0);
                buf.put_u8(0);
                // message length
                let mut cmd = Vec::<u8>::new();
                for i in payload {
                    let amf::Value::Amf0Value(v) = i;
                    amf::amf0::encoder::to_bytes(&mut cmd, &v).unwrap();
                }
                let len = cmd.len();
                buf.put_u16((len >> 8) as u16);
                buf.put_u8((len & 0xff) as u8);
                // message type_id
                buf.put_u8(message::msg_type::COMMAND_AMF0);
                // message stream id
                buf.put_u32(0);

                let head = buf.split();

                // Body
                buf.put(cmd.as_slice());

                (head, buf)
            }
            _ => {
                unimplemented!()
            }
        };

        self.wr_buf.put(head);
        if 4 <= self.out_chunk_size {
            self.wr_buf.put(body);
        } else {
            self.wr_buf.put(body.split_to(self.out_chunk_size as usize));
            while 0 < body.len() {
                Chunk::write_basic_header(&mut self.wr_buf, 3, cs_id);
                self.wr_buf.put(body.split_to(min(body.len(), self.out_chunk_size as usize)))
            }
        }
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

    fn write_basic_header<R: BufMut>(mut w: R, fmt: u8, cs_id: u32) {
        if 64 > cs_id {
            w.put_u8((fmt << 6) | (cs_id) as u8)
        } else {
            let tmp = cs_id - 64;
            /*  */ if 319 > cs_id {
                w.put_u8(fmt << 6);
                w.put_u8(tmp as u8);
            } else {
                w.put_u8((fmt << 6) | 0b00111111);
                w.put_u16(tmp as u16);
            }
        }
    }

    fn parse_amf0_packet<R: Buf>(payload: R) -> amf::Array::<amf::Value> {
        let mut reader = payload.reader();
        let mut rst = amf::Array::<amf::Value>::new();
        while let Ok(v) = amf::amf0::decoder::from_bytes(&mut reader) {
            rst.append(&mut vec!(amf::Value::Amf0Value(v)));
        }
        rst
    }
}

#[derive(Debug, Copy, Clone)]
pub enum LimitType {
    Hard,
    Soft,
    Dynamic,
    Unknown(u8)
}
