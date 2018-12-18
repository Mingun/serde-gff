//! Содержит реализацию структуры, описывающей ссылку на ресурс и реализацию типажей для
//! конвертации других типов данных в ссылку и обратно

use std::fmt;
use std::str::{self, FromStr, Utf8Error};
use std::string::FromUtf8Error;

/// Представляет ссылку на игровой ресурс, которым может быть шаблон объекта
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResRef(pub(crate) Vec<u8>);

impl ResRef {
  /// Возвращает представление данной ссылки на ресурс как строки, если она представлена в виде `UTF-8` строки
  #[inline]
  pub fn as_str(&self) -> Result<&str, Utf8Error> {
    str::from_utf8(&self.0)
  }
  /// Возвращает представление данной ссылки на ресурс как строки, если она представлена в виде `UTF-8` строки
  #[inline]
  pub fn as_string(self) -> Result<String, FromUtf8Error> {
    String::from_utf8(self.0)
  }
}

impl fmt::Debug for ResRef {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    if let Ok(value) = str::from_utf8(&self.0) {
      return write!(f, "{}", value);
    }
    self.0.fmt(f)
  }
}

impl fmt::Display for ResRef {
  #[inline]
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    let value = self.as_str().map_err(|_| fmt::Error)?;
    write!(f, "{}", value)
  }
}

impl Into<String> for ResRef {
  #[inline]
  fn into(self) -> String {
    String::from_utf8(self.0).expect("ResRef contains non UTF-8 string")
  }
}

impl<'a> Into<&'a str> for &'a ResRef {
  #[inline]
  fn into(self) -> &'a str {
    str::from_utf8(&self.0).expect("ResRef contains non UTF-8 string")
  }
}

impl<'a> From<&'a str> for ResRef {
  #[inline]
  fn from(str: &'a str) -> Self { ResRef(str.as_bytes().to_owned()) }
}

impl FromStr for ResRef {
  type Err = ();

  #[inline]
  fn from_str(str: &str) -> Result<Self, Self::Err> { Ok(str.into()) }
}
