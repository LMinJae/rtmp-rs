use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

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

    let mut buf = vec!(0_u8, 128);
    loop {
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
                Ok(Some(chunk)) => {
                    match chunk.msg {
                        rtmp::chunk::MessageData::SetChunkSize { chunk_size } => {
                            if chunk_size as usize > buf.capacity() {
                                buf.reserve_exact((chunk_size as usize) - buf.capacity());
                            }
                },
                _ => {}
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
