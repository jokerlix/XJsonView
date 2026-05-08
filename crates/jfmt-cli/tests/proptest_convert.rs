//! Round-trip property tests for jfmt convert.
//!
//! XML → JSON → XML: structurally equivalent modulo documented losses
//! (comments / PI / decl dropped; mixed-content order; non-contiguous
//! siblings excluded from the generator).
//!
//! JSON → XML → JSON: structurally equivalent in default array mode for
//! the generated subset of JSON shapes (objects with @attrs / #text /
//! arrays of scalars-only).

use jfmt_xml::{EventReader as XmlReader, XmlEvent};
use proptest::prelude::*;

// --- Generators ---

fn name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_]{0,8}".prop_map(String::from)
}

fn attr_pair() -> impl Strategy<Value = (String, String)> {
    (name(), "[a-zA-Z0-9 ]{0,12}".prop_map(String::from))
}

/// Generates well-formed XML where same-name siblings are always
/// contiguous (the streaming-friendly subset).
fn xml_doc() -> impl Strategy<Value = String> {
    let leaf = (
        name(),
        prop::collection::vec(attr_pair(), 0..3),
        prop::option::of("[a-zA-Z0-9 ]{1,16}".prop_map(String::from)),
    )
        .prop_map(|(n, attrs, text)| {
            let mut s = format!("<{n}");
            for (k, v) in &attrs {
                s.push_str(&format!(r#" {k}="{}""#, escape_attr(v)));
            }
            if let Some(t) = text {
                s.push('>');
                s.push_str(&escape_text(&t));
                s.push_str(&format!("</{n}>"));
            } else {
                s.push_str("/>");
            }
            s
        });

    // Wrap a single leaf element as root → guaranteed valid single-root XML.
    (name(), prop::collection::vec(leaf, 1..6)).prop_map(|(root_name, children)| {
        // Group consecutive siblings so same-name elements stay contiguous.
        let mut grouped = children.clone();
        grouped.sort_by_key(|c| extract_first_name(c));
        format!("<{root_name}>{}</{root_name}>", grouped.join(""))
    })
}

fn extract_first_name(s: &str) -> String {
    s.trim_start_matches('<')
        .split(|c: char| c == ' ' || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_string()
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

// --- Round-trip helpers ---

fn xml_to_json(xml: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    let args = jfmt_cli_test_args();
    jfmt_cli::commands::convert::xml_to_json::translate(xml.as_bytes(), &mut buf, &args).unwrap();
    buf
}

fn json_to_xml(json: &[u8]) -> String {
    let mut buf = Vec::new();
    let args = jfmt_cli_test_args();
    jfmt_cli::commands::convert::json_to_xml::translate(json, &mut buf, &args).unwrap();
    String::from_utf8(buf).unwrap()
}

fn jfmt_cli_test_args() -> jfmt_cli::cli::ConvertArgs {
    jfmt_cli::cli::ConvertArgs {
        input: None,
        output: None,
        from: None,
        to: None,
        array_rule: None,
        root: None,
        pretty: false,
        indent: None,
        tabs: false,
        xml_decl: false,
        strict: false,
    }
}

/// Parse XML to a Vec<XmlEvent> for structural comparison (filters out
/// Decl / Comment / PI per documented losses; sorts attrs since JSON
/// Map ordering is alphabetical, not insertion-ordered; drops empty
/// Text events since XML→JSON cannot distinguish "no text" from "empty
/// text").
fn parse_events(xml: &str) -> Vec<XmlEvent> {
    let mut r = XmlReader::new(xml.as_bytes());
    let mut out = Vec::new();
    while let Some(ev) = r.next_event().unwrap() {
        match ev {
            XmlEvent::Decl { .. } | XmlEvent::Comment(_) | XmlEvent::Pi { .. } => {}
            XmlEvent::Text(t) if t.is_empty() => {}
            XmlEvent::StartTag { name, mut attrs } => {
                attrs.sort();
                out.push(XmlEvent::StartTag { name, attrs });
            }
            other => out.push(other),
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn xml_to_json_to_xml_structural(xml in xml_doc()) {
        let json = xml_to_json(&xml);
        let xml2 = json_to_xml(&json);
        let evs1 = parse_events(&xml);
        let evs2 = parse_events(&xml2);
        prop_assert_eq!(evs1, evs2);
    }
}

fn json_doc() -> impl Strategy<Value = serde_json::Value> {
    (
        name(),
        prop::collection::vec(attr_pair(), 0..2),
        prop::option::of("[a-zA-Z0-9 ]{0,12}".prop_map(String::from)),
    )
        .prop_map(|(root, attrs, text)| {
            let mut obj = serde_json::Map::new();
            for (k, v) in attrs {
                obj.insert(format!("@{k}"), serde_json::Value::String(v));
            }
            if let Some(t) = text {
                obj.insert("#text".to_string(), serde_json::Value::String(t));
            }
            let mut top = serde_json::Map::new();
            top.insert(root, serde_json::Value::Object(obj));
            serde_json::Value::Object(top)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn json_to_xml_to_json_structural(value in json_doc()) {
        let json_in = serde_json::to_vec(&value).unwrap();
        let xml = json_to_xml(&json_in);
        let json_out = xml_to_json(&xml);
        let v_out: serde_json::Value = serde_json::from_slice(&json_out).unwrap();
        let v_in_normalized = normalize_for_compare(&value);
        prop_assert_eq!(v_in_normalized, v_out);
    }
}

fn normalize_for_compare(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) if m.len() == 1 => {
            let (k, child) = m.iter().next().unwrap();
            let mut wrapped = serde_json::Map::new();
            wrapped.insert(
                k.clone(),
                serde_json::Value::Array(vec![normalize_inner(child)]),
            );
            serde_json::Value::Object(wrapped)
        }
        _ => v.clone(),
    }
}

fn normalize_inner(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => {
            let mut out = serde_json::Map::new();
            for (k, val) in m {
                if k.starts_with('@') {
                    out.insert(k.clone(), val.clone());
                } else if k == "#text" {
                    // XML round-trip drops empty text — match that here.
                    if let serde_json::Value::String(s) = val {
                        if s.is_empty() {
                            continue;
                        }
                    }
                    out.insert(k.clone(), val.clone());
                } else {
                    out.insert(
                        k.clone(),
                        serde_json::Value::Array(vec![normalize_inner(val)]),
                    );
                }
            }
            serde_json::Value::Object(out)
        }
        _ => v.clone(),
    }
}
