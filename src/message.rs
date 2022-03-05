
use bytes::BytesMut;

// 5.3.1.2. Chunk Message Header
#[derive(Debug)]
pub struct MessagePacket {
    pub header: Header,
    pub payload: BytesMut,
}

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub timestamp: u32,
    pub length: u32,
    pub type_id: u8,
    pub stream_id: u32,
    pub timestamp_delta: Option<u32>,
}

pub const TIMESTAMP_MAX             : u32 = 0xFFFFFF;

pub mod msg_type {
    #![allow(dead_code)]
    pub const SET_CHUNK_SIZE        : u8 =  1;
    pub const ABORT                 : u8 =  2;
    pub const ACK                   : u8 =  3;
    pub const WINDOW_ACK_SIZE       : u8 =  5;
    pub const SET_PEER_BANDWIDTH    : u8 =  6;

    pub const USER_CONTROL          : u8 =  4;

    pub const COMMAND_AMF0          : u8 = 20;
    pub const COMMAND_AMF3          : u8 = 17;

    pub const DATA_AMF0             : u8 = 18;
    pub const DATA_AMF3             : u8 = 15;

    pub const SHARED_OBJECT_AMF0    : u8 = 19;
    pub const SHARED_OBJECT_AMF3    : u8 = 16;

    pub const AUDIO                 : u8 =  8;
    pub const VIDEO                 : u8 =  9;

    pub const AGGREGATE             : u8 = 22;
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

    Command { cs_id: u32, payload: amf::Value },
    Data { cs_id: u32, payload: amf::Value },
    SharedObject { cs_id: u32, payload: amf::Value },

    Audio { cs_id: u32, payload: BytesMut },
    Video { cs_id: u32, payload: BytesMut },

    Aggregate(Vec<Message>),

    Unknown { id: u8, payload: BytesMut }
}
