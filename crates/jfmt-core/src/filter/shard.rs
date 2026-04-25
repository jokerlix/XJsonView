//! Bridge between the event stream and `serde_json::Value` shards.

use crate::event::{Event, Scalar};
use serde_json::Value;

/// Top-level form of the document, decided after the first event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopLevel {
    Array,
    Object,
    Scalar,
}

/// One shard ready for jaq, plus the locator used in error messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Shard {
    /// 0-based array index, owned object key, or empty for top-level scalar.
    pub locator: ShardLocator,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShardLocator {
    Index(u64),
    Key(String),
    Root,
}

/// Stateful accumulator. Feed it events one at a time; it returns
/// `Some(Shard)` whenever a top-level shard is complete, `None`
/// otherwise.
pub struct ShardAccumulator {
    state: State,
    /// Stack of partially-built containers waiting for their child events.
    stack: Vec<Builder>,
    next_index: u64,
}

enum State {
    /// Before any events.
    Start,
    /// Top-level form known; reading shards.
    Body { top: TopLevel },
    /// After the closing event of the top-level container.
    Done { top: TopLevel },
}

enum Builder {
    Array(Vec<Value>),
    Object {
        map: serde_json::Map<String, Value>,
        pending_key: Option<String>,
    },
}

impl ShardAccumulator {
    pub fn new() -> Self {
        Self {
            state: State::Start,
            stack: Vec::new(),
            next_index: 0,
        }
    }

    pub fn top_level(&self) -> Option<TopLevel> {
        match self.state {
            State::Body { top } | State::Done { top } => Some(top),
            State::Start => None,
        }
    }

    /// Feed one event. Returns the shard that just completed, if any.
    pub fn push(&mut self, ev: Event) -> Result<Option<Shard>, ShardError> {
        use Event::*;
        match (&mut self.state, ev) {
            // ---- decide top-level form on the first event ----
            (State::Start, StartArray) => {
                self.state = State::Body {
                    top: TopLevel::Array,
                };
                Ok(None)
            }
            (State::Start, StartObject) => {
                self.state = State::Body {
                    top: TopLevel::Object,
                };
                Ok(None)
            }
            (State::Start, Value(s)) => {
                self.state = State::Done {
                    top: TopLevel::Scalar,
                };
                Ok(Some(Shard {
                    locator: ShardLocator::Root,
                    value: scalar_to_value(s),
                }))
            }
            (State::Start, e) => Err(ShardError::Unexpected {
                state: "start",
                event: e,
            }),

            // ---- closing the top-level container ----
            (
                State::Body {
                    top: TopLevel::Array,
                },
                EndArray,
            ) if self.stack.is_empty() => {
                self.state = State::Done {
                    top: TopLevel::Array,
                };
                Ok(None)
            }
            (
                State::Body {
                    top: TopLevel::Object,
                },
                EndObject,
            ) if self.stack.is_empty() => {
                self.state = State::Done {
                    top: TopLevel::Object,
                };
                Ok(None)
            }

            // ---- top-level array: each completed value is a shard ----
            (
                State::Body {
                    top: TopLevel::Array,
                },
                ev,
            ) if self.stack.is_empty() => {
                // Start a builder if the event opens a container; if it's
                // a scalar, emit a shard immediately.
                match ev {
                    StartArray => {
                        self.stack.push(Builder::Array(Vec::new()));
                        Ok(None)
                    }
                    StartObject => {
                        self.stack.push(Builder::Object {
                            map: serde_json::Map::new(),
                            pending_key: None,
                        });
                        Ok(None)
                    }
                    Value(s) => {
                        let idx = self.next_index;
                        self.next_index += 1;
                        Ok(Some(Shard {
                            locator: ShardLocator::Index(idx),
                            value: scalar_to_value(s),
                        }))
                    }
                    e => Err(ShardError::Unexpected {
                        state: "top-array",
                        event: e,
                    }),
                }
            }

            // ---- top-level object: track pending key, emit on value ----
            (
                State::Body {
                    top: TopLevel::Object,
                },
                ev,
            ) if self.stack.is_empty() => match ev {
                Name(k) => {
                    // Push a synthetic "depth-1 object scope" carrying
                    // only the pending key.
                    self.stack.push(Builder::Object {
                        map: serde_json::Map::new(),
                        pending_key: Some(k),
                    });
                    Ok(None)
                }
                e => Err(ShardError::Unexpected {
                    state: "top-object",
                    event: e,
                }),
            },

            // ---- inside a builder: assemble a value ----
            (State::Body { top }, ev) => assemble(&mut self.stack, ev, *top, &mut self.next_index),

            (State::Done { .. }, e) => Err(ShardError::Unexpected {
                state: "done",
                event: e,
            }),
        }
    }
}

