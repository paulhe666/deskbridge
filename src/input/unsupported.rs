use crate::protocol::InputEvent;

pub struct InputSink;

impl InputSink {
    pub fn new() -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "input injection is not supported on this platform",
        ))
    }

    pub fn apply(&mut self, _event: InputEvent) -> std::io::Result<()> {
        Ok(())
    }
}
