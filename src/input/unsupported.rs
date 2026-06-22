use crate::platform::ConnectionProfile;
use crate::protocol::InputEvent;

pub struct InputSink;

impl InputSink {
    pub fn new(_profile: ConnectionProfile) -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "input injection is not supported on this platform",
        ))
    }

    pub fn apply(&mut self, _event: InputEvent) -> std::io::Result<()> {
        Ok(())
    }

    pub fn screen_size(&self) -> (u32, u32) {
        (0, 0)
    }
}

pub fn screen_size() -> (u32, u32) {
    (0, 0)
}
