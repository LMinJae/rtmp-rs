use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use bytes::Buf;

use rtmp;

fn handshaking(mut stream: &TcpStream) {
    println!("Handshake Begin");

    let mut ctx = rtmp::handshake::Handshake::new();

    let mut buf = Vec::<u8>::with_capacity(1536);
    buf.insert(0, 0);    // 1 == buf.len(); for reading version
    loop {
        if let Ok(n) = stream.read(&mut buf) {
            unsafe { buf.set_len(n) }

            ctx.buffering(buf.as_slice());
        };

        match ctx.consume() {
            Ok(wr) => {
                let _ = stream.write(wr.as_slice());
            },
            Err(rtmp::handshake::HandshakeError::Done) => break,
            Err(e) => {
                eprintln!("Error while handshaking: {:?}", e);
                return
            }
        }

        unsafe { buf.set_len(buf.capacity()) };
    }

    println!("Handshake Done");
}

fn chunk_process(mut stream: &TcpStream) {
    let mut ctx = rtmp::chunk::Chunk::new();
    ctx.push(2, rtmp::message::Message::WindowAckSize { ack_window_size: 2_500_000 });
    ctx.push(2, rtmp::message::Message::SetPeerBandwidth { ack_window_size: 10_000_000, limit_type: rtmp::chunk::LimitType::Dynamic });
    ctx.push(2, rtmp::message::Message::SetChunkSize { chunk_size: 256 });

    let mut buf = vec!(0_u8, 128);
    loop {
        if 0 < ctx.wr_buf.len() {
            stream.write_all(ctx.wr_buf.split().chunk()).unwrap();
        }
        match stream.read(&mut buf) {
            Ok(0) => {
                return
            }
            Ok(n) => {
                unsafe { buf.set_len(n) }

                ctx.buffering(buf.as_slice());
            }
            _ => {}
        }

        loop {
            match ctx.poll() {
                Ok(None) => {
                    break
                }
                Ok(Some(msg)) => {
                    match msg {
                        rtmp::message::Message::SetChunkSize { chunk_size } => {
                            if 1 + chunk_size as usize > buf.capacity() {
                                buf.reserve_exact(1 + (chunk_size as usize) - buf.capacity())
                            }
                        }
                        rtmp::message::Message::Command { payload } => {
                            let cmd = {
                                match &payload[0] {
                                    amf::Value::Amf0Value(amf::amf0::Value::String(str)) => str.as_str(),
                                    _ => {
                                        eprintln!("Unexpected {:?}", payload);
                                        return
                                    }
                                }
                            };
                            let transaction_id = &payload[1];
                            match cmd {
                                "connect" => {
                                    if let amf::Value::Amf0Value(amf::amf0::Value::Object(obj)) = &payload[2] {
                                        eprintln!("{:?}({:?})", cmd, obj["app"]);

                                        ctx.push(3, rtmp::message::Message::Command { payload: amf::Array::<amf::Value>::from([
                                            amf::Value::Amf0Value(amf::amf0::Value::String("_result".to_string())),
                                            transaction_id.clone(),
                                            amf::Value::Amf0Value(amf::amf0::Value::Object(amf::Object {
                                                class_name: "".to_string(),
                                                property: amf::Property::from([
                                                    ("fmsVer".to_string(), amf::amf0::Value::String("FMS/3,5,3,824".to_string())),
                                                    ("capabilities".to_string(), amf::amf0::Value::Number(127.)),
                                                    ("mode".to_string(), amf::amf0::Value::Number(1.)),
                                                ])
                                            })),
                                            amf::Value::Amf0Value(amf::amf0::Value::Object(amf::Object {
                                                class_name: "".to_string(),
                                                property: amf::Property::from([
                                                    ("level".to_string(), amf::amf0::Value::String("status".to_string())),
                                                    ("code".to_string(), amf::amf0::Value::String("NetConnection.Connect.Success".to_string())),
                                                    ("description".to_string(), amf::amf0::Value::String("Connection succeeded.".to_string())),
                                                    ("objectEncoding".to_string(), amf::amf0::Value::Number(0.)),
                                                    ("data".to_string(), amf::amf0::Value::ECMAArray(amf::Property::from([
                                                        ("version".to_string(), amf::amf0::Value::String("3,5,3,824".to_string())),
                                                    ]))),
                                                ])
                                            })),
                                        ]) });

                                        ctx.push(3, rtmp::message::Message::Command { payload: amf::Array::<amf::Value>::from([
                                            amf::Value::Amf0Value(amf::amf0::Value::String("onBWDone".to_string())),
                                            amf::Value::Amf0Value(amf::amf0::Value::Number(0.)),
                                            amf::Value::Amf0Value(amf::amf0::Value::Null),
                                        ]) });
                                    }
                                }
                                "releaseStream" => {
                                    if let amf::Value::Amf0Value(amf::amf0::Value::String(stream_key)) = &payload[3] {
                                        eprintln!("{:?}({:?})", cmd, stream_key);
                                    }
                                }
                                "FCPublish" => {
                                    if let amf::Value::Amf0Value(amf::amf0::Value::String(stream_key)) = &payload[3] {
                                        eprintln!("{:?}({:?})", cmd, stream_key);
                                    }
                                }
                                "createStream" => {
                                    ctx.push(3, rtmp::message::Message::Command { payload: amf::Array::<amf::Value>::from([
                                        amf::Value::Amf0Value(amf::amf0::Value::String("_result".to_string())),
                                        transaction_id.clone(),
                                        amf::Value::Amf0Value(amf::amf0::Value::Null),
                                        amf::Value::Amf0Value(amf::amf0::Value::Number(1.)),
                                    ]) })
                                }
                                "publish" => {
                                    let name = {
                                        match &payload[3] {
                                            amf::Value::Amf0Value(amf::amf0::Value::String(str)) => str.as_str(),
                                            _ => {
                                                eprintln!("Unexpected {:?}", payload);
                                                return
                                            }
                                        }
                                    };
                                    let publish_type = {
                                        match &payload[4] {
                                            amf::Value::Amf0Value(amf::amf0::Value::String(str)) => str.as_str(),
                                            _ => {
                                                eprintln!("Unexpected {:?}", payload);
                                                return
                                            }
                                        }
                                    };

                                    eprintln!("{:?}({:?}, {:?})", cmd, name, publish_type);

                                    ctx.push(5, rtmp::message::Message::Command { payload: amf::Array::<amf::Value>::from([
                                        amf::Value::Amf0Value(amf::amf0::Value::String("onStatus".to_string())),
                                        amf::Value::Amf0Value(amf::amf0::Value::Number(0.)),
                                        amf::Value::Amf0Value(amf::amf0::Value::Null),
                                        amf::Value::Amf0Value(amf::amf0::Value::Object(amf::Object {
                                            class_name: "".to_string(),
                                            property: amf::Property::from([
                                                ("level".to_string(), amf::amf0::Value::String("status".to_string())),
                                                ("code".to_string(), amf::amf0::Value::String("NetStream.Publish.Start".to_string())),
                                            ])
                                        })),
                                    ]) })
                                }
                                "FCUnpublish" => {
                                    if let amf::Value::Amf0Value(amf::amf0::Value::String(stream_key)) = &payload[3] {
                                        eprintln!("{:?}({:?})", cmd, stream_key);
                                    }
                                }
                                "deleteStream" => {
                                    if let amf::Value::Amf0Value(amf::amf0::Value::Number(stream_id)) = &payload[3] {
                                        eprintln!("{:?}({:?})", cmd, stream_id);
                                    }
                                }
                                _ => {
                                    eprintln!("{:?} {:?} {:?}", cmd, transaction_id, payload)
                                }
                            }
                        }
                        rtmp::message::Message::Data { payload } => {
                            let p0 = {
                                match &payload[0] {
                                    amf::Value::Amf0Value(amf::amf0::Value::String(str)) => str.as_str(),
                                    _ => {
                                        eprintln!("Unexpected {:?}", payload);
                                        return
                                    }
                                }
                            };
                            match p0 {
                                "@setDataFrame" => {
                                    let p1 = {
                                        match &payload[1] {
                                            amf::Value::Amf0Value(amf::amf0::Value::String(str)) => str.as_str(),
                                            _ => {
                                                eprintln!("Unexpected {:?}", payload);
                                                return
                                            }
                                        }
                                    };
                                    let p2 = {
                                        match &payload[2] {
                                            amf::Value::Amf0Value(amf::amf0::Value::ECMAArray(arr)) => arr,
                                            _ => {
                                                eprintln!("Unexpected {:?}", payload);
                                                return
                                            }
                                        }
                                    };
                                    eprintln!("{:?} {:?} {:?}", p0, p1, p2);
                                }
                                _ => {
                                    eprintln!("Unexpected {:?}", payload);
                                }
                            };
                        }
                        rtmp::message::Message::Audio { control, payload: _ } => {
                            let _codec = control >> 4;
                            let _rate = (control >> 2) & 3;
                            let _size = (control >> 1) & 1;
                            let _channel = control & 1;
                        }
                        rtmp::message::Message::Video { control, payload: _ } => {
                            let _frame = control >> 4;
                            let _codec = control & 0xF;
                        }
                        _ => {
                            eprintln!("{:?}", msg)
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error while chunk processing: {:?}", e);
                    return
                }
            }
        }

        unsafe { buf.set_len(buf.capacity()) };
    }
}

fn handle_client(mut stream: TcpStream) {
    handshaking(&mut stream);
    chunk_process(&mut stream);
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.1.2.7:1935")?;

    for stream in listener.incoming() {
        handle_client(stream?)
    }
    Ok(())
}
