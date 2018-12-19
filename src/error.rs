//! Реализация структуры, описывающей ошибки кодирования или декодирования GFF

use std::borrow::Cow;
use std::fmt;
use std::error;
use std::io;
use std::result;

use self::Error::*;

/// Виды ошибок, который могут возникнуть при чтении и интерпретации GFF-файла
#[derive(Debug)]
pub enum Error {
  /// Произошла ошибка чтения или записи из/в нижележащего буфера
  Io(io::Error),
  /// Произошла ошибка кодирования или декодирования строки, например, из-за использования
  /// символа, не поддерживаемого кодировкой
  Encoding(Cow<'static, str>),
  /// В файле встретилось значение неизвестного типа, хранящее указанное значение
  UnknownValue {
    /// Идентификатор типа значения
    tag: u32,
    /// Значение, которое было записано в файле для данного тега
    value: u32
  },
  /// Разбор уже завершен
  ParsingFinished,
  /// Некорректное значение для метки. Метка не должна превышать по длине 16 байт в UTF-8,
  /// но указанное значение больше. Ошибка содержит длину текста, который пытаются преобразовать
  TooLongLabel(usize),
}
/// Тип результата, используемый в методах данной библиотеки
pub type Result<T> = result::Result<T, Error>;

impl fmt::Display for Error {
  fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      Io(ref err) => err.fmt(fmt),
      Encoding(ref msg) => msg.fmt(fmt),
      UnknownValue { tag, value } => write!(fmt, "Unknown field value (tag: {}, value: {})", tag, value),
      ParsingFinished => write!(fmt, "Parsing finished"),
      TooLongLabel(len) => write!(fmt, "Too long label: label can contain up to 16 bytes, but string contains {} bytes in UTF-8", len),
    }
  }
}

impl error::Error for Error {
  fn description(&self) -> &str {
    match *self {
      Io(ref err) => error::Error::description(err),
      Encoding(ref msg) => msg,
      UnknownValue { .. } => "Unknown field value",
      ParsingFinished => "Parsing finished",
      TooLongLabel(..) => "Too long label",
    }
  }

  fn cause(&self) -> Option<&error::Error> {
    match *self {
      Io(ref err) => Some(err),
      _ => None,
    }
  }
}

impl From<io::Error> for Error {
  fn from(value: io::Error) -> Self { Io(value) }
}
/// Реализация для конвертации из ошибок кодирования библиотеки `encodings`
impl From<Cow<'static, str>> for Error {
  fn from(value: Cow<'static, str>) -> Self { Encoding(value) }
}
