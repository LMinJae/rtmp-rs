use bytes::{BufMut, BytesMut};

// 5.3.1.2. Chunk Message Header
#[derive(Debug)]
pub struct MessagePacket {
    pub header: Header,
    pub payload: BytesMut,
}

impl MessagePacket {
    pub fn new(header: Header) -> Self {
        MessagePacket {
            header,
            payload: BytesMut::with_capacity(header.length as usize),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub timestamp: u32,
    pub length: u32,
    pub type_id: u8,
    pub stream_id: u32,
    pub timestamp_delta: u32,
}

impl Header {
    pub fn write_chunk_message_header<W: BufMut>(&self, mut w: W, fmt: u8) {
        if 3 == fmt { return }

        // timestamp
        if TIMESTAMP_MAX < self.timestamp {
            w.put_u16(0xFFFF);
            w.put_u8(0xFF);
        } else if 0 == fmt{
            w.put_u8(0xFF & (self.timestamp >> 16) as u8);
            w.put_u8(0xFF & (self.timestamp >> 8) as u8);
            w.put_u8(0xFF & (self.timestamp) as u8);
        } else {
            w.put_u8(0xFF & (self.timestamp_delta >> 16) as u8);
            w.put_u8(0xFF & (self.timestamp_delta >> 8) as u8);
            w.put_u8(0xFF & (self.timestamp_delta) as u8);
        }

        if 2 > fmt {
            // message length
            w.put_u8(0xFF & (self.length >> 16) as u8);
            w.put_u8(0xFF & (self.length >> 8) as u8);
            w.put_u8(0xFF & (self.length) as u8);

            // message type_id
            w.put_u8(self.type_id);

            if 1 > fmt {
                // message stream id
                w.put_u32(self.stream_id);
            }
        }

        // extended timestamp
        if TIMESTAMP_MAX < self.timestamp {
            if 0 == fmt {
                w.put_u32(self.timestamp);
            } else {
                w.put_u32(self.timestamp_delta);
            }
        }
    }
}

pub const TIMESTAMP_MAX             : u32 = 0xFFFFFF;

pub mod msg_type {
    #![allow(dead_code)]
    pub const SET_CHUNK_SIZE        : u8 = 0x01;
    pub const ABORT                 : u8 = 0x02;
    pub const ACK                   : u8 = 0x03;
    pub const USER_CONTROL          : u8 = 0x04;
    pub const WINDOW_ACK_SIZE       : u8 = 0x05;
    pub const SET_PEER_BANDWIDTH    : u8 = 0x06;

    pub const AUDIO                 : u8 = 0x08;
    pub const VIDEO                 : u8 = 0x09;

    pub const DATA_AMF3             : u8 = 0x0F;
    pub const SHARED_OBJECT_AMF3    : u8 = 0x10;
    pub const COMMAND_AMF3          : u8 = 0x11;
    pub const DATA_AMF0             : u8 = 0x12;
    pub const SHARED_OBJECT_AMF0    : u8 = 0x13;
    pub const COMMAND_AMF0          : u8 = 0x14;

    pub const AGGREGATE             : u8 = 0x16;

    pub const SILENT_RECONNECT      : u8 = 0x20;
}

pub mod user_control_event {
    #![allow(dead_code)]
    pub const STREAM_BEGIN          : u16 = 0;
    pub const STREAM_EOF            : u16 = 1;
    pub const STREAM_DRY            : u16 = 2;
    pub const SET_BUFFER_LENGTH     : u16 = 3;
    pub const STREAM_IS_RECORDED    : u16 = 4;
    pub const PING_REQUEST          : u16 = 6;
    pub const PING_RESPONSE         : u16 = 7;

    // FMS 3.5
    pub const STREAM_BUFFER_EMPTY   : u16 = 31;
    pub const STREAM_BUFFER_READY   : u16 = 32;
}

#[derive(Debug)]
pub enum UserControlEvent {
    StreamBegin { stream_id: u32 },
    StreamEoF { stream_id: u32 },
    StreamDry { stream_id: u32 },
    SetBufferLength { stream_id: u32, length: u32 },
    StreamIsRecorded { stream_id: u32 },
    PingRequest { timestamp: u32 },
    PingResponse { timestamp: u32 },
    Unknown { id: u16, payload: BytesMut }
}

#[derive(Debug)]
pub enum Message {
    SetChunkSize { chunk_size: i32 },
    Abort { chunk_stream_id: u32 },
    Ack { sequence_number: u32 },
    WindowAckSize { ack_window_size: u32 },
    SetPeerBandwidth { ack_window_size: u32, limit_type: crate::chunk::LimitType },

    UserControl(UserControlEvent),

    Command { payload: amf::Array<amf::Value> },
    Data { payload: amf::Array<amf::Value> },
    SharedObject { header: Header, payload: BytesMut },

    Audio { delta: u32, control: u8, payload: BytesMut },
    Video { delta: u32, control: u8, payload: BytesMut },

    Aggregate(Vec<Message>),

    Unknown { id: u8, payload: BytesMut }
}
