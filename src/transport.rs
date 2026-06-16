use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use crate::protocol::{self, Frame};

#[derive(Clone)]
pub struct SharedWriter {
    inner: Arc<Mutex<TcpStream>>,
}

impl SharedWriter {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            inner: Arc::new(Mutex::new(stream)),
        }
    }

    pub fn write(&self, frame: Frame) -> std::io::Result<()> {
        protocol::write_frame(&mut *self.inner.lock().unwrap(), &frame)
    }
}
