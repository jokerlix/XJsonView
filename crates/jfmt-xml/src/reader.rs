use crate::Result;
use std::io::Read;

pub struct EventReader<R: Read> {
    _reader: R,
}

impl<R: Read> EventReader<R> {
    pub fn new(reader: R) -> Self {
        Self { _reader: reader }
    }

    pub fn next_event(&mut self) -> Result<Option<crate::XmlEvent>> {
        Ok(None)
    }
}
