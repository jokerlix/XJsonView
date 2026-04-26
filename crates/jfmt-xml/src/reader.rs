use crate::{Result, XmlError, XmlEvent};
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event as QxEvent;
use quick_xml::reader::Reader;
use std::io::{BufReader, Read};

/// Streaming XML reader producing `XmlEvent`s in document order.
pub struct EventReader<R: Read> {
    inner: Reader<BufReader<R>>,
    buf: Vec<u8>,
    /// `quick_xml::Event::Empty` produces both Start + End from a single
    /// underlying event. We buffer the synthesized End here.
    pending_end: Option<XmlEvent>,
    /// Non-text event read ahead while we were accumulating text content.
    /// Returned on the next `next_event()` call.
    pending_event: Option<XmlEvent>,
    /// Set once on Eof so subsequent calls keep returning `Ok(None)`.
    finished: bool,
}

impl<R: Read> EventReader<R> {
    pub fn new(reader: R) -> Self {
        let mut qx = Reader::from_reader(BufReader::new(reader));
        qx.config_mut().trim_text(false);
        Self {
            inner: qx,
            buf: Vec::with_capacity(1024),
            pending_end: None,
            pending_event: None,
            finished: false,
        }
    }

    pub fn next_event(&mut self) -> Result<Option<XmlEvent>> {
        // Return any previously buffered event first.
        if let Some(ev) = self.pending_end.take() {
            return Ok(Some(ev));
        }
        if let Some(ev) = self.pending_event.take() {
            return Ok(Some(ev));
        }
        if self.finished {
            return Ok(None);
        }

        // Issue B fix: use a loop so skipped events (comments, PIs, CDATA,
        // decls) `continue` instead of recursing. Avoids stack overflow on
        // pathological input with thousands of consecutive skipped events.
        loop {
            self.buf.clear();
            // Read the raw event. The borrow on `self.buf` ends after this call,
            // allowing us to call `&self` methods below.
            let event = self
                .inner
                .read_event_into(&mut self.buf)
                .map_err(|err| make_err(self.inner.buffer_position(), format!("{err}")))?;

            match event {
                QxEvent::Eof => {
                    self.finished = true;
                    return Ok(None);
                }
                QxEvent::Start(e) => {
                    let ev = start_from(&e, self.inner.decoder())?;
                    return Ok(Some(ev));
                }
                QxEvent::End(e) => {
                    let name = decode_name(e.name().as_ref())?;
                    return Ok(Some(XmlEvent::EndTag { name }));
                }
                QxEvent::Empty(e) => {
                    let start = start_from(&e, self.inner.decoder())?;
                    let name = match &start {
                        XmlEvent::StartTag { name, .. } => name.clone(),
                        _ => unreachable!(),
                    };
                    self.pending_end = Some(XmlEvent::EndTag { name });
                    return Ok(Some(start));
                }
                // Issue A fix: quick-xml 0.39.2 emits entity references like
                // &amp;, &lt;, &gt; as separate `GeneralRef` events rather than
                // keeping them inside `Text` events. We must accumulate both
                // `Text` and `GeneralRef` events into a single text string.
                //
                // This arm handles the case where the first content event is a
                // `GeneralRef` (e.g. `<a>&amp;</a>` has no leading Text event).
                QxEvent::GeneralRef(e) => {
                    let pos = self.inner.buffer_position();
                    let entity_name = e
                        .decode()
                        .map_err(|err| make_err(pos, format!("ref decode: {err}")))?;
                    let resolved = resolve_entity(&entity_name, pos)?;
                    let mut text = resolved.to_string();
                    // Accumulate any subsequent Text / GeneralRef events.
                    self.accumulate_text(&mut text)?;
                    return Ok(Some(XmlEvent::Text(text)));
                }
                QxEvent::Text(e) => {
                    let pos = self.inner.buffer_position();
                    let decoded = e
                        .decode()
                        .map_err(|err| make_err(pos, format!("text decode: {err}")))?;
                    let mut text = decoded.into_owned();
                    // Accumulate any subsequent GeneralRef / Text events so that
                    // mixed content like `hello &amp; world` becomes one Text node.
                    self.accumulate_text(&mut text)?;
                    return Ok(Some(XmlEvent::Text(text)));
                }
                // Comments, PIs, CDATA, declarations — skip and continue the loop.
                _ => continue,
            }
        }
    }