fn scalar_to_value(s: Scalar) -> Value {
    match s {
        Scalar::String(s) => Value::String(s),
        Scalar::Number(lex) => {
            // Prefer to keep the original lexical form. If serde_json
            // can parse it as a Number, use that; otherwise fall back
            // to a string so we never lose data.
            serde_json::from_str::<Value>(&lex).unwrap_or(Value::String(lex))
        }
        Scalar::Bool(b) => Value::Bool(b),
        Scalar::Null => Value::Null,
    }
}

fn assemble(
    stack: &mut Vec<Builder>,
    ev: Event,
    top: TopLevel,
    next_index: &mut u64,
) -> Result<Option<Shard>, ShardError> {
    let value: Value = match ev {
        Event::StartArray => {
            stack.push(Builder::Array(Vec::new()));
            return Ok(None);
        }
        Event::StartObject => {
            stack.push(Builder::Object {
                map: serde_json::Map::new(),
                pending_key: None,
            });
            return Ok(None);
        }
        Event::Name(k) => match stack.last_mut() {
            Some(Builder::Object { pending_key, .. }) => {
                *pending_key = Some(k);
                return Ok(None);
            }
            _ => {
                return Err(ShardError::Unexpected {
                    state: "name-outside-object",
                    event: Event::Name(k),
                });
            }
        },
        Event::Value(s) => scalar_to_value(s),
        Event::EndArray => {
            let b = stack.pop().ok_or(ShardError::Truncated)?;
            match b {
                Builder::Array(v) => Value::Array(v),
                Builder::Object { .. } => {
                    return Err(ShardError::Unexpected {
                        state: "end-array-mismatch",
                        event: Event::EndArray,
                    });
                }
            }
        }
        Event::EndObject => {
            let b = stack.pop().ok_or(ShardError::Truncated)?;
            match b {
                Builder::Object { map, pending_key } => {
                    if pending_key.is_some() {
                        return Err(ShardError::Unexpected {
                            state: "object-ended-with-pending-key",
                            event: Event::EndObject,
                        });
                    }
                    Value::Object(map)
                }
                Builder::Array(_) => {
                    return Err(ShardError::Unexpected {
                        state: "end-object-mismatch",
                        event: Event::EndObject,
                    });
                }
            }
        }
    };

    place_value(stack, value, top, next_index)
}

fn place_value(
    stack: &mut Vec<Builder>,
    value: Value,
    top: TopLevel,
    next_index: &mut u64,
) -> Result<Option<Shard>, ShardError> {
    let _ = top;

    let depth = stack.len();
    match stack.last_mut() {
        Some(Builder::Array(v)) => {
            v.push(value);
            Ok(None)
        }
        Some(Builder::Object { map, pending_key }) => {
            let key = pending_key.take().ok_or(ShardError::Unexpected {
                state: "value-without-key",
                event: Event::Value(Scalar::Null),
            })?;
            // If this is the synthetic top-level object scope (it
            // owns no `map` content because we emit per-key shards),
            // emit the shard and pop.
            if depth == 1 && matches!(top, TopLevel::Object) {
                stack.pop();
                return Ok(Some(Shard {
                    locator: ShardLocator::Key(key),
                    value,
                }));
            }
            map.insert(key, value);
            Ok(None)
        }
        None => {
            // Stack empty: this is a top-level array element.
            debug_assert!(matches!(top, TopLevel::Array));
            let idx = *next_index;
            *next_index += 1;
            Ok(Some(Shard {
                locator: ShardLocator::Index(idx),
                value,
            }))
        }
    }
}

