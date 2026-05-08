//! Round-trip property test: serialize-then-parse equals input event sequence
//! for a generated subset of XML events.

use jfmt_xml::{EventReader, EventWriter, XmlEvent, XmlWriter};
use proptest::prelude::*;

fn name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9]{0,8}".prop_map(String::from)
}

fn attr_strategy() -> impl Strategy<Value = (String, String)> {
    (name_strategy(), "[a-zA-Z0-9 ]{0,12}".prop_map(String::from))
}

fn element_strategy() -> impl Strategy<Value = Vec<XmlEvent>> {
    (
        name_strategy(),
        prop::collection::vec(attr_strategy(), 0..3),
        "[a-zA-Z0-9 ]{0,16}".prop_map(String::from),
    )
        .prop_map(|(name, attrs, text)| {
            // Dedupe attribute names — duplicate keys are invalid XML and
            // would be rejected by the parser.
            let mut seen = std::collections::HashSet::new();
            let attrs: Vec<_> = attrs
                .into_iter()
                .filter(|(k, _)| seen.insert(k.clone()))
                .collect();
            let mut evs = vec![XmlEvent::StartTag {
                name: name.clone(),
                attrs,
            }];
            if !text.is_empty() {
                evs.push(XmlEvent::Text(text));
            }
            evs.push(XmlEvent::EndTag { name });
            evs
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn write_then_read_preserves_events(events in element_strategy()) {
        let mut buf = Vec::new();
        let mut w = XmlWriter::new(&mut buf);
        for ev in &events {
            w.write_event(ev).unwrap();
        }
        w.finish().unwrap();

        let mut r = EventReader::new(&buf[..]);
        let mut got = Vec::new();
        while let Some(ev) = r.next_event().unwrap() {
            got.push(ev);
        }
        prop_assert_eq!(events, got);
    }
}
