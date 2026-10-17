//! Streaming ABI validation: never expand a guest byte list into JSON values.
use crate::{WasmError, WasmLimits};
use serde::Serialize;
use serde::ser::{self, Impossible};

pub(crate) fn bounded<T: Serialize>(value: &T, limits: &WasmLimits) -> Result<(), WasmError> {
    let mut budget = Budget {
        maximum: limits.payload_bytes.min(1024 * 1024),
        total: 0,
        path: std::any::type_name::<T>().into(),
        field: String::new(),
        depth: 0,
        bytes: false,
    };
    value.serialize(&mut budget)
}
struct Budget {
    maximum: usize,
    total: usize,
    path: String,
    field: String,
    depth: usize,
    bytes: bool,
}
impl Budget {
    fn add(&mut self, amount: usize) -> Result<(), WasmError> {
        self.total = self.total.saturating_add(amount);
        if self.total > self.maximum {
            Err(WasmError::OversizedPayload("aggregate".into()))
        } else {
            Ok(())
        }
    }
    fn nested<T: ?Sized + Serialize>(&mut self, field: &str, value: &T) -> Result<(), WasmError> {
        let previous = std::mem::replace(&mut self.field, field.into());
        let length = self.path.len();
        self.path.push('.');
        self.path.push_str(field);
        self.depth += 1;
        let result = if self.depth > 64 {
            Err(WasmError::OversizedPayload("depth".into()))
        } else {
            value.serialize(&mut *self)
        };
        self.depth -= 1;
        self.path.truncate(length);
        self.field = previous;
        result
    }
    fn list_maximum(&self) -> usize {
        match self.field.as_str() {
            "bytes" | "body" => 1024 * 1024,
            "value" => 64 * 1024,
            "nodes" => 4096,
            "counters" => 5,
            "messages" if self.path.contains("CompletionRequest") => 64,
            "usage" if self.path.contains("CompletionResponse") => 5,
            "fragments" | "headers" => 64,
            "effects" | "permission_upper_bound" | "before" | "after" | "roots" | "hosts" => 32,
            _ => 256,
        }
    }
    fn string(&mut self, value: &str) -> Result<(), WasmError> {
        let content = self.path.contains(".history.")
            || self.path.contains(".messages.")
            || self.path.contains(".fragments.");
        let maximum = match self.field.as_str() {
            "hosts" => 253,
            "id" | "name" | "revision" | "root" | "session" | "turn" | "request" | "call_id"
            | "before" | "after" | "requirements" | "roots" | "StatusItem" | "plugin_data_root" => {
                128
            }
            "key" if !self.path.contains(".nodes.") => 128,
            "parameters_json" => 64 * 1024,
            "arguments_json" | "outcome_json" => 1024 * 1024,
            "text" | "url" | "signature" | "Text" if content => 1024 * 1024,
            "system" | "text"
                if self.path.contains("CompletionRequest")
                    || self.path.contains("CompletionResponse") =>
            {
                1024 * 1024
            }
            _ => 4096,
        };
        if value.len() > maximum {
            return Err(WasmError::OversizedPayload(self.field.clone()));
        }
        if matches!(
            self.field.as_str(),
            "id" | "name"
                | "revision"
                | "root"
                | "key"
                | "session"
                | "turn"
                | "request"
                | "call_id"
                | "before"
                | "after"
                | "requirements"
                | "roots"
        ) && value.is_empty()
        {
            return Err(WasmError::MalformedResult(format!("empty {}", self.field)));
        }
        self.add(value.len().saturating_add(8))
    }
}
impl ser::Error for WasmError {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Self::MalformedResult("protocol serialization failed".into())
    }
}
struct Fields<'a>(&'a mut Budget);
struct Sequence<'a> {
    budget: &'a mut Budget,
    count: usize,
    maximum: usize,
    previous_bytes: bool,
    tuple_variant: Option<(String, usize)>,
}
impl<'a> ser::Serializer for &'a mut Budget {
    type Ok = ();
    type Error = WasmError;
    type SerializeSeq = Sequence<'a>;
    type SerializeTuple = Sequence<'a>;
    type SerializeTupleStruct = Sequence<'a>;
    type SerializeTupleVariant = Sequence<'a>;
    type SerializeMap = Impossible<(), WasmError>;
    type SerializeStruct = Fields<'a>;
    type SerializeStructVariant = Fields<'a>;
    fn serialize_bool(self, _: bool) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_i8(self, _: i8) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_i16(self, _: i16) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_i32(self, _: i32) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_i64(self, _: i64) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_u8(self, _: u8) -> Result<(), WasmError> {
        self.add(if self.bytes { 1 } else { 8 })
    }
    fn serialize_u16(self, _: u16) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_u32(self, _: u32) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_u64(self, _: u64) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_f32(self, v: f32) -> Result<(), WasmError> {
        self.serialize_f64(f64::from(v))
    }
    fn serialize_f64(self, v: f64) -> Result<(), WasmError> {
        if v.is_finite() {
            self.add(8)
        } else {
            Err(WasmError::MalformedResult("nonfinite number".into()))
        }
    }
    fn serialize_char(self, v: char) -> Result<(), WasmError> {
        self.add(v.len_utf8() + 8)
    }
    fn serialize_str(self, v: &str) -> Result<(), WasmError> {
        self.string(v)
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), WasmError> {
        if v.len() > self.list_maximum() {
            return Err(WasmError::OversizedPayload(self.field.clone()));
        }
        self.add(v.len() + 8)
    }
    fn serialize_none(self) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<(), WasmError> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), WasmError> {
        self.add(8)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<(), WasmError> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<(), WasmError> {
        self.nested(variant, v)
    }
    fn serialize_seq(self, length: Option<usize>) -> Result<Sequence<'a>, WasmError> {
        let maximum = self.list_maximum();
        if length.is_some_and(|len| len > maximum) {
            return Err(WasmError::OversizedPayload(self.field.clone()));
        }
        self.add(8)?;
        let previous_bytes = self.bytes;
        self.bytes = matches!(self.field.as_str(), "bytes" | "body" | "value");
        Ok(Sequence {
            budget: self,
            count: 0,
            maximum,
            previous_bytes,
            tuple_variant: None,
        })
    }
    fn serialize_tuple(self, length: usize) -> Result<Sequence<'a>, WasmError> {
        self.serialize_seq(Some(length))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        length: usize,
    ) -> Result<Sequence<'a>, WasmError> {
        self.serialize_seq(Some(length))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        length: usize,
    ) -> Result<Sequence<'a>, WasmError> {
        let previous = std::mem::replace(&mut self.field, variant.into());
        let path_length = self.path.len();
        self.path.push('.');
        self.path.push_str(variant);
        let mut seq = self.serialize_seq(Some(length))?;
        seq.tuple_variant = Some((previous, path_length));
        Ok(seq)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, WasmError> {
        Err(WasmError::MalformedResult("untyped protocol map".into()))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Fields<'a>, WasmError> {
        self.add(8)?;
        Ok(Fields(self))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Fields<'a>, WasmError> {
        self.add(8)?;
        Ok(Fields(self))
    }
}
impl ser::SerializeStruct for Fields<'_> {
    type Ok = ();
    type Error = WasmError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), WasmError> {
        self.0.nested(key, value)
    }
    fn end(self) -> Result<(), WasmError> {
        Ok(())
    }
}
impl ser::SerializeStructVariant for Fields<'_> {
    type Ok = ();
    type Error = WasmError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), WasmError> {
        self.0.nested(key, value)
    }
    fn end(self) -> Result<(), WasmError> {
        Ok(())
    }
}
impl Sequence<'_> {
    fn element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), WasmError> {
        self.count += 1;
        if self.count > self.maximum {
            return Err(WasmError::OversizedPayload(self.budget.field.clone()));
        }
        value.serialize(&mut *self.budget)
    }
    fn finish(self) -> Result<(), WasmError> {
        self.budget.bytes = self.previous_bytes;
        if let Some((field, length)) = self.tuple_variant {
            self.budget.field = field;
            self.budget.path.truncate(length);
        }
        Ok(())
    }
}
macro_rules! sequence {
    ($trait:ident, $method:ident) => {
        impl ser::$trait for Sequence<'_> {
            type Ok = ();
            type Error = WasmError;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), WasmError> {
                self.element(value)
            }
            fn end(self) -> Result<(), WasmError> {
                self.finish()
            }
        }
    };
}
sequence!(SerializeSeq, serialize_element);
sequence!(SerializeTuple, serialize_element);
sequence!(SerializeTupleStruct, serialize_field);
sequence!(SerializeTupleVariant, serialize_field);

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Serialize)]
    struct BytePayload {
        bytes: HugeList,
    }
    struct HugeList;
    impl Serialize for HugeList {
        fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            // A length check must reject before visiting even one list element.
            let _sequence = serializer.serialize_seq(Some(64 * 1024 * 1024))?;
            panic!("oversized list was traversed");
        }
    }
    #[test]
    fn oversized_byte_lists_are_refused_before_allocating_or_traversing_elements() {
        assert!(matches!(
            bounded(&BytePayload { bytes: HugeList }, &WasmLimits::default()),
            Err(WasmError::OversizedPayload(_))
        ));
    }
}
