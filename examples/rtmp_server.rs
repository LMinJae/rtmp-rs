use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use rtmp::handshake::{Handshake, HandshakeError};

fn handshaking(mut stream: &TcpStream) {
    println!("Handshake Begin");

    let mut ctx = Handshake::new();

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
            Err(HandshakeError::Done) => break,
            Err(e) => {
                eprintln!("Error while handshaking: {:?}", e);
            }
        }

        unsafe { buf.set_len(buf.capacity()) };
    }

    println!("Handshake Done");
}

fn handle_client(mut stream: TcpStream) {
    handshaking(&mut stream);
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.1.2.7:1935")?;

    for stream in listener.incoming() {
        handle_client(stream?)
    }
    Ok(())
}
