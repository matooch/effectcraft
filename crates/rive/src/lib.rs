//! Rive runtime files (`.riv`): reading and writing.
//!
//! Implemented from Rive's public format description
//! (<https://rive.app/docs/runtimes/advanced-topic/format>) and the object model the Rive CLI
//! reports (`rive schema <Type> --json`, turned into [`defs`] by `cargo xtask rive-defs`); no
//! runtime source was used. Test files and reference frames come from the Rive CLI, used only as
//! an external oracle (`fixtures/rive/build.sh`), like ffmpeg for the codecs.
//!
//! **The stream** ([`RivFile::read`], [`RivFile::write`]): a header (`RIVE`, major and minor
//! version, file id), a table of contents giving the field type of every property newer than
//! the major version, then objects: a type key and `(property key, value)` pairs ended by 0.
//! Values are LEB128 unsigned integers (uints, ids, bools, enums), little-endian `f32`s,
//! little-endian `u32` colours, or length-prefixed bytes (strings, byte blobs, id lists). Reading
//! keeps every object and property, known or not, so writing it back gives the same bytes.
//!
//! **The document** ([`Document::from_file`]): artboards with their object lists (the indices
//! `parentId`, `objectId` and `interpolatorId` refer to), linear animations with their keyed
//! objects, properties, keyframes and interpolators. Objects are read in context: a component
//! belongs to the last artboard, a keyed object to the last animation, and so on.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod defs;
pub mod import;
mod model;

pub use import::{ImportResult, import};
pub use model::{
    Artboard, Document, Interp, Interpolator, KeyFrame, KeyValue, KeyedObject, KeyedProperty, LinearAnimation, Loop, Object, StateMachineInfo, keys,
};

/// The major version this reader understands (runtimes only read their own major version).
pub const MAJOR_VERSION: u64 = 7;

/// Largest number of objects or properties read from one file (hostile-input cap).
const MAX_ITEMS: usize = 4_000_000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RiveError {
    #[error("not a Rive file (no RIVE fingerprint)")]
    NotRive,
    #[error("Rive format {major}.{minor} is not supported (this reader reads major version {MAJOR_VERSION})")]
    Version { major: u64, minor: u64 },
    #[error("the Rive file ends early ({0})")]
    Truncated(&'static str),
    #[error("the Rive file is malformed: {0}")]
    Malformed(String),
}

/// How a property's value is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// LEB128 unsigned integer: uints, ids, bools, enums.
    Uint,
    /// Little-endian IEEE 754 `f32`.
    Float,
    /// Length-prefixed UTF-8.
    String,
    /// Length-prefixed bytes (byte blobs, id lists).
    Bytes,
    /// Little-endian `u32`, `0xAARRGGBB`.
    Color,
}

impl Field {
    /// The table-of-contents code (2 bits per property).
    fn toc_code(self) -> u32 {
        match self {
            Field::Uint => 0,
            Field::String | Field::Bytes => 1,
            Field::Float => 2,
            Field::Color => 3,
        }
    }
    fn from_toc(code: u32) -> Field {
        match code & 3 {
            1 => Field::Bytes,
            2 => Field::Float,
            3 => Field::Color,
            _ => Field::Uint,
        }
    }
}

/// A property value as stored.
#[derive(Clone, Debug, PartialEq)]
pub enum RawValue {
    Uint(u64),
    Float(f32),
    String(String),
    Bytes(Vec<u8>),
    Color(u32),
}

impl RawValue {
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            RawValue::Uint(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            RawValue::Float(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            RawValue::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_color(&self) -> Option<u32> {
        match self {
            RawValue::Color(c) => Some(*c),
            _ => None,
        }
    }
}

/// One object of the stream: its type key and its properties in file order.
#[derive(Clone, Debug, PartialEq)]
pub struct RawObject {
    pub type_key: u32,
    pub props: Vec<(u32, RawValue)>,
}

impl RawObject {
    pub fn new(type_key: u32) -> RawObject {
        RawObject { type_key, props: Vec::new() }
    }
    /// The type's name from the object model, if known.
    pub fn type_name(&self) -> Option<&'static str> {
        type_name(self.type_key)
    }
    pub fn get(&self, key: u32) -> Option<&RawValue> {
        self.props.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }
    pub fn uint(&self, key: u32) -> Option<u64> {
        self.get(key).and_then(RawValue::as_u64)
    }
    pub fn float(&self, key: u32) -> Option<f32> {
        self.get(key).and_then(RawValue::as_f32)
    }
    pub fn string(&self, key: u32) -> Option<&str> {
        self.get(key).and_then(RawValue::as_str)
    }
    pub fn color(&self, key: u32) -> Option<u32> {
        self.get(key).and_then(RawValue::as_color)
    }
    /// Set a property (replacing an earlier value of the same key).
    pub fn set(&mut self, key: u32, value: RawValue) {
        match self.props.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.props.push((key, value)),
        }
    }
}

