//! Десериализатор для формата Bioware GFF (Generic File Format)

use std::io::{Read, Seek};
use encoding::{DecoderTrap, EncodingRef};
use serde::de::{self, Visitor, DeserializeSeed};

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
impl<'de, 'a, R: Read + Seek> de::Deserializer<'de> for &'a mut Deserializer<R> {
  type Error = Error;

  #[inline]
  fn is_human_readable(&self) -> bool { false }

  unsupported!(deserialize_i8);
  unsupported!(deserialize_u8);
  unsupported!(deserialize_i16);
  unsupported!(deserialize_u16);
  unsupported!(deserialize_i32);
  unsupported!(deserialize_u32);
  unsupported!(deserialize_i64);
  unsupported!(deserialize_u64);
  unsupported!(deserialize_f32);
  unsupported!(deserialize_f64);

  unsupported!(deserialize_bool);
  unsupported!(deserialize_char);
  unsupported!(deserialize_str);
  unsupported!(deserialize_string);
  unsupported!(deserialize_bytes);
  unsupported!(deserialize_byte_buf);
  unsupported!(deserialize_option);
  unsupported!(deserialize_unit);
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

  fn deserialize_unit_struct<V>(self, name: &'static str, _visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    unimplemented!("`deserialize_unit_struct(name: {})` not yet supported. Token: {:?}", name, token)
  }
  fn deserialize_newtype_struct<V>(self, name: &'static str, _visitor: V) -> Result<V::Value>
    where V: Visitor<'de>,
  {
    let token = self.next_token()?;
    unimplemented!("`deserialize_newtype_struct(name: {})` not yet supported. Token: {:?}", name, token)
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
