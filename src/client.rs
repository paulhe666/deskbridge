use std::net::TcpStream;
use std::path::PathBuf;

use crate::clipboard::{Clipboard, ClipboardApi};
use crate::file_transfer;
use crate::input::InputSink;
use crate::protocol::{self, FrameKind};
use crate::transport::SharedWriter;

pub fn run(server: &str) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(server)?;
    stream.set_nodelay(true)?;
    let writer = SharedWriter::new(stream.try_clone()?);
    writer.write(crate::protocol::Frame::new(
        FrameKind::Hello,
        protocol::hello_payload(),
    ))?;

    let mut input = InputSink::new()?;
    let mut clipboard = Clipboard::new()?;
    let receive_root = std::env::temp_dir().join("deskbridge-received");
    let mut receiving_file = None;

    loop {
        let frame = protocol::read_frame(&mut stream)?;
        match frame.kind {
            FrameKind::Input => input.apply(protocol::decode_input(&frame.payload)?)?,
            FrameKind::Clipboard => {
                let payload = protocol::decode_clipboard(&frame.payload)?;
                clipboard.write(&payload)?;
            }
            FrameKind::FileStart => {
                let (relative, len) = protocol::decode_file_start(&frame.payload)?;
                receiving_file = Some(file_transfer::start_receive(&receive_root, &relative, len)?);
            }
            FrameKind::FileChunk => {
                if let Some(file) = receiving_file.as_mut() {
                    if file.write_chunk(&frame.payload)? {
                        receiving_file = None;
                    }
                }
            }
            FrameKind::DragEnd => {
                let files = vec![PathBuf::from(&receive_root)];
                clipboard.write(&crate::protocol::ClipboardPayload::Files(files))?;
            }
            _ => {}
        }
    }
}
