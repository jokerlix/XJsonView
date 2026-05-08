//! Parser for the --array-rule flag.
//!
//! Syntax: comma-separated dotted paths (no whitespace, no wildcards in v0.2.0).
//! Examples: "users.user", "users.user,items.item", "root.deeply.nested.elem".

use std::collections::HashSet;

#[derive(Debug, Default, Clone)]
pub struct ArrayRules {
    /// Dotted path → "this element should NOT be wrapped in an array."
    collapse: HashSet<String>,
}

impl ArrayRules {
    pub fn parse(spec: Option<&str>) -> Self {
        let mut collapse = HashSet::new();
        if let Some(spec) = spec {
            for piece in spec.split(',') {
                let p = piece.trim();
                if !p.is_empty() {
                    collapse.insert(p.to_string());
                }
            }
        }
        Self { collapse }
    }

    /// Path is a dot-joined chain of element names from the document root.
    pub fn collapse(&self, path: &str) -> bool {
        self.collapse.contains(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_list() {
        let r = ArrayRules::parse(Some("a.b,c.d.e"));
        assert!(r.collapse("a.b"));
        assert!(r.collapse("c.d.e"));
        assert!(!r.collapse("a"));
    }

    #[test]
    fn empty_input_is_empty() {
        let r = ArrayRules::parse(None);
        assert!(!r.collapse("anything"));
    }
}
