//! Photoshop "Descriptor" binary format — the key/type/value structure used
//! for the `8BIM desc` section of `.abr` (and elsewhere: PSD layer effects,
//! `.asl` styles, actions...). Grammar below was derived and verified
//! byte-exact (zero leftover bytes) against the `desc` section of a real
//! CS/CC-era `.abr` file:
//!
//! ```text
//! Descriptor  := version:u32 Object
//! Object      := name:UnicodeString classID:OSType count:u32 (key:OSType type:OSTypeTag Value)*
//! UnicodeString := len:u32 (len UTF-16BE code units, NUL-terminated & the NUL counted in len)
//! OSType      := len:u32 (len==0 => next 4 bytes are a literal FourCC; else that many raw bytes)
//! Value (by OSTypeTag):
//!   "TEXT" => UnicodeString
//!   "long" => i32
//!   "doub" => f64
//!   "bool" => u8 (0/1)
//!   "enum" => type:OSType value:OSType
//!   "UntF" => unit:FourCC(4 raw bytes) value:f64
//!   "Objc" => Object
//!   "VlLs" => count:u32 (type:FourCC(4 raw bytes) Value)*
//! ```
//!
//! Only the tags actually needed for sampled-brush presets are implemented.

use crate::error::{ConverterError, Result};

#[derive(Debug, Clone)]
pub enum DescValue {
    Text(String),
    Long(i32),
    Double(f64),
    Bool(bool),
    Enum { type_id: String, value: String },
    UnitFloat { unit: String, value: f64 },
    Object(DescObject),
    List(Vec<DescValue>),
}

impl DescValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            DescValue::Text(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_double(&self) -> Option<f64> {
        match self {
            DescValue::Double(d) => Some(*d),
            DescValue::Long(l) => Some(*l as f64),
            _ => None,
        }
    }
    pub fn as_unit_float(&self) -> Option<(&str, f64)> {
        match self {
            DescValue::UnitFloat { unit, value } => Some((unit, *value)),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            DescValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&DescObject> {
        match self {
            DescValue::Object(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[DescValue]> {
        match self {
            DescValue::List(items) => Some(items),
            _ => None,
        }
    }
}

/// An ordered key/value bag (Photoshop descriptors are order-sensitive in
/// practice, so this is a `Vec`, not a map).
#[derive(Debug, Clone, Default)]
pub struct DescObject {
    pub name: String,
    pub class_id: String,
    pub items: Vec<(String, DescValue)>,
}

impl DescObject {
    pub fn new(class_id: impl Into<String>) -> Self {
        DescObject {
            name: String::new(),
            class_id: class_id.into(),
            items: Vec::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&DescValue> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn push(&mut self, key: impl Into<String>, value: DescValue) -> &mut Self {
        self.items.push((key.into(), value));
        self
    }
}

pub fn read_descriptor(data: &[u8]) -> Result<DescObject> {
    let mut r = Reader { data, pos: 0 };
    let _version = r.u32()?;
    r.object()
}

pub fn write_descriptor(obj: &DescObject) -> Vec<u8> {
    let mut w = Writer { out: Vec::new() };
    w.u32(16); // descriptor version, matches every observed real file
    w.object(obj);
    w.out
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| ConverterError::Malformed("descriptor: unexpected end of data".into()))?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn fourcc(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.take(4)?).into_owned())
    }

    fn unicode_string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len * 2)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        let mut s = String::from_utf16_lossy(&units);
        if s.ends_with('\0') {
            s.pop();
        }
        Ok(s)
    }

    fn ostype_id(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes = if len == 0 { self.take(4)? } else { self.take(len)? };
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn object(&mut self) -> Result<DescObject> {
        let name = self.unicode_string()?;
        let class_id = self.ostype_id()?;
        let count = self.u32()?;
        let mut items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let key = self.ostype_id()?;
            let type_tag = self.fourcc()?;
            let value = self.value(&type_tag)?;
            items.push((key, value));
        }
        Ok(DescObject { name, class_id, items })
    }

    fn value(&mut self, type_tag: &str) -> Result<DescValue> {
        match type_tag {
            "TEXT" => Ok(DescValue::Text(self.unicode_string()?)),
            "long" => Ok(DescValue::Long(self.i32()?)),
            "doub" => Ok(DescValue::Double(self.f64()?)),
            "bool" => Ok(DescValue::Bool(self.u8()? != 0)),
            "enum" => Ok(DescValue::Enum {
                type_id: self.ostype_id()?,
                value: self.ostype_id()?,
            }),
            "UntF" => Ok(DescValue::UnitFloat {
                unit: self.fourcc()?,
                value: self.f64()?,
            }),
            "Objc" => Ok(DescValue::Object(self.object()?)),
            "VlLs" => {
                let count = self.u32()?;
                let mut items = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let item_type = self.fourcc()?;
                    items.push(self.value(&item_type)?);
                }
                Ok(DescValue::List(items))
            }
            other => Err(ConverterError::Malformed(format!(
                "descriptor: unsupported value type {other:?}"
            ))),
        }
    }
}

struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn raw_fourcc(&mut self, s: &str) {
        let mut bytes = [b' '; 4];
        for (i, b) in s.as_bytes().iter().take(4).enumerate() {
            bytes[i] = *b;
        }
        self.out.extend_from_slice(&bytes);
    }

    fn unicode_string(&mut self, s: &str) {
        let mut units: Vec<u16> = s.encode_utf16().collect();
        units.push(0); // Photoshop NUL-terminates and counts it in the length
        self.u32(units.len() as u32);
        for u in units {
            self.out.extend_from_slice(&u.to_be_bytes());
        }
    }

    /// Matches what real files do: exactly-4-byte keys/classIDs (`Brsh`,
    /// `Nm  `, `Dmtr`, `null`, ...) use the `len==0` literal-FourCC branch;
    /// anything else (`sampledData`, `flipX`, `brushPreset`, ...) is written
    /// length-prefixed.
    fn ostype_id(&mut self, s: &str) {
        let bytes = s.as_bytes();
        if bytes.len() == 4 {
            self.u32(0);
            self.out.extend_from_slice(bytes);
        } else {
            self.u32(bytes.len() as u32);
            self.out.extend_from_slice(bytes);
        }
    }

    fn object(&mut self, obj: &DescObject) {
        self.unicode_string(&obj.name);
        self.ostype_id(&obj.class_id);
        self.u32(obj.items.len() as u32);
        for (key, value) in &obj.items {
            self.ostype_id(key);
            self.raw_fourcc(type_tag_of(value));
            self.value(value);
        }
    }

    fn value(&mut self, value: &DescValue) {
        match value {
            DescValue::Text(s) => self.unicode_string(s),
            DescValue::Long(v) => self.i32(*v),
            DescValue::Double(v) => self.f64(*v),
            DescValue::Bool(v) => self.u8(*v as u8),
            DescValue::Enum { type_id, value } => {
                self.ostype_id(type_id);
                self.ostype_id(value);
            }
            DescValue::UnitFloat { unit, value } => {
                self.raw_fourcc(unit);
                self.f64(*value);
            }
            DescValue::Object(o) => self.object(o),
            DescValue::List(items) => {
                self.u32(items.len() as u32);
                for item in items {
                    self.raw_fourcc(type_tag_of(item));
                    self.value(item);
                }
            }
        }
    }
}

