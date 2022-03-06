use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::SystemTime;
use bytes::Buf;

use rtmp;

// https://doc.rust-lang.org/book/ch20-03-graceful-shutdown-and-cleanup.html
mod thread_pool {
    use std::thread;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::sync::Mutex;

    enum Message {
        New(Box<dyn FnOnce() + Send + 'static>),
        Terminate,
    }

    struct Worker {
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Worker {
        pub fn new(rx: Arc<Mutex<mpsc::Receiver<Message>>>) -> Self {
            let thread = thread::spawn(move || loop {
                match rx.lock().unwrap().recv().unwrap() {
                    Message::New(job) => {
                        job();
                    }
                    Message::Terminate => {
                        break;
                    }
                }
            });

            Worker {
                thread: Some(thread),
            }
        }
    }

    pub(crate) struct ThreadPool {
        workers: Vec<Worker>,
        tx: mpsc::Sender<Message>,
    }
    
    impl ThreadPool {
        pub fn new(size: usize) -> Self {
            let (tx, rx) = mpsc::channel();
            let rx = Arc::new(Mutex::new(rx));

            let mut workers = Vec::with_capacity(size);
            for _ in 0..size {
                workers.push(Worker::new(Arc::clone(&rx)));
            }

            ThreadPool {
                workers,
                tx,
            }
        }

        pub fn spawn<F>(&self, f: F)
        where
            F: FnOnce() + Send + 'static,
        {
            self.tx.send(Message::New(Box::new(f))).unwrap();
        }
    }

    impl Drop for ThreadPool {
        fn drop(&mut self) {
            for _ in &self.workers {
                self.tx.send(Message::Terminate).unwrap();
            }

            for w in &mut self.workers {
                if let Some(t) = w.thread.take() {
                    t.join().unwrap();
                }
            }
        }
    }
}

struct Connection {
    stream: TcpStream,
    ctx: rtmp::chunk::Chunk,

    prev_timestamp: Option<SystemTime>,
    prev_bytes_in: u32,
    bytes_out: u32,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Self {
        Connection {
            stream,
            ctx: rtmp::chunk::Chunk::new(),

            prev_timestamp: None,
            prev_bytes_in: 0,
            bytes_out: 0,
        }
    }

    pub fn start(&mut self) {
        self.handshaking();
        self.chunk_process();
    }