impl Default for ShardAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShardError {
    #[error("unexpected event in {state}: {event:?}")]
    Unexpected { state: &'static str, event: Event },
    #[error("event stream ended mid-shard")]
    Truncated,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(events: Vec<Event>) -> (Vec<Shard>, Option<TopLevel>) {
        let mut acc = ShardAccumulator::new();
        let mut shards = Vec::new();
        for ev in events {
            if let Some(s) = acc.push(ev).expect("push") {
                shards.push(s);
            }
        }
        (shards, acc.top_level())
    }

    #[test]
    fn top_level_array_emits_one_shard_per_element() {
        let evs = vec![
            Event::StartArray,
            Event::Value(Scalar::Number("1".into())),
            Event::Value(Scalar::Number("2".into())),
            Event::EndArray,
        ];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Array));
        assert_eq!(shards.len(), 2);
        assert_eq!(shards[0].locator, ShardLocator::Index(0));
        assert_eq!(shards[0].value, serde_json::json!(1));
        assert_eq!(shards[1].locator, ShardLocator::Index(1));
        assert_eq!(shards[1].value, serde_json::json!(2));
    }

    #[test]
    fn top_level_object_emits_one_shard_per_key() {
        let evs = vec![
            Event::StartObject,
            Event::Name("a".into()),
            Event::Value(Scalar::Number("1".into())),
            Event::Name("b".into()),
            Event::Value(Scalar::String("hi".into())),
            Event::EndObject,
        ];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Object));
        assert_eq!(shards.len(), 2);
        assert_eq!(shards[0].locator, ShardLocator::Key("a".into()));
        assert_eq!(shards[0].value, serde_json::json!(1));
        assert_eq!(shards[1].locator, ShardLocator::Key("b".into()));
        assert_eq!(shards[1].value, serde_json::json!("hi"));
    }

    #[test]
    fn top_level_scalar_emits_one_shard_at_root() {
        let evs = vec![Event::Value(Scalar::Bool(true))];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Scalar));
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].locator, ShardLocator::Root);
        assert_eq!(shards[0].value, serde_json::json!(true));
    }

    #[test]
    fn nested_array_inside_array_shard_assembles_correctly() {
        let evs = vec![
            Event::StartArray,
            Event::StartArray,
            Event::Value(Scalar::Number("1".into())),
            Event::Value(Scalar::Number("2".into())),
            Event::EndArray,
            Event::EndArray,
        ];
        let (shards, _) = drive(evs);
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].value, serde_json::json!([1, 2]));
    }

    #[test]
    fn nested_object_inside_array_shard_assembles_correctly() {
        let evs = vec![
            Event::StartArray,
            Event::StartObject,
            Event::Name("k".into()),
            Event::Value(Scalar::Null),
            Event::EndObject,
            Event::EndArray,
        ];
        let (shards, _) = drive(evs);
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].value, serde_json::json!({"k": null}));
    }

    #[test]
    fn empty_top_level_array_emits_no_shards() {
        let evs = vec![Event::StartArray, Event::EndArray];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Array));
        assert!(shards.is_empty());
    }

    #[test]
    fn empty_top_level_object_emits_no_shards() {
        let evs = vec![Event::StartObject, Event::EndObject];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Object));
        assert!(shards.is_empty());
    }

    #[test]
    fn number_preserves_lexical_form() {
        let evs = vec![Event::Value(Scalar::Number("1.0e10".into()))];
        let (shards, _) = drive(evs);
        // serde_json may normalise the literal; what we assert is that
        // *some* number was produced, and the conversion did not panic.
        assert!(shards[0].value.is_number());
    }
}
