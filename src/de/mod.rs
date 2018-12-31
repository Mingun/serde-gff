//! Десериализатор для формата Bioware GFF (Generic File Format)

use std::io::{Read, Seek};
use encoding::{DecoderTrap, EncodingRef};
use serde::de::{self, Visitor, DeserializeSeed, IntoDeserializer};

use string::GffString;
use value::SimpleValueRef;
use error::{Error, Result};
use parser::{Parser, Token};

mod string;
mod value;

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
  /// Десериализует все примитивные типы GFF файла (все типы, кроме структур и списков)
  fn deserialize_value<'de, V>(&mut self, value: SimpleValueRef, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    use self::SimpleValueRef::*;

    match value {
      Byte(val)     => visitor.visit_u8(val),
      Char(val)     => visitor.visit_i8(val),
      Word(val)     => visitor.visit_u16(val),
      Short(val)    => visitor.visit_i16(val),
      Dword(val)    => visitor.visit_u32(val),
      Int(val)      => visitor.visit_i32(val),
      Dword64(val)  => visitor.visit_u64(self.parser.read_u64(val)?),
      Int64(val)    => visitor.visit_i64(self.parser.read_i64(val)?),
      Float(val)    => visitor.visit_f32(val),
      Double(val)   => visitor.visit_f64(self.parser.read_f64(val)?),
      String(val)   => visitor.visit_string(self.parser.read_string(val)?),
      ResRef(val)   => {
        let resref = self.parser.read_resref(val)?;
        if let Ok(str) = resref.as_str() {
          return visitor.visit_str(str);
        }
        visitor.visit_byte_buf(resref.0)
      },
      LocString(val)=> {
        use serde::Deserializer;

        let value: GffString = self.parser.read_loc_string(val)?.into();
        value.into_deserializer().deserialize_any(visitor)
      },
      Void(val)     => visitor.visit_byte_buf(self.parser.read_byte_buf(val)?),
    }
  }
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
macro_rules! complex {
  ($token:ident, $self:ident, $visitor:ident . $method:ident) => (
    {
      let value = $visitor.$method(&mut *$self)?;
      let token = $self.next_token()?;
      if let Token::$token = token {
        Ok(value)
      } else {
        Err(Error::Unexpected(stringify!($token), token))
      }
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
  fn deserialize_string<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    match token {
      Token::Value(SimpleValueRef::String(value)) => {
        visitor.visit_string(self.parser.read_string(value)?)
      },
      Token::Value(SimpleValueRef::ResRef(value)) => {
        visitor.visit_string(self.parser.read_resref(value)?.as_string()?)
      },
      _ => Err(Error::Unexpected("String, ResRef", token)),
    }
  }
  #[inline]
  fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    self.deserialize_byte_buf(visitor)
  }
  fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    match token {
      Token::Value(SimpleValueRef::Void(value)) => {
        visitor.visit_byte_buf(self.parser.read_byte_buf(value)?)
      },
      Token::Value(SimpleValueRef::ResRef(value)) => {
        visitor.visit_byte_buf(self.parser.read_resref(value)?.0)
      },
      _ => Err(Error::Unexpected("Void, ResRef", token)),
    }
  }

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

  fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    match token {
      Token::Value(value)       => self.deserialize_value(value, visitor),
      Token::ListBegin { .. }   => complex!(ListEnd, self, visitor.visit_seq),
      Token::RootBegin { .. }   => complex!(RootEnd, self, visitor.visit_map),
      Token::ItemBegin { .. }   => complex!(ItemEnd, self, visitor.visit_map),
      Token::StructBegin { .. } => complex!(StructEnd, self, visitor.visit_map),
      Token::Label(index) => {
        let label = self.parser.read_label(index)?;
        visitor.visit_str(label.as_str()?)
      },
      _ => unimplemented!("`deserialize_any`, token: {:?}", token)
    }
  }
  fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    self.parser.skip_next(token);
    visitor.visit_none()
  }
  /// Данный метод вызывается при необходимости десериализовать идентификатор перечисления
  fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    use self::SimpleValueRef::*;

    let token = self.next_token()?;
    match token {
      //TODO: После решения https://github.com/serde-rs/serde/issues/745 можно будет передавать числа
      Token::Value(Byte(val))     => visitor.visit_string(val.to_string()),
      Token::Value(Char(val))     => visitor.visit_string(val.to_string()),
      Token::Value(Word(val))     => visitor.visit_string(val.to_string()),
      Token::Value(Short(val))    => visitor.visit_string(val.to_string()),
      Token::Value(Dword(val))    => visitor.visit_string(val.to_string()),
      Token::Value(Int(val))      => visitor.visit_string(val.to_string()),
      Token::Value(Dword64(val))  => visitor.visit_string(self.parser.read_u64(val)?.to_string()),
      Token::Value(Int64(val))    => visitor.visit_string(self.parser.read_i64(val)?.to_string()),
      Token::Value(String(val))   => visitor.visit_string(self.parser.read_string(val)?),
      _ => Err(Error::Unexpected("Byte, Char, Word, Short, Dword, Int, Int64, String", token)),
    }
  }

  fn deserialize_map<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    match token {
      Token::RootBegin   { .. } => complex!(RootEnd,   self, visitor.visit_map),
      Token::ItemBegin   { .. } => complex!(ItemEnd,   self, visitor.visit_map),
      Token::StructBegin { .. } => complex!(StructEnd, self, visitor.visit_map),
      token => Err(Error::Unexpected("RootBegin, ItemBegin, StructBegin", token)),
    }
  }
  fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    match token {
      Token::ListBegin { .. } => complex!(ListEnd, self, visitor.visit_seq),
      token => Err(Error::Unexpected("ListBegin", token)),
    }
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
  fn deserialize_struct<V>(self, _name: &'static str, _fields: &'static [&'static str], visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    self.deserialize_map(visitor)
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
      Token::Label(..) => seed.deserialize(Field(&mut **self)).map(Some),
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

impl<'de, 'a, R: Read + Seek> de::SeqAccess<'de> for &'a mut Deserializer<R> {
  type Error = Error;

  fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where T: DeserializeSeed<'de>,
  {
    let token = self.peek_token()?.clone();
    match token {
      Token::ListEnd => Ok(None),
      Token::ItemBegin { .. } => seed.deserialize(&mut **self).map(Some),
      token => Err(Error::Unexpected("ItemBegin", token)),
    }
  }
}

macro_rules! delegate {
  ($dser_method:ident) => (
    #[inline]
    fn $dser_method<V>(self, visitor: V) -> Result<V::Value>
      where V: Visitor<'de>,
    {
      (self.0).$dser_method(visitor)
    }
  );
  ($dser_method:ident, name) => (
    #[inline]
    fn $dser_method<V>(self, name: &'static str, visitor: V) -> Result<V::Value>
      where V: Visitor<'de>,
    {
      (self.0).$dser_method(name, visitor)
    }
  );
  ($dser_method:ident, names) => (
    #[inline]
    fn $dser_method<V>(self, name: &'static str, fields: &'static [&'static str], visitor: V) -> Result<V::Value>
      where V: Visitor<'de>,
    {
      (self.0).$dser_method(name, fields, visitor)
    }
  );
}
/// Десериализатор для чтения идентификаторов полей
struct Field<'a, R: 'a + Read + Seek>(&'a mut Deserializer<R>);

impl<'de, 'a, R: 'a + Read + Seek> de::Deserializer<'de> for Field<'a, R> {
  type Error = Error;

  #[inline]
  fn is_human_readable(&self) -> bool { false }

  fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.0.next_token()?;
    if let Token::Label(index) = token {
      let label = self.0.parser.read_label(index)?;
      return visitor.visit_str(label.as_str()?);
    }
    return Err(Error::Unexpected("Label", token));
  }

  delegate!(deserialize_i8);
  delegate!(deserialize_u8);
  delegate!(deserialize_i16);
  delegate!(deserialize_u16);
  delegate!(deserialize_i32);
  delegate!(deserialize_u32);
  delegate!(deserialize_i64);
  delegate!(deserialize_u64);
  delegate!(deserialize_f32);
  delegate!(deserialize_f64);

  delegate!(deserialize_bool);
  delegate!(deserialize_char);
  delegate!(deserialize_str);
  delegate!(deserialize_string);
  delegate!(deserialize_bytes);
  delegate!(deserialize_byte_buf);
  delegate!(deserialize_option);
  delegate!(deserialize_unit);
  delegate!(deserialize_map);
  delegate!(deserialize_seq);

  delegate!(deserialize_any);
  delegate!(deserialize_ignored_any);

  delegate!(deserialize_unit_struct, name);
  delegate!(deserialize_newtype_struct, name);
  delegate!(deserialize_struct, names);
  delegate!(deserialize_enum, names);

  #[inline]
  fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    self.0.deserialize_tuple(len, visitor)
  }
  #[inline]
  fn deserialize_tuple_struct<V>(self, name: &'static str, len: usize, visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    self.0.deserialize_tuple_struct(name, len, visitor)
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