    pub fn handshaking(&mut self) {
        println!("Handshake Begin");

        let mut ctx = rtmp::handshake::Handshake::new();

        let mut buf = Vec::<u8>::with_capacity(1536);
        buf.insert(0, 0);    // 1 == buf.len(); for reading version
        loop {
            if let Ok(n) = self.stream.read(&mut buf) {
                unsafe { buf.set_len(n) }

                ctx.buffering(buf.as_slice());
            };

            match ctx.consume() {
                Ok(wr) => {
                    let _ = self.stream.write(wr.as_slice());
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

    fn flush(&mut self) {
        if 0 < self.ctx.wr_buf.len() {
            if None != self.prev_timestamp {
                self.bytes_out += self.ctx.wr_buf.len() as u32;
            }

            self.stream.write_all(self.ctx.wr_buf.split().chunk()).unwrap();
        }
    }

    fn chunk_process(&mut self) {
        let mut buf = vec!(0_u8, 128);
        loop {
            self.flush();
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    return
                }
                Ok(n) => {
                    unsafe { buf.set_len(n) }

                    self.ctx.buffering(buf.as_slice());
                }
                _ => {}
            }

            loop {
                match self.ctx.poll() {
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
                                    "connect" =>  self.connect(payload),
                                    "_checkbw" =>  self._checkbw(payload),
                                    "releaseStream" => self.releaseStream(payload),
                                    "FCPublish" => self.FCPublish(payload),
                                    "createStream" => self.createStream(payload),
                                    "publish" => self.publish(payload),
                                    "FCUnpublish" => self.FCUnpublish(payload),
                                    "deleteStream" => self.deleteStream(payload),
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

    // RPC methods
    #[allow(non_snake_case)]
    fn connect(&mut self, packet: amf::Array<amf::Value>) {
        self.ctx.push(2, rtmp::message::Message::WindowAckSize { ack_window_size: 2_500_000 });
        self.ctx.push(2, rtmp::message::Message::SetPeerBandwidth { ack_window_size: 10_000_000, limit_type: rtmp::chunk::LimitType::Dynamic });
        self.ctx.push(2, rtmp::message::Message::SetChunkSize { chunk_size: 256 });

        let transaction_id = &packet[1];
        if let amf::Value::Amf0Value(amf::amf0::Value::Object(obj)) = &packet[2] {
            eprintln!("{:?}({:?})", "connect", obj["app"]);

            self.ctx.push(3, rtmp::message::Message::Command { payload: amf::Array::<amf::Value>::from([
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

            // Determine RTT and bandwidth by reply _checkbw message
            if None == self.prev_timestamp {
                self.flush();

                self.prev_timestamp = Some(SystemTime::now());
                self.prev_bytes_in = self.ctx.bytes_in;
                self.bytes_out = 0;
                self.ctx.push(3, rtmp::message::Message::Command {
                    payload: amf::Array::<amf::Value>::from([
                        amf::Value::Amf0Value(amf::amf0::Value::String("onBWDone".to_string())),
                        amf::Value::Amf0Value(amf::amf0::Value::Number(0.)),
                        amf::Value::Amf0Value(amf::amf0::Value::Null),
                    ])
                });

                self.flush();
            }
        }
    }

    #[allow(non_snake_case)]
    fn _checkbw(&mut self, packet: amf::Array<amf::Value>) {
        if let Some(prev) = self.prev_timestamp {
            match prev.elapsed() {
                Ok(elapsed) => {
                    let secs = elapsed.as_secs_f64();
                    eprintln!("[Estimated BW] RTT: {:?}s In/Out: {:.4?}/{:.4?} KB/S", secs, (self.ctx.bytes_in - self.prev_bytes_in) as f64 / 1024. / secs, self.bytes_out as f64 / 1024. / secs);
                }
                _ => {}
            }
            self.prev_timestamp = None;
        }
    }

    #[allow(non_snake_case)]
    fn releaseStream(&mut self, packet: amf::Array<amf::Value>) {
        if let amf::Value::Amf0Value(amf::amf0::Value::String(stream_key)) = &packet[3] {
            eprintln!("{:?}({:?})", "releaseStream", stream_key);
        }
    }

    #[allow(non_snake_case)]
    fn FCPublish(&mut self, packet: amf::Array<amf::Value>) {
        if let amf::Value::Amf0Value(amf::amf0::Value::String(stream_key)) = &packet[3] {
            eprintln!("{:?}({:?})", "FCPublish", stream_key);
        }
    }

    #[allow(non_snake_case)]
    fn createStream(&mut self, packet: amf::Array<amf::Value>) {
        let transaction_id = &packet[1];
        self.ctx.push(3, rtmp::message::Message::Command { payload: amf::Array::<amf::Value>::from([
            amf::Value::Amf0Value(amf::amf0::Value::String("_result".to_string())),
            transaction_id.clone(),
            amf::Value::Amf0Value(amf::amf0::Value::Null),
            amf::Value::Amf0Value(amf::amf0::Value::Number(1.)),
        ]) })
    }

    #[allow(non_snake_case)]
    fn publish(&mut self, packet: amf::Array<amf::Value>) {
        let name = {
            match &packet[3] {
                amf::Value::Amf0Value(amf::amf0::Value::String(str)) => str.as_str(),
                _ => {
                    eprintln!("Unexpected {:?}", packet);
                    return
                }
            }
        };
        let publish_type = {
            match &packet[4] {
                amf::Value::Amf0Value(amf::amf0::Value::String(str)) => str.as_str(),
                _ => {
                    eprintln!("Unexpected {:?}", packet);
                    return
                }
            }
        };

        eprintln!("{:?}({:?}, {:?})", "publish", name, publish_type);

        self.ctx.push(5, rtmp::message::Message::Command { payload: amf::Array::<amf::Value>::from([
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

    #[allow(non_snake_case)]
    fn FCUnpublish(&mut self, packet: amf::Array<amf::Value>) {
        if let amf::Value::Amf0Value(amf::amf0::Value::String(stream_key)) = &packet[3] {
            eprintln!("{:?}({:?})", "FCUnpublish", stream_key);
        }
    }

    #[allow(non_snake_case)]
    fn deleteStream(&mut self, packet: amf::Array<amf::Value>) {
        if let amf::Value::Amf0Value(amf::amf0::Value::Number(stream_id)) = &packet[3] {
            eprintln!("{:?}({:?})", "deleteStream", stream_id);
        }
    }
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.1.2.7:1935")?;

    let pool = thread_pool::ThreadPool::new(1024);

    for stream in listener.incoming() {
        if let Ok(s) = stream {
            pool.spawn(|| {
                Connection::new(s).start();
            });
        }
    }

    Ok(())
}
