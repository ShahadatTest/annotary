//! The deterministic writer: JSON in the *leaf-objects-inline* layout.
//!
//! Rules (from the sidecar prototype's `to_json_smart`): an object or array whose values are all
//! scalars prints on one line (`{"k": v, "k2": v2}` / `[v, v2]`); anything containing a container
//! nests with 2-space indentation, one entry per line; empty containers print as `{}` / `[]`; the
//! file ends with a single newline. Key order is the struct field order of [`crate::model`]; the
//! same input always yields the same bytes.
//!
//! Implementation: any `Serialize` value is first turned into an ordered [`Node`] tree (a tiny
//! serde `Serializer`, so struct field order is preserved without `preserve_order`), then laid
//! out. This is what keeps later tickets' additions (assets, threads, actions) automatic: derive
//! `Serialize` in field order with `skip_serializing_if` for the omit-when-empty rules and the
//! layout follows.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;

use serde::Serialize;
use serde::ser::{self, Impossible};

/// An ordered JSON tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Null,
    Bool(bool),
    /// Already-formatted JSON number text.
    Number(String),
    Str(String),
    Arr(Vec<Node>),
    Obj(Vec<(String, Node)>),
}

impl Node {
    fn is_scalar(&self) -> bool {
        !matches!(self, Node::Arr(_) | Node::Obj(_))
    }
}

/// Serialize `value` into the canonical Sidecar text (with the trailing newline).
pub fn emit<T: Serialize>(value: &T) -> String {
    let node = to_node(value).expect("Sidecar types serialise");
    let mut out = String::new();
    layout(&node, 0, &mut out);
    out.push('\n');
    out
}

/// Turn any `Serialize` value into a [`Node`] tree.
pub fn to_node<T: Serialize>(value: &T) -> Result<Node, Error> {
    value.serialize(NodeSerializer)
}

/// Write `node` in the leaf-inline layout at `indent` levels.
pub fn layout(node: &Node, indent: usize, out: &mut String) {
    match node {
        Node::Null => out.push_str("null"),
        Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Node::Number(n) => out.push_str(n),
        Node::Str(s) => out.push_str(&json_string(s)),
        Node::Arr(items) => {
            if items.is_empty() {
                out.push_str("[]");
            } else if items.iter().all(Node::is_scalar) {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    layout(item, indent, out);
                }
                out.push(']');
            } else {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(",\n");
                    }
                    pad(indent + 1, out);
                    layout(item, indent + 1, out);
                }
                out.push('\n');
                pad(indent, out);
                out.push(']');
            }
        }
        Node::Obj(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
            } else if entries.iter().all(|(_, v)| v.is_scalar()) {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&json_string(k));
                    out.push_str(": ");
                    layout(v, indent, out);
                }
                out.push('}');
            } else {
                out.push_str("{\n");
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(",\n");
                    }
                    pad(indent + 1, out);
                    out.push_str(&json_string(k));
                    out.push_str(": ");
                    layout(v, indent + 1, out);
                }
                out.push('\n');
                pad(indent, out);
                out.push('}');
            }
        }
    }
}

fn pad(indent: usize, out: &mut String) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

/// A JSON string literal (serde_json escaping: `"`, `\`, control characters; non-ASCII verbatim).
pub fn json_string(s: &str) -> String {
    serde_json::to_string(s).expect("string serialises")
}

// ---------------------------------------------------------------- serde → Node

/// Serialization failure (only for shapes the Sidecar never uses, e.g. non-string map keys).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl ser::StdError for Error {}

impl ser::Error for Error {
    fn custom<T: core::fmt::Display>(msg: T) -> Self {
        Error(msg.to_string())
    }
}

struct NodeSerializer;

fn num<T: core::fmt::Display>(v: T) -> Result<Node, Error> {
    let mut s = String::new();
    let _ = write!(s, "{v}");
    Ok(Node::Number(s))
}

impl ser::Serializer for NodeSerializer {
    type Ok = Node;
    type Error = Error;
    type SerializeSeq = SeqBuilder;
    type SerializeTuple = SeqBuilder;
    type SerializeTupleStruct = SeqBuilder;
    type SerializeTupleVariant = SeqBuilder;
    type SerializeMap = MapBuilder;
    type SerializeStruct = MapBuilder;
    type SerializeStructVariant = MapBuilder;

