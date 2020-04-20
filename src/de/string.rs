//! Содержит реализацию конвертирования типа GFF строки в десериализатор
//! с помощью которого из него могут быть прочитаны другие совместимые типы.

use std::marker::PhantomData;

use serde::forward_to_deserialize_any;
use serde::de::{Deserializer, Error, IntoDeserializer, Visitor};

use crate::string::{GffString, StringKey};

impl<'de, E> IntoDeserializer<'de, E> for StringKey
  where E: Error,
{
  type Deserializer = StringKeyDeserializer<E>;

  #[inline]
  fn into_deserializer(self) -> Self::Deserializer {
    StringKeyDeserializer { value: self, marker: PhantomData }
  }
}

/// Десериализатор, использующий в качестве источника данных тип [`StringKey`].
///
/// Позволяет прочитать из ключа число, сформированное по формуле:
/// ```rust,ignore
/// ((language as u32) << 1) | gender as u32
/// ```
/// Именно в таком формате оно храниться в GFF файле.
///
/// [`StringKey`]: ../../enum.StringKey.html
#[derive(Debug)]
pub struct StringKeyDeserializer<E> {
  /// Источник данных, из которого достаются данные для десериализации других структур
  value: StringKey,
  /// Фиктивный элемент, для связывания типа ошибки `E`
  marker: PhantomData<E>,
}

impl<'de, E> Deserializer<'de> for StringKeyDeserializer<E>
  where E: Error,
{
  type Error = E;

  fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where V: Visitor<'de>,
  {
    visitor.visit_u32(self.value.into())
  }

  forward_to_deserialize_any!(
    bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str
    string bytes byte_buf option unit unit_struct newtype_struct seq
    tuple tuple_struct map struct enum identifier ignored_any
  );
}

///////////////////////////////////////////////////////////////////////////////////////////////////

impl<'de, E> IntoDeserializer<'de, E> for GffString
  where E: Error,
{
  type Deserializer = GffStringDeserializer<E>;

  #[inline]
  fn into_deserializer(self) -> Self::Deserializer {
    GffStringDeserializer { value: self, marker: PhantomData }
  }
}

/// Десериализатор, использующий в качестве источника данных тип [`GffString`].
///
/// В зависимости от типа хранимой строки позволяет прочитать из значения либо `u32`,
/// являющемся StrRef индексом, либо отображение из `u32` (содержащего комбинированное
/// значение языка и пола строки) на `String` с текстом строки для данного языка и пола.
///
/// [`GffString`]: ../../enum.GffString.html
#[derive(Debug)]
pub struct GffStringDeserializer<E> {
  /// Источник данных, из которого достаются данные для десериализации других структур
  value: GffString,
  /// Фиктивный элемент, для связывания типа ошибки `E`
  marker: PhantomData<E>,
}
impl<'de, E> Deserializer<'de> for GffStringDeserializer<E>
  where E: Error,
{
  type Error = E;

  fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where V: Visitor<'de>,
  {
    use self::GffString::*;

    match self.value {
      External(str_ref) => visitor.visit_u32(str_ref.0),
      Internal(strings) => visitor.visit_map(strings.into_deserializer()),
    }
  }

  forward_to_deserialize_any!(
    bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str
    string bytes byte_buf option unit unit_struct newtype_struct seq
    tuple tuple_struct map struct enum identifier ignored_any
  );
}
