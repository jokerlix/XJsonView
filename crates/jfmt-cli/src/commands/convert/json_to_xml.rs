//! Streaming JSON → XML translator (spec §4.2).

use crate::cli::ConvertArgs;
use anyhow::{anyhow, bail, Context, Result};
use jfmt_xml::{EventWriter, XmlEvent, XmlPrettyConfig, XmlWriter};
use std::io::{Read, Write};

pub fn translate<R: Read, W: Write>(input: R, output: W, args: &ConvertArgs) -> Result<()> {
    // For Task 10 we materialize JSON to a serde_json::Value first.
    // The spec promises constant memory only for XML→JSON; JSON→XML can
    // use serde_json since the input shape (must be top-level object for
    // the convert use case) is bounded by the user's input. This is a
    // pragmatic v0.2.0 simplification; constant-memory streaming JSON→XML
    // is a follow-up.
    let mut buf = Vec::new();
    let mut reader = input;
    std::io::Read::read_to_end(&mut reader, &mut buf).context("reading JSON")?;
    let value: serde_json::Value = serde_json::from_slice(&buf).context("parsing JSON input")?;

    let indent_u8 = args
        .indent
        .map(|n| n.min(u8::MAX as usize) as u8)
        .unwrap_or(if args.pretty { 2 } else { 0 });
    let cfg = XmlPrettyConfig {
        indent: indent_u8,
        tabs: args.tabs,
        xml_decl: args.xml_decl,
    };
    let mut w = XmlWriter::with_config(output, cfg);

    // Resolve the root element name.
    let root_name = match &value {
        serde_json::Value::Object(map) if map.len() == 1 && args.root.is_none() => {
            map.keys().next().unwrap().clone()
        }
        _ => args.root.clone().ok_or_else(|| {
            anyhow!("JSON top level is not a single-key object; pass --root NAME to wrap it")
        })?,
    };

    // If the value is the single-key object case, unwrap; otherwise the
    // entire value becomes the root element's content.
    let root_value = match &value {
        serde_json::Value::Object(map) if map.len() == 1 && args.root.is_none() => {
            map.values().next().unwrap().clone()
        }
        _ => value,
    };

    write_element(&mut w, &root_name, &root_value)?;
    w.finish()?;
    Ok(())
}

fn write_element<W: Write>(
    w: &mut XmlWriter<W>,
    name: &str,
    value: &serde_json::Value,
) -> Result<()> {
    use serde_json::Value;
    match value {
        Value::Null => {
            w.write_event(&XmlEvent::StartTag {
                name: name.into(),
                attrs: vec![],
            })?;
            w.write_event(&XmlEvent::EndTag { name: name.into() })?;
        }
        Value::Bool(b) => {
            w.write_event(&XmlEvent::StartTag {
                name: name.into(),
                attrs: vec![],
            })?;
            w.write_event(&XmlEvent::Text(b.to_string()))?;
            w.write_event(&XmlEvent::EndTag { name: name.into() })?;
        }
        Value::Number(n) => {
            w.write_event(&XmlEvent::StartTag {
                name: name.into(),
                attrs: vec![],
            })?;
            w.write_event(&XmlEvent::Text(n.to_string()))?;
            w.write_event(&XmlEvent::EndTag { name: name.into() })?;
        }
        Value::String(s) => {
            w.write_event(&XmlEvent::StartTag {
                name: name.into(),
                attrs: vec![],
            })?;
            w.write_event(&XmlEvent::Text(s.clone()))?;
            w.write_event(&XmlEvent::EndTag { name: name.into() })?;
        }
        Value::Array(items) => {
            for v in items {
                write_element(w, name, v)?;
            }
        }
        Value::Object(map) => {
            // Partition into attrs (keys starting with @), text (#text),
            // and children (everything else).
            let mut attrs: Vec<(String, String)> = Vec::new();
            let mut text: Option<String> = None;
            let mut children: Vec<(&String, &Value)> = Vec::new();
            for (k, v) in map {
                if let Some(attr_key) = k.strip_prefix('@') {
                    let s = match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => String::new(),
                        _ => bail!("attribute @{attr_key} must be scalar, got {}", describe(v)),
                    };
                    attrs.push((attr_key.to_string(), s));
                } else if k == "#text" {
                    let s = match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => String::new(),
                        _ => bail!("#text must be scalar, got {}", describe(v)),
                    };
                    text = Some(s);
                } else if k.starts_with('#') {
                    bail!("unrecognized special key '{k}' (only #text supported)");
                } else {
                    children.push((k, v));
                }
            }

            w.write_event(&XmlEvent::StartTag {
                name: name.into(),
                attrs,
            })?;
            if let Some(t) = text {
                w.write_event(&XmlEvent::Text(t))?;
            }
            for (child_name, child_val) in children {
                write_element(w, child_name, child_val)?;
            }
            w.write_event(&XmlEvent::EndTag { name: name.into() })?;
        }
    }
    Ok(())
}

fn describe(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(json: &str) -> String {
        let args = ConvertArgs {
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
        };
        let mut out = Vec::new();
        translate(json.as_bytes(), &mut out, &args).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn single_key_object_becomes_root() {
        assert_eq!(run(r#"{"a": "v"}"#), "<a>v</a>");
    }

    #[test]
    fn attributes_then_text() {
        assert_eq!(
            run(r##"{"a": {"@x": "1", "#text": "v"}}"##),
            r#"<a x="1">v</a>"#
        );
    }

    #[test]
    fn array_emits_siblings() {
        assert_eq!(
            run(r#"{"a": {"b": ["v1", "v2"]}}"#),
            "<a><b>v1</b><b>v2</b></a>"
        );
    }

    #[test]
    fn null_emits_empty_element() {
        assert_eq!(run(r#"{"a": null}"#), "<a></a>");
    }

    #[test]
    fn number_and_bool_emit_as_text() {
        // serde_json's default Map orders keys alphabetically.
        assert_eq!(
            run(r#"{"a": {"n": 42, "b": true}}"#),
            "<a><b>true</b><n>42</n></a>"
        );
    }

    #[test]
    fn multi_key_top_level_errors() {
        let args = ConvertArgs {
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
        };
        let mut out = Vec::new();
        let err = translate(r#"{"a":1,"b":2}"#.as_bytes(), &mut out, &args).unwrap_err();
        assert!(format!("{err:#}").contains("--root"));
    }
}
