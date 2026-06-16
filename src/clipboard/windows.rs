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
        // Full Windows implementation belongs here:
        // CF_UNICODETEXT, CF_DIB, and CF_HDROP.
        Ok(None)
    }

    fn write(&mut self, _payload: &ClipboardPayload) -> std::io::Result<()> {
        Ok(())
    }
}
