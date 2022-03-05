The implementation of RTMP Protocol in Rust
===

For practice
- Reading specifications
- Writing article
- Rust Programing language
- Understanding streaming infrastructure

# Why Rust?
- The primary reason is, reusability.
> Most of the programming languages that used in server-side offer native interfaces.
> And, Rust support wasm.
> So, i thought writing protocol processing library in rust can be reusing in next steps.

# Why RTMP?
`RTMP` is traditional, but widely using in these days.
- `WebRTC` is fast, but it was too big project to implement from scratch.(For streaming, need to implement SDP, ICE, RTCP, RTP, etc..)
- `WebCodecs` provide video/audio buffer directly. but, for understanding infrastructure, design from scratch without traditional technology is weired.
- `SRT` provide streaming. but, as i understand, it just 1:1 MPEG2-TS transport. Don't provide streaming specific functions.

- `RTMP` is designed partialy reusable.

> an application-level protocol designed for multiplexing and packetizing multimedia transport streams (such as audio, video, and interactive content) over a suitable transport protocol (such as TCP).

> While RTMP was designed to work with the RTMP Chunk Stream, it can send the messages using any other transport protocol.

And, Full version of `RTMP` specification is not exist...
It means much things can be writing...(Practice writing article)

# TODO
- [x] Accept incoming RTMP live stream
  - [x] Handshake: Basic (5.2. Handshake)
  - [x] Define RTMP context
  - [x] Chunking (5.3. Chunking)
  - [x] Protocol Control Message (5.4. Protocol Control Messages)
  - [x] RTMP Message (6. RTMP Message Formats)
  - [x] AMF0
  - [x] RTMP Command Messages (7. RTMP Command Messages)
  - [x] Undocumented specifications(ex. FCPublish, releaseStream, etc.)

# References
- [RTMP Specification 1.0](https://www.adobe.com/content/dam/acom/en/devnet/rtmp/pdf/rtmp_specification_1.0.pdf)
- [RTMPE Specification](https://www.cs.cmu.edu/~dst/Adobe/Gallery/RTMPE.txt)
- [librtmp](https://github.com/ossrs/librtmp)
