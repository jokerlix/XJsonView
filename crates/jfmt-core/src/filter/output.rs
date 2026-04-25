//! Shape-preserving emitter. See spec §4.1.

use super::{shard::TopLevel, FilterError};
use crate::event::{Event, Scalar};
use crate::writer::EventWriter;
use serde_json::Value;

/// Streaming output for single-document mode. Caller drives by
/// calling `begin(top)` once, then `emit(...)` per shard, then
/// `finish()`.
pub struct OutputShaper<W: EventWriter> {
    writer: W,
    top: Option<TopLevel>,
}

impl<W: EventWriter> OutputShaper<W> {
    pub fn new(writer: W) -> Self {
        Self { writer, top: None }
    }

    pub fn begin(&mut self, top: TopLevel) -> Result<(), FilterError> {
        self.top = Some(top);
        match top {
            TopLevel::Array => self.writer.write_event(&Event::StartArray)?,
            TopLevel::Object => self.writer.write_event(&Event::StartObject)?,
            TopLevel::Scalar => {}
        }
        Ok(())
    }

    /// Emit zero, one, or many jaq output values for a single shard.
    /// `key` is `Some` if the input top-level was Object; `None`
    /// otherwise. `where_` is used in `OutputShape` errors.
    pub fn emit(
        &mut self,
        outputs: Vec<Value>,
        key: Option<&str>,
        where_: &str,
    ) -> Result<(), FilterError> {
        let top = self.top.expect("begin must be called");
        match top {
            TopLevel::Array => {
                for v in outputs {
                    write_value(&mut self.writer, &v)?;
                }
                Ok(())
            }
            TopLevel::Object => match outputs.len() {
                0 => Ok(()),
                1 => {
                    let k = key.expect("Object top-level requires key");
                    self.writer.write_event(&Event::Name(k.to_string()))?;
                    write_value(&mut self.writer, &outputs[0])
                }
                _ => Err(FilterError::OutputShape {
                    where_: where_.to_string(),
                    kind: "object",
                }),
            },
            TopLevel::Scalar => match outputs.len() {
                0 => Ok(()),
                1 => write_value(&mut self.writer, &outputs[0]),
                _ => Err(FilterError::OutputShape {
                    where_: where_.to_string(),
                    kind: "scalar",
                }),
            },
        }
    }

    pub fn finish(mut self) -> Result<(), FilterError> {
        match self.top {
            Some(TopLevel::Array) => self.writer.write_event(&Event::EndArray)?,
            Some(TopLevel::Object) => self.writer.write_event(&Event::EndObject)?,
            Some(TopLevel::Scalar) | None => {}
        }
        self.writer.finish()?;
        Ok(())
    }

    /// Test-only: finish and return the underlying bytes. Requires
    /// the writer to support `into_inner()`.
    #[cfg(test)]
    pub fn finish_into_bytes(mut self) -> Result<Vec<u8>, FilterError>
    where
        W: crate::writer::IntoInner<Vec<u8>>,
    {
        match self.top {
            Some(TopLevel::Array) => self.writer.write_event(&Event::EndArray)?,
            Some(TopLevel::Object) => self.writer.write_event(&Event::EndObject)?,
            Some(TopLevel::Scalar) | None => {}
        }
        self.writer.finish()?;
        Ok(self.writer.into_inner())
    }
}

/// Emit a `serde_json::Value` as a sequence of `Event`s into `writer`.
fn write_value<W: EventWriter>(writer: &mut W, v: &Value) -> Result<(), FilterError> {
    match v {
        Value::Null => writer.write_event(&Event::Value(Scalar::Null))?,
        Value::Bool(b) => writer.write_event(&Event::Value(Scalar::Bool(*b)))?,
        Value::Number(n) => writer.write_event(&Event::Value(Scalar::Number(n.to_string())))?,
        Value::String(s) => writer.write_event(&Event::Value(Scalar::String(s.clone())))?,
        Value::Array(items) => {
            writer.write_event(&Event::StartArray)?;
            for it in items {
                write_value(writer, it)?;
            }
            writer.write_event(&Event::EndArray)?;
        }
        Value::Object(map) => {
            writer.write_event(&Event::StartObject)?;
            for (k, v) in map {
                writer.write_event(&Event::Name(k.clone()))?;
                write_value(writer, v)?;
            }
            writer.write_event(&Event::EndObject)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::MinifyWriter;
    use serde_json::json;

    /// Helper: shape outputs and return the resulting bytes as UTF-8.
    fn shape(top: TopLevel, calls: &[(&[Value], Option<&str>)]) -> Result<String, FilterError> {
        let buf = Vec::<u8>::new();
        let writer = MinifyWriter::new(buf);
        let mut shaper = OutputShaper::new(writer);
        shaper.begin(top)?;
        for (i, (vals, key)) in calls.iter().enumerate() {
            let where_ = format!("idx={i}");
            shaper.emit(vals.to_vec(), *key, &where_)?;
        }
        let bytes = shaper.finish_into_bytes()?;
        Ok(String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn array_zero_outputs_drops_element() {
        let s = shape(TopLevel::Array, &[(&[], None), (&[json!(1)], None)]).unwrap();
        assert_eq!(s, "[1]");
    }

    #[test]
    fn array_n_outputs_expand() {
        let s = shape(TopLevel::Array, &[(&[json!(1), json!(2)], None)]).unwrap();
        assert_eq!(s, "[1,2]");
    }

    #[test]
    fn object_one_output_writes_pair() {
        let s = shape(TopLevel::Object, &[(&[json!(1)], Some("a"))]).unwrap();
        assert_eq!(s, "{\"a\":1}");
    }

    #[test]
    fn object_zero_outputs_drops_key() {
        let s = shape(
            TopLevel::Object,
            &[(&[], Some("a")), (&[json!(2)], Some("b"))],
        )
        .unwrap();
        assert_eq!(s, "{\"b\":2}");
    }

    #[test]
    fn object_n_outputs_errors() {
        let err = shape(TopLevel::Object, &[(&[json!(1), json!(2)], Some("a"))]).unwrap_err();
        assert!(matches!(
            err,
            FilterError::OutputShape { kind: "object", .. }
        ));
    }

    #[test]
    fn scalar_one_output_writes_value() {
        let s = shape(TopLevel::Scalar, &[(&[json!(true)], None)]).unwrap();
        assert_eq!(s, "true");
    }

    #[test]
    fn scalar_n_outputs_errors() {
        let err = shape(TopLevel::Scalar, &[(&[json!(1), json!(2)], None)]).unwrap_err();
        assert!(matches!(
            err,
            FilterError::OutputShape { kind: "scalar", .. }
        ));
    }
}
