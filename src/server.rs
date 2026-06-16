use std::net::TcpListener;

use crate::protocol::{self, Frame, FrameKind};

pub fn run(bind: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind)?;
    eprintln!("deskbridge server listening on {bind}");
    let (mut stream, addr) = listener.accept()?;
    eprintln!("client connected from {addr}");
    protocol::write_frame(
        &mut stream,
        &Frame::new(FrameKind::Hello, protocol::hello_payload()),
    )?;

    // Windows server work lands here:
    // 1. global mouse/keyboard hooks
    // 2. edge detection and cursor lock/release
    // 3. clipboard watcher for text / CF_DIB / CF_HDROP
    // 4. file drag source and file transfer stream
    loop {
        let frame = protocol::read_frame(&mut stream)?;
        if frame.kind == FrameKind::Hello {
            eprintln!("protocol hello received");
        }
    }
}
