//! Содержит реализацию структуры, описывающей версию GFF файла, реализацию типажей для
//! конвертации других типов данных в версию и обратно и известные версии файлов

use std::fmt::{self, Display, Formatter};
use std::io::{Read, Write, Result};

/// Версия формата файла. Записана во вторых 4-х байтах файла, сразу после сигнатуры
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Version([u8; 4]);

impl Version {
  /// Версия GFF формата, являющаяся текущей. Заголовки, создаваемые без указания версии,
  /// имеют данную версию в качестве умолчания.
  //TODO: После решения https://github.com/rust-lang/rust/issues/24111 можно сделать функции
  // константными и использовать метод `new`.
  pub const V3_2: Version = Version(*b"V3.2");

  /// Создает новый объект версии из старшей и младшей половины версии
  #[inline]
  pub fn new(major: u8, minor: u8) -> Self {
    Version([b'V', major + b'0', b'.', minor + b'0'])
  }
  /// Старший номер версии формата файла, хранимый в байте 1 версии
  #[inline]
  pub fn major(&self) -> u8 { self.0[1] - b'0' }
  /// Младший номер версии формата файла, хранимый в байте 3 версии
  #[inline]
  pub fn minor(&self) -> u8 { self.0[3] - b'0' }

  /// Читает версию файла из потока
  #[inline]
  pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
    let mut version = Version([0u8; 4]);
    reader.read(&mut version.0)?;
    Ok(version)
  }
  /// Записывает версию файла в поток
  #[inline]
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    writer.write_all(&self.0)
  }
}

impl Display for Version {
  /// Выводит версию в поток в формате `<major>.<minor>`
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    write!(f, "{}.{}", self.major(), self.minor())
  }
}
