//! Pretty-printed output — implementation lands in Task 8. This is a
//! placeholder so `writer::mod` compiles.

use crate::event::Event;
use crate::writer::EventWriter;
use crate::Result;
use std::io::Write;

/// Placeholder — real implementation in Task 8.
pub struct PrettyWriter<W: Write> {
    _w: W,
}

impl<W: Write> PrettyWriter<W> {
    pub fn new(_w: W) -> Self {
        panic!("PrettyWriter not yet implemented (lands in Task 8)")
    }
}

impl<W: Write> EventWriter for PrettyWriter<W> {
    fn write_event(&mut self, _e: &Event) -> Result<()> {
        unimplemented!()
    }
    fn finish(&mut self) -> Result<()> {
        unimplemented!()
    }
}