/// A whole `.riv` file as a flat object stream.
#[derive(Clone, Debug, PartialEq)]
pub struct RivFile {
    pub major: u64,
    pub minor: u64,
    pub file_id: u64,
    /// The table of contents: properties whose field type the file declares, in file order.
    pub toc: Vec<(u32, Field)>,
    pub objects: Vec<RawObject>,
}

/// Name of a type key in the object model.
pub fn type_name(type_key: u32) -> Option<&'static str> {
    defs::TYPES.binary_search_by_key(&type_key, |t| t.0).ok().and_then(|i| defs::TYPES.get(i)).map(|t| t.1)
}

/// Type key of a type name.
pub fn type_key(name: &str) -> Option<u32> {
    defs::TYPES.iter().find(|t| t.1 == name).map(|t| t.0)
}

/// Whether objects of the type join their artboard's object list.
pub(crate) fn in_artboard(type_key: u32) -> bool {
    defs::TYPES.binary_search_by_key(&type_key, |t| t.0).ok().and_then(|i| defs::TYPES.get(i)).is_some_and(|t| t.2)
}

/// Name and field type of a property key in the object model.
pub fn property(key: u32) -> Option<(&'static str, Field)> {
    defs::PROPS.binary_search_by_key(&key, |p| p.0).ok().and_then(|i| defs::PROPS.get(i)).map(|p| (p.1, p.3))
}

// ------------------------------------------------------------------------------ reading

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.b.len()
    }
    fn varuint(&mut self, what: &'static str) -> Result<u64, RiveError> {
        let mut v: u64 = 0;
        for i in 0..10u32 {
            let byte = *self.b.get(self.pos).ok_or(RiveError::Truncated(what))?;
            self.pos += 1;
            let bits = u64::from(byte & 0x7f);
            let shift = 7 * i;
            if shift == 63 && bits > 1 {
                return Err(RiveError::Malformed(format!("{what} overflows 64 bits")));
            }
            v |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(RiveError::Malformed(format!("{what} is longer than 10 bytes")))
    }
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], RiveError> {
        let end = self.pos.checked_add(n).ok_or(RiveError::Truncated(what))?;
        let s = self.b.get(self.pos..end).ok_or(RiveError::Truncated(what))?;
        self.pos = end;
        Ok(s)
    }
    fn u32_le(&mut self, what: &'static str) -> Result<u32, RiveError> {
        let s = self.take(4, what)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(s);
        Ok(u32::from_le_bytes(a))
    }
    fn bytes(&mut self, what: &'static str) -> Result<&'a [u8], RiveError> {
        let n = self.varuint(what)?;
        let n = usize::try_from(n).map_err(|_| RiveError::Truncated(what))?;
        self.take(n, what)
    }
    fn key(&mut self, what: &'static str) -> Result<u32, RiveError> {
        let k = self.varuint(what)?;
        u32::try_from(k).map_err(|_| RiveError::Malformed(format!("{what} {k} is out of range")))
    }
}