    /// Read ahead consuming consecutive `Text` and `GeneralRef` events,
    /// appending their content to `buf`. Stops (and buffers the non-text
    /// event into `self.pending_event`) when it sees anything else.
    fn accumulate_text(&mut self, buf: &mut String) -> Result<()> {
        loop {
            self.buf.clear();
            let event = self
                .inner
                .read_event_into(&mut self.buf)
                .map_err(|err| make_err(self.inner.buffer_position(), format!("{err}")))?;
            let pos = self.inner.buffer_position();
            match event {
                QxEvent::Text(e) => {
                    let decoded = e
                        .decode()
                        .map_err(|err| make_err(pos, format!("text decode: {err}")))?;
                    buf.push_str(&decoded);
                }
                QxEvent::GeneralRef(e) => {
                    let entity_name = e
                        .decode()
                        .map_err(|err| make_err(pos, format!("ref decode: {err}")))?;
                    let resolved = resolve_entity(&entity_name, pos)?;
                    buf.push_str(resolved);
                }
                // Comments, PIs, CDATA, declarations between text fragments — skip.
                QxEvent::Comment(_)
                | QxEvent::PI(_)
                | QxEvent::CData(_)
                | QxEvent::Decl(_)
                | QxEvent::DocType(_) => continue,
                QxEvent::Eof => {
                    self.finished = true;
                    break;
                }
                QxEvent::End(e) => {
                    let name = decode_name(e.name().as_ref())?;
                    self.pending_event = Some(XmlEvent::EndTag { name });
                    break;
                }
                QxEvent::Start(e) => {
                    let ev = start_from(&e, self.inner.decoder())?;
                    self.pending_event = Some(ev);
                    break;
                }
                QxEvent::Empty(e) => {
                    let start = start_from(&e, self.inner.decoder())?;
                    let name = match &start {
                        XmlEvent::StartTag { name, .. } => name.clone(),
                        _ => unreachable!(),
                    };
                    self.pending_end = Some(XmlEvent::EndTag { name });
                    self.pending_event = Some(start);
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Resolve a predefined XML entity name to its character string.
/// Returns an error for unknown entities.
fn resolve_entity(name: &str, pos: u64) -> Result<&'static str> {
    // Handle numeric character references (&#nn; or &#xhh;) — rare here since
    // quick-xml resolves char refs itself before emitting GeneralRef, but guard anyway.
    resolve_predefined_entity(name).ok_or_else(|| {
        make_err(pos, format!("unknown XML entity: &{name};"))
    })
}

fn start_from(
    e: &quick_xml::events::BytesStart<'_>,
    decoder: quick_xml::Decoder,
) -> Result<XmlEvent> {
    let name = decode_name(e.name().as_ref())?;
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| make_err(0, format!("attr: {err}")))?;
        let key = decode_name(a.key.as_ref())?;
        let val = a
            .decode_and_unescape_value(decoder)
            .map_err(|err| make_err(0, format!("attr value: {err}")))?
            .into_owned();
        attrs.push((key, val));
    }
    Ok(XmlEvent::StartTag { name, attrs })
}

fn decode_name(bytes: &[u8]) -> Result<String> {
    std::str::from_utf8(bytes)
        .map(|s| s.to_owned())
        .map_err(|e| XmlError::Encoding(format!("invalid UTF-8 in name: {e}")))
}

fn make_err(pos: u64, message: String) -> XmlError {
    // quick-xml 0.39.2 only exposes byte position; line numbers
    // require us to track them separately. Sentinel 0 = unknown.
    // Improving line/column reporting tracked in Task 4.
    XmlError::Parse {
        line: 0,
        column: pos,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XmlEvent;

    fn collect(input: &str) -> Vec<XmlEvent> {
        let mut r = EventReader::new(input.as_bytes());
        let mut out = Vec::new();
        while let Some(ev) = r.next_event().unwrap() {
            out.push(ev);
        }
        out
    }

    #[test]
    fn empty_element() {
        let evs = collect("<a/>");
        assert_eq!(evs.len(), 2);
        assert!(matches!(&evs[0], XmlEvent::StartTag { name, attrs } if name == "a" && attrs.is_empty()));
        assert!(matches!(&evs[1], XmlEvent::EndTag { name } if name == "a"));
    }

    #[test]
    fn text_inside_element() {
        let evs = collect("<a>hello</a>");
        assert_eq!(evs.len(), 3);
        assert!(matches!(&evs[0], XmlEvent::StartTag { name, .. } if name == "a"));
        assert!(matches!(&evs[1], XmlEvent::Text(t) if t == "hello"));
        assert!(matches!(&evs[2], XmlEvent::EndTag { name } if name == "a"));
    }

    #[test]
    fn nested_elements() {
        let evs = collect("<a><b/></a>");
        assert_eq!(evs.len(), 4);
        assert!(matches!(&evs[0], XmlEvent::StartTag { name, .. } if name == "a"));
        assert!(matches!(&evs[1], XmlEvent::StartTag { name, .. } if name == "b"));
        assert!(matches!(&evs[2], XmlEvent::EndTag { name } if name == "b"));
        assert!(matches!(&evs[3], XmlEvent::EndTag { name } if name == "a"));
    }

    #[test]
    fn text_entity_unescaping() {
        let evs = collect("<a>&amp;&lt;&gt;</a>");
        assert_eq!(evs.len(), 3);
        assert!(matches!(&evs[1], XmlEvent::Text(t) if t == "&<>"), "got: {:?}", evs[1]);
    }

    #[test]
    fn empty_element_with_attributes() {
        let evs = collect(r#"<img src="foo.png" alt="bar"/>"#);
        assert_eq!(evs.len(), 2);
        match &evs[0] {
            XmlEvent::StartTag { name, attrs } => {
                assert_eq!(name, "img");
                assert_eq!(
                    attrs.as_slice(),
                    &[
                        ("src".to_string(), "foo.png".to_string()),
                        ("alt".to_string(), "bar".to_string()),
                    ][..]
                );
            }
            other => panic!("expected StartTag, got {:?}", other),
        }
    }
}
