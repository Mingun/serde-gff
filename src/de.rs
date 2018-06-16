//! Десериализатор для формата Bioware GFF (Generic File Format)

use std::io::{Read, Seek};
use encoding::{DecoderTrap, EncodingRef};
use serde::de::{self, Visitor, DeserializeSeed};

use value::SimpleValueRef;
use error::{Error, Result};
use parser::{Parser, Token};

/// Структура для поддержки чтения GFF файлов в экосистеме serde
pub struct Deserializer<R: Read + Seek> {
  /// Итератор, поставляющий токены в процессе разбора файла
  parser: Parser<R>,
  /// Подсмотренный вперед на один переход токен
  peeked: Option<Token>,
}

impl<R: Read + Seek> Deserializer<R> {
  /// Создает десериализатор для чтения GFF файла из указанного источника данных с использованием
  /// кодировки `UTF-8` для декодирования строк и генерацией ошибки в случае, если декодировать
  /// набор байт, как строку в этой кодировке, не удалось.
  ///
  /// # Параметры
  /// - `reader`: Источник данных для чтения файла
  ///
  /// # Ошибки
  /// В случае, если не удалось прочитать заголовок GFF файла -- например, он слишком короткий
  pub fn new(reader: R) -> Result<Self> {
    Ok(Deserializer { parser: Parser::new(reader)?, peeked: None })
  }
  /// Создает десериализатор для чтения GFF файла из указанного источника данных с использованием
  /// указанной кодировки для декодирования строк.
  ///
  /// # Параметры
  /// - `reader`: Источник данных для чтения файла
  /// - `encoding`: Кодировка для декодирования символов в строках
  /// - `trap`: Способ обработки символов в строках, которые не удалось декодировать с
  ///   использованием выбранной кодировки
  ///
  /// # Ошибки
  /// В случае, если не удалось прочитать заголовок GFF файла -- например, он слишком короткий
  pub fn with_encoding(reader: R, encoding: EncodingRef, trap: DecoderTrap) -> Result<Self> {
    Ok(Deserializer { parser: Parser::with_encoding(reader, encoding, trap)?, peeked: None })
  }

  /// Возвращает следующий токен из потока, поглощая его
  #[inline]
  fn next_token(&mut self) -> Result<Token> {
    match self.peeked.take() {
      Some(v) => Ok(v),
      None => self.parser.next_token(),
    }
  }
  /// Подсматривает следующий токен в потоке, не поглощая его
  fn peek_token(&mut self) -> Result<&Token> {
    if self.peeked.is_none() {
      self.peeked = Some(self.next_token()?);
    }
    match self.peeked {
      Some(ref value) => Ok(value),
      _ => unreachable!(),
    }
  }
}

macro_rules! unsupported {
  ($dser_method:ident) => (
    fn $dser_method<V>(self, _visitor: V) -> Result<V::Value>
      where V: Visitor<'de>,
    {
      let token = self.next_token()?;
      unimplemented!(concat!(stringify!($dser_method), " not yet supported. Token: {:?}"), token)
    }
  )
}
/// Реализует разбор простых типов данных.
///
/// # Параметры
/// - `dser_method`: реализуемый макросом метод
/// - `visit_method`: метод типажа [`Visitor`], который будет вызван для создания конечного значения
/// - `type`: тип GFF файла, одно из значений перечисления [`SimpleValueRef`]
/// 
/// [`Visitor`]: https://docs.serde.rs/serde/de/trait.Visitor.html
/// [`SimpleValueRef`]: ../enum.SimpleValueRef.html
macro_rules! primitive {
  ($dser_method:ident, $visit_method:ident, $type:ident) => (
    fn $dser_method<V>(self, visitor: V) -> Result<V::Value>
      where V: Visitor<'de>,
    {
      let token = self.next_token()?;
      if let Token::Value(SimpleValueRef::$type(value)) = token {
        return visitor.$visit_method(value);
      }
      return Err(Error::Unexpected(stringify!($type), token));
    }
  );
  ($dser_method:ident, $visit_method:ident, $type:ident, $read:ident) => (
    fn $dser_method<V>(self, visitor: V) -> Result<V::Value>
      where V: Visitor<'de>,
    {
      let token = self.next_token()?;
      if let Token::Value(SimpleValueRef::$type(value)) = token {
        return visitor.$visit_method(self.parser.$read(value)?);
      }
      return Err(Error::Unexpected(stringify!($type), token));
    }
  );
}
impl<'de, 'a, R: Read + Seek> de::Deserializer<'de> for &'a mut Deserializer<R> {
  type Error = Error;

  #[inline]
  fn is_human_readable(&self) -> bool { false }

  primitive!(deserialize_i8 , visit_i8 , Char);
  primitive!(deserialize_u8 , visit_u8 , Byte);
  primitive!(deserialize_i16, visit_i16, Short);
  primitive!(deserialize_u16, visit_u16, Word);
  primitive!(deserialize_i32, visit_i32, Int);
  primitive!(deserialize_u32, visit_u32, Dword);
  primitive!(deserialize_i64, visit_i64, Int64, read_i64);
  primitive!(deserialize_u64, visit_u64, Dword64, read_u64);
  primitive!(deserialize_f32, visit_f32, Float);
  primitive!(deserialize_f64, visit_f64, Double, read_f64);

  fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    if let Token::Value(SimpleValueRef::Byte(value)) = token {
      return visitor.visit_bool(value != 0);
    }
    return Err(Error::Unexpected("Byte", token));
  }
  fn deserialize_char<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    if let Token::Value(SimpleValueRef::Byte(value)) = token {
      return visitor.visit_char(value as char);
    }
    if let Token::Value(SimpleValueRef::Char(value)) = token {
      return visitor.visit_char(value as u8 as char);
    }
    return Err(Error::Unexpected("Byte, Char", token));
  }

  #[inline]
  fn deserialize_str<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    self.deserialize_string(visitor)
  }
  primitive!(deserialize_string, visit_string, String, read_string);
  #[inline]
  fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    self.deserialize_byte_buf(visitor)
  }
  primitive!(deserialize_byte_buf, visit_byte_buf, Void, read_byte_buf);

  /// Всегда разбирает любое значение, как `Some(...)`, формат не умеет хранить признак
  /// отсутствия значения. `None` в опциональные поля будет записываться только потому,
  /// что при десериализации данное поле не будет найдено
  #[inline]
  fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    visitor.visit_some(self)
  }
  /// Десериализует любую GFF структуру в `unit`, в остальных случаях выдает ошибку
  #[inline]
  fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    match token {
      Token::RootBegin { .. } |
      Token::ItemBegin { .. } |
      Token::StructBegin { .. } => {
        self.parser.skip_next(token);
        visitor.visit_unit()
      },
      token => Err(Error::Unexpected("RootBegin, ItemBegin, StructBegin", token)),
    }
  }
  unsupported!(deserialize_map);
  unsupported!(deserialize_seq);

  unsupported!(deserialize_any);

  fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    visitor.visit_none()
  }
  fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    if let Token::Label(index) = token {
      let label = self.parser.read_label(index)?;
      return visitor.visit_str(label.as_str()?);
    }
    return Err(Error::Unexpected("Label", token));
  }

  /// Десериализует любую GFF структуру в `unit`, в остальных случаях выдает ошибку
  #[inline]
  fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    self.deserialize_unit(visitor)
  }
  /// Разбирает в newtype структуру нижележащее значение
  #[inline]
  fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    visitor.visit_newtype_struct(self)
  }
  fn deserialize_tuple<V>(self, len: usize, _visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    unimplemented!("`deserialize_tuple(len: {})` not yet supported. Token: {:?}", len, token)
  }
  fn deserialize_tuple_struct<V>(self, name: &'static str, len: usize, _visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    unimplemented!("`deserialize_tuple_struct(name: {}, len: {})` not yet supported. Token: {:?}", name, len, token)
  }
  fn deserialize_struct<V>(self, name: &'static str, fields: &'static [&'static str], _visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    unimplemented!("`deserialize_struct(name: {}, fields: {})` not yet supported. Token: {:?}", name, fields.len(), token)
  }
  fn deserialize_enum<V>(self, name: &'static str, variants: &'static [&'static str], _visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    unimplemented!("`deserialize_enum(name: {}, variants: {})` not yet supported. Token: {:?}", name, variants.len(), token)
  }
}

impl<'de, 'a, R: Read + Seek> de::MapAccess<'de> for &'a mut Deserializer<R> {
  type Error = Error;

  fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where K: DeserializeSeed<'de>,
  {
    let token = self.peek_token()?.clone();
    match token {
      Token::RootEnd | Token::ItemEnd | Token::StructEnd => Ok(None),
      Token::Label(..) => seed.deserialize(&mut **self).map(Some),
      token => Err(Error::Unexpected("Label", token)),
    }
  }

  #[inline]
  fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where V: DeserializeSeed<'de>,
  {
    seed.deserialize(&mut **self)
  }
}

#[cfg(test)]
mod empty_file {
  //! Тестирование разбора пустого файла - содержащего только заголовок и структуру верхнего уровня
  use std::fs::File;
  use serde::de::Deserialize;
  use super::Deserializer;

  fn run<'de, T: Deserialize<'de>>(type_name: &str) -> T {
    // Читаемый файл содержит только одну пустую структуру верхнего уровня
    let file = File::open("test-data/empty.gff").expect("test file 'empty.gff' not exist");
    let mut deserializer = Deserializer::new(file).expect("can't read GFF header");

    Deserialize::deserialize(&mut deserializer).expect(&format!("can't deserialize to {}", type_name))
  }

  #[test]
  fn to_unit() {
    let _test: () = run("()");
  }

  #[test]
  fn to_unit_struct() {
    #[derive(Deserialize)]
    struct Unit;

    let _test: Unit = run("unit struct");
  }

  #[test]
  fn to_empty_struct() {
    #[derive(Deserialize)]
    struct Empty {}

    let _test: Empty = run("empty struct");
  }

  #[test]
  #[should_panic(expected = "missing field `_value`")]
  fn to_struct() {
    #[derive(Deserialize)]
    struct Struct { _value: i32 }

    let _test: Struct = run("struct with fields");
  }
}
