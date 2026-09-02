//! Minimal NSKeyedArchiver unwrapper.
//!
//! Apple's `NSKeyedArchiver` serializes an object graph into a plist shaped
//! like `{$version, $archiver, $top: {root: CF$UID}, $objects: [...]}`, where
//! every reference between objects is a `plist::Value::Uid` pointing into the
//! flat `$objects` array instead of being inlined. This module walks that
//! graph and returns a plain, self-contained `plist::Value` tree with every
//! `Uid` resolved to the value it points at, so callers can just do ordinary
//! dictionary/array lookups.
//!
//! We don't attempt real `NSCoding` class-aware decoding (no custom
//! `initWithCoder:` semantics) — for the flat scalar-heavy dictionaries
//! Procreate stores, resolving `Uid` references is sufficient.

use crate::error::{ConverterError, Result};
use plist::{Uid, Value};
use std::io::Cursor;

const MAX_DEPTH: usize = 64;

/// Parses `data` as a binary/XML plist and, if it's NSKeyedArchiver-shaped,
/// returns the resolved `$top.root` object. If it isn't keyed-archiver
/// shaped (no `$archiver`/`$objects`), the raw parsed value is returned
/// as-is so this also works as a plain plist decoder.
pub fn decode_root(data: &[u8]) -> Result<Value> {
    let value = Value::from_reader(Cursor::new(data))?;
    let Some(dict) = value.as_dictionary() else {
        return Ok(value);
    };
    let (Some(objects), Some(top)) = (dict.get("$objects"), dict.get("$top")) else {
        return Ok(value);
    };
    let objects = objects
        .as_array()
        .ok_or_else(|| ConverterError::Malformed("$objects is not an array".into()))?;
    let root_uid = top
        .as_dictionary()
        .and_then(|d| d.get("root"))
        .and_then(|v| v.as_uid())
        .ok_or_else(|| ConverterError::Malformed("$top.root missing".into()))?;

    resolve(&Value::Uid(*root_uid), objects, 0)
}

fn resolve(value: &Value, objects: &[Value], depth: usize) -> Result<Value> {
    if depth > MAX_DEPTH {
        return Err(ConverterError::Malformed(
            "NSKeyedArchiver object graph too deep (possible cycle)".into(),
        ));
    }
    match value {
        Value::Uid(uid) => {
            let idx = uid.get() as usize;
            let target = objects
                .get(idx)
                .ok_or_else(|| ConverterError::Malformed(format!("dangling CF$UID {idx}")))?;
            resolve(target, objects, depth + 1)
        }
        Value::Array(items) => {
            let resolved = items
                .iter()
                .map(|v| resolve(v, objects, depth + 1))
                .collect::<Result<Vec<_>>>()?;
            Ok(Value::Array(resolved))
        }
        Value::Dictionary(dict) => {
            let mut out = plist::Dictionary::new();
            for (k, v) in dict.iter() {
                out.insert(k.clone(), resolve(v, objects, depth + 1)?);
            }
            Ok(Value::Dictionary(out))
        }
        other => Ok(other.clone()),
    }
}

/// Converts a resolved `Value::Dictionary` into a `serde_json` object,
/// recursively, for stashing as free-form `extra` fidelity data. Any
/// non-dictionary root becomes an empty object.
pub fn to_json_object(value: &Value) -> serde_json::Map<String, serde_json::Value> {
    match value.as_dictionary() {
        Some(dict) => dict
            .iter()
            .map(|(k, v)| (k.clone(), to_json(v)))
            .collect(),
        None => serde_json::Map::new(),
    }
}

fn to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Real(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Integer(i) => i
            .as_signed()
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Data(_) => serde_json::Value::String("<binary data>".into()),
        Value::Date(d) => serde_json::Value::String(format!("{d:?}")),
        Value::Uid(u) => serde_json::Value::Number(u.get().into()),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(to_json).collect()),
        Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter().map(|(k, v)| (k.clone(), to_json(v))).collect(),
        ),
        _ => serde_json::Value::Null,
    }
}

/// Convenience accessors for pulling scalar fields out of a resolved
/// NSKeyedArchiver dictionary, where numbers are commonly `Real` and
/// occasionally `Integer`.
pub trait ValueExt {
    fn field_f64(&self, key: &str) -> Option<f64>;
    fn field_bool(&self, key: &str) -> Option<bool>;
    fn field_str(&self, key: &str) -> Option<&str>;
}

/// Builds a minimal NSKeyedArchiver-shaped binary plist whose `$top.root`
/// object is a dictionary with `fields` inlined directly (no per-value `Uid`
/// indirection) — the mirror image of what [`decode_root`]'s `resolve` step
/// already accepts, since it passes non-`Uid` values through unchanged. This
/// is our own self-consistent construction, not a byte-exact reproduction of
/// real `NSCoding` class-based archiving (see module docs); good enough for
/// [`decode_root`] to read back, best-effort for a real Procreate install.
pub fn encode_root(fields: &serde_json::Map<String, serde_json::Value>) -> Result<Vec<u8>> {
    let mut root_dict = plist::Dictionary::new();
    for (key, value) in fields {
        if let Some(plist_value) = json_to_plist(value) {
            root_dict.insert(key.clone(), plist_value);
        }
    }

    let mut top = plist::Dictionary::new();
    top.insert("root".to_string(), Value::Uid(Uid::new(1)));

    let mut root = plist::Dictionary::new();
    root.insert("$version".to_string(), Value::Integer(100_000i64.into()));
    root.insert(
        "$archiver".to_string(),
        Value::String("NSKeyedArchiver".to_string()),
    );
    root.insert("$top".to_string(), Value::Dictionary(top));
    root.insert(
        "$objects".to_string(),
        Value::Array(vec![
            Value::String("$null".to_string()),
            Value::Dictionary(root_dict),
        ]),
    );

    let mut buf = Vec::new();
    Value::Dictionary(root).to_writer_binary(&mut buf)?;
    Ok(buf)
}

fn json_to_plist(value: &serde_json::Value) -> Option<Value> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(Value::Boolean(*b)),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Some(Value::Integer(i.into())),
            None => n.as_f64().map(Value::Real),
        },
        serde_json::Value::String(s) => Some(Value::String(s.clone())),
        serde_json::Value::Array(items) => {
            Some(Value::Array(items.iter().filter_map(json_to_plist).collect()))
        }
        serde_json::Value::Object(map) => {
            let mut dict = plist::Dictionary::new();
            for (k, v) in map {
                if let Some(plist_value) = json_to_plist(v) {
                    dict.insert(k.clone(), plist_value);
                }
            }
            Some(Value::Dictionary(dict))
        }
    }
}

impl ValueExt for Value {
    fn field_f64(&self, key: &str) -> Option<f64> {
        let v = self.as_dictionary()?.get(key)?;
        v.as_real().or_else(|| v.as_signed_integer().map(|i| i as f64))
    }

    fn field_bool(&self, key: &str) -> Option<bool> {
        self.as_dictionary()?.get(key)?.as_boolean()
    }

    fn field_str(&self, key: &str) -> Option<&str> {
        self.as_dictionary()?.get(key)?.as_string()
    }
}
