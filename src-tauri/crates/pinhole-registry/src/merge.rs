//! Deep merge of YAML values (overrides over the shipped registry, child
//! family over its `inherits:` parent).

use serde_yaml::Value;

/// Merge `over` into `base`: mappings merge key by key (recursively, `over`
/// wins), everything else — scalars, sequences, `null` — replaces. Key order
/// of `base` is kept; new keys are appended in `over`'s order.
pub fn deep_merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Mapping(b), Value::Mapping(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}
