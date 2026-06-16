use crate::clipboard::ClipboardApi;
use crate::protocol::ClipboardPayload;

pub struct Clipboard;

impl Clipboard {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }
}

impl ClipboardApi for Clipboard {
    fn read(&mut self) -> std::io::Result<Option<ClipboardPayload>> {
        Ok(None)
    }

    fn write(&mut self, _payload: &ClipboardPayload) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "clipboard is not implemented on this platform",
        ))
    }
}