impl RivFile {
    /// Read a `.riv` file. Unknown types are kept; a property the object model doesn't know
    /// and the table of contents doesn't declare can't be skipped and is an error.
    pub fn read(bytes: &[u8]) -> Result<RivFile, RiveError> {
        let mut r = Reader { b: bytes, pos: 0 };
        if r.take(4, "fingerprint").map_err(|_| RiveError::NotRive)? != b"RIVE" {
            return Err(RiveError::NotRive);
        }
        let major = r.varuint("major version")?;
        let minor = r.varuint("minor version")?;
        if major != MAJOR_VERSION {
            return Err(RiveError::Version { major, minor });
        }
        let file_id = r.varuint("file id")?;
        let mut toc_keys = Vec::new();
        loop {
            let k = r.key("table of contents")?;
            if k == 0 {
                break;
            }
            if toc_keys.len() >= MAX_ITEMS {
                return Err(RiveError::Malformed("the table of contents is too long".into()));
            }
            toc_keys.push(k);
        }
        let mut toc = Vec::with_capacity(toc_keys.len());
        let mut word = 0u32;
        for (i, k) in toc_keys.into_iter().enumerate() {
            if i % 16 == 0 {
                word = r.u32_le("table of contents field types")?;
            }
            toc.push((k, Field::from_toc(word >> ((i % 16) * 2))));
        }
        let mut objects = Vec::new();
        let mut count = 0usize;
        while !r.at_end() {
            let type_key = r.key("object type")?;
            let mut obj = RawObject::new(type_key);
            loop {
                let key = r.key("property key")?;
                if key == 0 {
                    break;
                }
                count += 1;
                if count > MAX_ITEMS {
                    return Err(RiveError::Malformed("too many properties".into()));
                }
                let field = match property(key) {
                    Some((_, f)) => f,
                    None => toc.iter().find(|(k, _)| *k == key).map(|(_, f)| *f).ok_or_else(|| {
                        RiveError::Malformed(format!("property {key} of {} is unknown and not in the table of contents", type_label(type_key)))
                    })?,
                };
                let value = match field {
                    Field::Uint => RawValue::Uint(r.varuint("property value")?),
                    Field::Float => RawValue::Float(f32::from_bits(r.u32_le("property value")?)),
                    Field::Color => RawValue::Color(r.u32_le("property value")?),
                    Field::String => RawValue::String(String::from_utf8_lossy(r.bytes("property value")?).into_owned()),
                    Field::Bytes => RawValue::Bytes(r.bytes("property value")?.to_vec()),
                };
                obj.props.push((key, value));
            }
            objects.push(obj);
            if objects.len() > MAX_ITEMS {
                return Err(RiveError::Malformed("too many objects".into()));
            }
        }
        Ok(RivFile { major, minor, file_id, toc, objects })
    }

    /// Write the file. Properties are written as stored; the table of contents is written as
    /// stored plus any property the object stream uses that neither the object model nor the
    /// table of contents declares (whose field type follows from its value).
    pub fn write(&self) -> Vec<u8> {
        let mut toc = self.toc.clone();
        for o in &self.objects {
            for (k, v) in &o.props {
                if property(*k).is_none() && !toc.iter().any(|(t, _)| t == k) {
                    toc.push((*k, field_of(v)));
                }
            }
        }
        let mut out = b"RIVE".to_vec();
        put_varuint(&mut out, self.major);
        put_varuint(&mut out, self.minor);
        put_varuint(&mut out, self.file_id);
        for (k, _) in &toc {
            put_varuint(&mut out, u64::from(*k));
        }
        put_varuint(&mut out, 0);
        for chunk in toc.chunks(16) {
            let word = chunk.iter().enumerate().fold(0u32, |w, (i, (_, f))| w | (f.toc_code() << (i * 2)));
            out.extend_from_slice(&word.to_le_bytes());
        }
        for o in &self.objects {
            put_varuint(&mut out, u64::from(o.type_key));
            for (k, v) in &o.props {
                put_varuint(&mut out, u64::from(*k));
                match v {
                    RawValue::Uint(x) => put_varuint(&mut out, *x),
                    RawValue::Float(x) => out.extend_from_slice(&x.to_bits().to_le_bytes()),
                    RawValue::Color(x) => out.extend_from_slice(&x.to_le_bytes()),
                    RawValue::String(s) => {
                        put_varuint(&mut out, s.len() as u64);
                        out.extend_from_slice(s.as_bytes());
                    }
                    RawValue::Bytes(b) => {
                        put_varuint(&mut out, b.len() as u64);
                        out.extend_from_slice(b);
                    }
                }
            }
            put_varuint(&mut out, 0);
        }
        out
    }
}

fn field_of(v: &RawValue) -> Field {
    match v {
        RawValue::Uint(_) => Field::Uint,
        RawValue::Float(_) => Field::Float,
        RawValue::String(_) => Field::String,
        RawValue::Bytes(_) => Field::Bytes,
        RawValue::Color(_) => Field::Color,
    }
}

fn put_varuint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

pub(crate) fn type_label(type_key: u32) -> String {
    type_name(type_key).map(str::to_string).unwrap_or_else(|| format!("type {type_key}"))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_import;
