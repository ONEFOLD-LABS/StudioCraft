//! JSON mutations for the fuzz tests (see `mod.rs` beside it).

#![allow(dead_code)]

use crate::fuzzkit::Rng;

/// A replacement for `old`: mostly a value of the same kind that no writer would produce
/// (extreme numbers, odd strings), sometimes a value of another type.
pub fn odd_value(rng: &mut Rng, old: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Value, json};
    if !rng.chance(4) {
        match old {
            Value::Number(n) => {
                let x = n.as_i64().unwrap_or(0);
                let unsigned = x >= 0;
                return match rng.below(12) {
                    0 | 1 => json!(x.saturating_add(1)),
                    2 | 3 if unsigned => json!(x.saturating_sub(1).max(0)),
                    4 if unsigned => json!(x.saturating_mul(1000)),
                    5 => json!(0),
                    6 if unsigned => json!(u32::MAX),
                    7 if unsigned => json!(u64::MAX),
                    8 => json!(i64::MIN),
                    9 => json!(1e308),
                    10 => json!(-1),
                    _ => json!(x / 2),
                };
            }
            Value::String(_) => {
                return match rng.below(4) {
                    0 => json!(""),
                    1 => json!("\u{0}\u{202e}x"),
                    2 => json!("../../etc/passwd"),
                    _ => json!("x".repeat(10_000)),
                };
            }
            Value::Bool(b) => return Value::Bool(!b),
            _ => {}
        }
    }
    match rng.below(6) {
        0 => Value::Null,
        1 => Value::Bool(rng.chance(2)),
        2 => json!(u64::MAX),
        3 => json!(""),
        4 => json!([]),
        _ => json!({}),
    }
}

/// One random change of the JSON tree: replace, remove, duplicate, swap, nudge a number.
pub fn mutate_json(rng: &mut Rng, v: &mut serde_json::Value) {
    use serde_json::Value;
    // Walk down a random path; stop at a leaf or by chance.
    let mut node = v;
    loop {
        let go_on = !rng.chance(4);
        let next = match node {
            Value::Array(a) if !a.is_empty() && go_on => {
                let i = rng.below(a.len());
                Some(i)
            }
            Value::Object(o) if !o.is_empty() && go_on => Some(rng.below(o.len())),
            _ => None,
        };
        let Some(i) = next else { break };
        // Change this container sometimes instead of going deeper.
        if rng.chance(3) {
            match node {
                Value::Array(a) => match rng.below(3) {
                    0 => {
                        a.remove(i);
                    }
                    1 => {
                        let copy = a[i].clone();
                        a.insert(i, copy);
                    }
                    _ => {
                        let j = rng.below(a.len());
                        a.swap(i, j);
                    }
                },
                Value::Object(o) => {
                    if let Some(key) = o.keys().nth(i).cloned() {
                        if rng.chance(2) {
                            o.remove(&key);
                        } else {
                            let old = o.get(&key).cloned().unwrap_or_default();
                            o.insert(key, odd_value(rng, &old));
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        node = match node {
            Value::Array(a) => &mut a[i],
            Value::Object(o) => match o.values_mut().nth(i) {
                Some(x) => x,
                None => return,
            },
            _ => return,
        };
    }
    let old = node.clone();
    *node = odd_value(rng, &old);
}
