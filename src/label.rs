//! Содержит реализацию структуры, описывающей название поля в GFF файле и реализацию типажей для
//! конвертации других типов данных в метку и обратно

use std::fmt;
use std::result::Result;
use std::str::{from_utf8, Utf8Error};

/// Описание названия поля структуры GFF файла. GFF файл состоит из дерева структур, а каждая
/// структура -- из полей с именем и значением. Имена полей представлены данной структурой
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label([u8; 16]);

impl Label {
  /// Возвращает представление данной метки как текста, если он представлен в виде `UTF-8` строки
  pub fn as_str(&self) -> Result<&str, Utf8Error> {
    for i in 0..self.0.len() {
      // Во внутреннем представлении данные метки продолжаются до первого нулевого символа,
      // однако сам нулевой символ не храниться -- это просто заполнитель
      if self.0[i] == 0 {
        return from_utf8(&self.0[0..i])
      }
    }
    return from_utf8(&self.0);
  }
}

impl fmt::Debug for Label {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    if let Ok(value) = self.as_str() {
      return write!(f, "Label({})", value);
    }
    write!(f, "Label(")?;
    self.0.fmt(f)?;
    return write!(f, ")");
  }
}

impl fmt::Display for Label {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    let value = self.as_str().map_err(|_| fmt::Error)?;
    write!(f, "{}", value)
  }
}

impl From<[u8; 16]> for Label {
  fn from(arr: [u8; 16]) -> Self { Label(arr) }
}

impl AsRef<[u8]> for Label {
  fn as_ref(&self) -> &[u8] { &self.0 }
}
