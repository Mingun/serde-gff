//! Содержит реализацию типажа `Deserialize` для десериализации типа `Value`

use std::fmt;
use indexmap::IndexMap;
use serde::de::{Deserialize, Deserializer, Error, SeqAccess, MapAccess, Visitor};

use Label;
use value::Value;

/// Структура для конвертации событий десериализации от serde в объект `Label`
struct LabelVisitor;

impl<'de> Visitor<'de> for LabelVisitor {
  type Value = Label;

  fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
    formatter.write_str("a string with length in UTF-8 <=16, byte buffer with length <=16, or char")
  }

  #[inline]
  fn visit_char<E>(self, value: char) -> Result<Label, E>
    where E: Error,
  {
    self.visit_string(value.to_string())
  }
  #[inline]
  fn visit_str<E>(self, value: &str) -> Result<Label, E>
    where E: Error,
  {
    self.visit_bytes(value.as_bytes())
  }

  #[inline]
  fn visit_bytes<E>(self, value: &[u8]) -> Result<Label, E>
    where E: Error,
  {
    use error::Error::TooLongLabel;

    match Label::from_bytes(value) {
      Ok(label) => Ok(label),
      Err(TooLongLabel(len)) => Err(E::invalid_length(len, &self)),
      Err(err) => Err(E::custom(err)),// На самом деле, этот вариант невозможен
    }
  }
}

/// Десериализует метку из строки или массива байт
impl<'de> Deserialize<'de> for Label {
  #[inline]
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de>,
  {
    deserializer.deserialize_any(LabelVisitor)
  }
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// Макрос, создающий функцию конвертации события serde в один из вариантов GFF значения
macro_rules! value_from_primitive {
  ($name:ident, $type:ty => $variant:ident) => (
    #[inline]
    fn $name<E>(self, value: $type) -> Result<Value, E> {
      Ok(Value::$variant(value.into()))
    }
  );
}

/// Структура для конвертации событий десериализации от serde в объект `Value`
struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
  type Value = Value;

  fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
    formatter.write_str("any valid GFF value")
  }

  #[inline]
  fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
    Ok(Value::Byte(if value { 1 } else { 0 }))
  }

  value_from_primitive!(visit_i8 , i8  => Char);
  value_from_primitive!(visit_i16, i16 => Short);
  value_from_primitive!(visit_i32, i32 => Int);
  value_from_primitive!(visit_i64, i64 => Int64);
  //visit_i128 - не поддерживается

  value_from_primitive!(visit_u8 , u8  => Byte);
  value_from_primitive!(visit_u16, u16 => Word);
  value_from_primitive!(visit_u32, u32 => Dword);
  value_from_primitive!(visit_u64, u64 => Dword64);
  //visit_u128 - не поддерживается

  value_from_primitive!(visit_f32, f32 => Float);
  value_from_primitive!(visit_f64, f64 => Double);

  //visit_char - не поддерживается

  value_from_primitive!(visit_str, &str => String);
  //visit_borrowed_str - устраивает реализация по умолчанию
  value_from_primitive!(visit_string, String => String);

  value_from_primitive!(visit_bytes, &[u8] => Void);
  //visit_borrowed_bytes - устраивает реализация по умолчанию
  value_from_primitive!(visit_byte_buf, Vec<u8> => Void);

  //visit_none - не поддерживается
  #[inline]
  fn visit_some<D>(self, deserializer: D) -> Result<Value, D::Error>
    where D: Deserializer<'de>,
  {
    Deserialize::deserialize(deserializer)
  }

  #[inline]
  fn visit_unit<E>(self) -> Result<Value, E> {
    Ok(Value::Struct(IndexMap::with_capacity(0)))
  }
  //visit_newtype_struct - не поддерживается

  #[inline]
  fn visit_seq<V>(self, mut seq: V) -> Result<Value, V::Error>
    where V: SeqAccess<'de>,
  {
    let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));

    while let Some(elem) = seq.next_element()? {
      vec.push(elem);
    }

    Ok(Value::List(vec))
  }
  fn visit_map<V>(self, mut map: V) -> Result<Value, V::Error>
    where V: MapAccess<'de>,
  {
    let mut values = IndexMap::with_capacity(map.size_hint().unwrap_or(0));

    while let Some((key, value)) = map.next_entry()? {
      values.insert(key, value);
    }

    Ok(Value::Struct(values))
  }
  //visit_enum - не поддерживается
}

impl<'de> Deserialize<'de> for Value {
  #[inline]
  fn deserialize<D>(deserializer: D) -> Result<Value, D::Error>
    where D: Deserializer<'de>,
  {
    deserializer.deserialize_any(ValueVisitor)
  }
}