    fn serialize_bool(self, v: bool) -> Result<Node, Error> {
        Ok(Node::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_i16(self, v: i16) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_i32(self, v: i32) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_i64(self, v: i64) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_u8(self, v: u8) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_u16(self, v: u16) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_u32(self, v: u32) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_u64(self, v: u64) -> Result<Node, Error> {
        num(v)
    }
    fn serialize_f32(self, v: f32) -> Result<Node, Error> {
        Ok(Node::Number(serde_json::to_string(&v).unwrap_or_else(|_| "null".into())))
    }
    fn serialize_f64(self, v: f64) -> Result<Node, Error> {
        Ok(Node::Number(serde_json::to_string(&v).unwrap_or_else(|_| "null".into())))
    }
    fn serialize_char(self, v: char) -> Result<Node, Error> {
        Ok(Node::Str(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<Node, Error> {
        Ok(Node::Str(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Node, Error> {
        Ok(Node::Arr(v.iter().map(|b| Node::Number(b.to_string())).collect()))
    }
    fn serialize_none(self) -> Result<Node, Error> {
        Ok(Node::Null)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<Node, Error> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<Node, Error> {
        Ok(Node::Null)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Node, Error> {
        Ok(Node::Null)
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<Node, Error> {
        Ok(Node::Str(variant.to_string()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, v: &T) -> Result<Node, Error> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<Node, Error> {
        Ok(Node::Obj(alloc::vec![(variant.to_string(), v.serialize(NodeSerializer)?)]))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SeqBuilder, Error> {
        Ok(SeqBuilder { items: Vec::with_capacity(len.unwrap_or(0)), variant: None })
    }
    fn serialize_tuple(self, len: usize) -> Result<SeqBuilder, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<SeqBuilder, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<SeqBuilder, Error> {
        Ok(SeqBuilder { items: Vec::with_capacity(len), variant: Some(variant) })
    }
    fn serialize_map(self, len: Option<usize>) -> Result<MapBuilder, Error> {
        Ok(MapBuilder { entries: Vec::with_capacity(len.unwrap_or(0)), pending: None, variant: None })
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> Result<MapBuilder, Error> {
        self.serialize_map(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<MapBuilder, Error> {
        Ok(MapBuilder { entries: Vec::with_capacity(len), pending: None, variant: Some(variant) })
    }
}

pub struct SeqBuilder {
    items: Vec<Node>,
    variant: Option<&'static str>,
}

impl SeqBuilder {
    fn finish(self) -> Node {
        match self.variant {
            Some(v) => Node::Obj(alloc::vec![(v.to_string(), Node::Arr(self.items))]),
            None => Node::Arr(self.items),
        }
    }
}

impl ser::SerializeSeq for SeqBuilder {
    type Ok = Node;
    type Error = Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        self.items.push(v.serialize(NodeSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Node, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeTuple for SeqBuilder {
    type Ok = Node;
    type Error = Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        ser::SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Node, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeTupleStruct for SeqBuilder {
    type Ok = Node;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        ser::SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Node, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeTupleVariant for SeqBuilder {
    type Ok = Node;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        ser::SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Node, Error> {
        Ok(self.finish())
    }
}

pub struct MapBuilder {
    entries: Vec<(String, Node)>,
    pending: Option<String>,
    variant: Option<&'static str>,
}

impl MapBuilder {
    fn finish(self) -> Node {
        match self.variant {
            Some(v) => Node::Obj(alloc::vec![(v.to_string(), Node::Obj(self.entries))]),
            None => Node::Obj(self.entries),
        }
    }
}

impl ser::SerializeMap for MapBuilder {
    type Ok = Node;
    type Error = Error;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Error> {
        self.pending = Some(key.serialize(KeySerializer)?);
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        let key = self.pending.take().ok_or_else(|| Error("value without key".into()))?;
        self.entries.push((key, v.serialize(NodeSerializer)?));
        Ok(())
    }
    fn end(self) -> Result<Node, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeStruct for MapBuilder {
    type Ok = Node;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, v: &T) -> Result<(), Error> {
        self.entries.push((key.to_string(), v.serialize(NodeSerializer)?));
        Ok(())
    }
    fn end(self) -> Result<Node, Error> {
        Ok(self.finish())
    }
}

impl ser::SerializeStructVariant for MapBuilder {
    type Ok = Node;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, v: &T) -> Result<(), Error> {
        ser::SerializeStruct::serialize_field(self, key, v)
    }
    fn end(self) -> Result<Node, Error> {
        Ok(self.finish())
    }
}

/// Map keys must be strings (JSON).
struct KeySerializer;

fn bad_key<T>() -> Result<T, Error> {
    Err(Error("map keys must be strings".into()))
}

impl ser::Serializer for KeySerializer {
    type Ok = String;
    type Error = Error;
    type SerializeSeq = Impossible<String, Error>;
    type SerializeTuple = Impossible<String, Error>;
    type SerializeTupleStruct = Impossible<String, Error>;
    type SerializeTupleVariant = Impossible<String, Error>;
    type SerializeMap = Impossible<String, Error>;
    type SerializeStruct = Impossible<String, Error>;
    type SerializeStructVariant = Impossible<String, Error>;

    fn serialize_str(self, v: &str) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_char(self, v: char) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<String, Error> {
        Ok(variant.to_string())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, v: &T) -> Result<String, Error> {
        v.serialize(self)
    }
    fn serialize_bool(self, _: bool) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_i8(self, v: i8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u8(self, v: u8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_f32(self, _: f32) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_f64(self, _: f64) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_none(self) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_some<T: ?Sized + Serialize>(self, _: &T) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_unit(self) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<String, Error> {
        bad_key()
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        bad_key()
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        bad_key()
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, Error> {
        bad_key()
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        bad_key()
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        bad_key()
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        bad_key()
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        bad_key()
    }
}