fn type_tag_of(value: &DescValue) -> &'static str {
    match value {
        DescValue::Text(_) => "TEXT",
        DescValue::Long(_) => "long",
        DescValue::Double(_) => "doub",
        DescValue::Bool(_) => "bool",
        DescValue::Enum { .. } => "enum",
        DescValue::UnitFloat { .. } => "UntF",
        DescValue::Object(_) => "Objc",
        DescValue::List(_) => "VlLs",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_sampled_brush_preset_list() {
        let mut tip = DescObject::new("sampledBrush");
        tip.push("Dmtr", DescValue::UnitFloat { unit: "#Pxl".into(), value: 512.0 })
            .push("Angl", DescValue::UnitFloat { unit: "#Ang".into(), value: 0.0 })
            .push("Rndn", DescValue::UnitFloat { unit: "#Prc".into(), value: 100.0 })
            .push("Spcn", DescValue::UnitFloat { unit: "#Prc".into(), value: 25.0 })
            .push("Intr", DescValue::Bool(true))
            .push("flipX", DescValue::Bool(false))
            .push("flipY", DescValue::Bool(false))
            .push("sampledData", DescValue::Text("abc-123".into()));

        let mut preset = DescObject::new("brushPreset");
        preset.push("Nm  ", DescValue::Text("Test Brush".into()));
        preset.push("Brsh", DescValue::Object(tip));

        let mut root = DescObject::new("null");
        root.push("Brsh", DescValue::List(vec![DescValue::Object(preset)]));

        let bytes = write_descriptor(&root);
        let parsed = read_descriptor(&bytes).unwrap();

        let presets = parsed.get("Brsh").unwrap().as_list().unwrap();
        assert_eq!(presets.len(), 1);
        let preset = presets[0].as_object().unwrap();
        assert_eq!(preset.get("Nm  ").unwrap().as_text().unwrap(), "Test Brush");
        let tip = preset.get("Brsh").unwrap().as_object().unwrap();
        assert_eq!(tip.get("Dmtr").unwrap().as_unit_float().unwrap(), ("#Pxl", 512.0));
        assert_eq!(
            tip.get("sampledData").unwrap().as_text().unwrap(),
            "abc-123"
        );
    }

    #[test]
    fn parses_the_real_photoshop_desc_section() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/RakeBrushpackPhotoshopFinal.abr");
        let abr = std::fs::read(path).expect("fixture abr file");
        let samp_len = u32::from_be_bytes(abr[12..16].try_into().unwrap()) as usize;
        let end = 16 + samp_len;
        let patt_len = u32::from_be_bytes(abr[end + 8..end + 12].try_into().unwrap()) as usize;
        let after_patt = end + 12 + patt_len;
        let desc_len = u32::from_be_bytes(abr[after_patt + 8..after_patt + 12].try_into().unwrap()) as usize;
        let desc_start = after_patt + 12;

        let root = read_descriptor(&abr[desc_start..desc_start + desc_len]).unwrap();
        let presets = root.get("Brsh").unwrap().as_list().unwrap();
        assert_eq!(presets.len(), 28);
        let first = presets[0].as_object().unwrap();
        assert_eq!(first.get("Nm  ").unwrap().as_text().unwrap().trim(), "Very Basic Rake");
        let tip = first.get("Brsh").unwrap().as_object().unwrap();
        assert_eq!(tip.get("Dmtr").unwrap().as_unit_float().unwrap(), ("#Pxl", 700.0));
    }
}
