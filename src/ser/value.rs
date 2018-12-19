//! Содержит реализацию типажа `Serialize` для сериализации типа `Value`

use serde::ser::{Serialize, SerializeMap, Serializer};

use Label;
use value::Value;

impl Serialize for Label {
  #[inline]
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer,
  {
    serializer.serialize_bytes(self.as_ref())
  }
}

impl Serialize for Value {
  #[inline]
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer,
  {
    use self::Value::*;

    match *self {
      Byte(val)       => serializer.serialize_u8(val),
      Char(val)       => serializer.serialize_i8(val),
      Word(val)       => serializer.serialize_u16(val),
      Short(val)      => serializer.serialize_i16(val),
      Dword(val)      => serializer.serialize_u32(val),
      Int(val)        => serializer.serialize_i32(val),
      Dword64(val)    => serializer.serialize_u64(val),
      Int64(val)      => serializer.serialize_i64(val),
      Float(val)      => serializer.serialize_f32(val),
      Double(val)     => serializer.serialize_f64(val),
      String(ref val) => serializer.serialize_str(&val),
      ResRef(ref val) => serializer.serialize_bytes(&val.0),
      //TODO: реализовать сериализацию LocString
      LocString(ref _val) => unimplemented!("serialization of LocString not yet implemented"),
      Void(ref val)   => serializer.serialize_bytes(&val),
      Struct(ref val) => {
        let mut map = serializer.serialize_map(Some(val.len()))?;
        for (k, v) in val {
          map.serialize_key(k)?;
          map.serialize_value(v)?;
        }
        map.end()
      },
      List(ref val)   => val.serialize(serializer),
    }
  }
}
