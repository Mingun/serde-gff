//! Содержит описания структур заголовка GFF файла

use std::cmp::max;
use std::io::{Read, Write, Result};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};

pub use crate::sig::*;
pub use crate::ver::*;

/// Описание области файла, описывающей местоположение списков записей в файле
#[derive(Debug, Default)]
pub struct Section {
  /// Смещение в байтах от начала файла в сериализованном виде
  pub offset: u32,
  /// Количество записей по смещению `offset`. Размер записи зависит от конкретного поля
  pub count:  u32,
}

impl Section {
  /// Читает описание области из потока
  #[inline]
  pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
    Ok(Section {
      offset: reader.read_u32::<LE>()?,
      count:  reader.read_u32::<LE>()?,
    })
  }
  /// Записывает описание области файла в поток
  #[inline]
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    writer.write_u32::<LE>(self.offset)?;
    writer.write_u32::<LE>(self.count)
  }
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// Заголовок GFF файла. Заголовок содержит вид файла, версию формата и информацию о
/// 6 областях, файла, содержащих данные:
/// - Список структур в файле
/// - Общий список полей всех структур файла
/// - Список уникальных названий полей
/// - Список с данными полей
/// - Вспомогательный список для индексов для сложных структур данных
/// - Вспомогательный список для хранения списочных значений полей
#[derive(Debug)]
pub struct Header {
  /// Конкретный вид GFF файла
  pub signature: Signature,
  /// Версия файла
  pub version: Version,

  /// Содержит смещение в байтах от начала файла области с расположением
  /// структур и их количество
  pub structs: Section,

  /// Содержит смещение в байтах от начала файла области с расположением
  /// полей структур и их количество
  pub fields: Section,

  /// Содержит смещение в байтах от начала файла области с расположением
  /// меток полей в структурах и их количество
  pub labels: Section,

  /// Содержит смещение в байтах от начала файла области с расположением
  /// сериализованных значений полей и суммарное число байт данных
  pub field_data: Section,

  /// Содержит смещение в байтах от начала файла области с расположением
  /// индексов полей и их количество
  pub field_indices: Section,

  /// Содержит смещение в байтах от начала файла области с расположением
  /// индексов списков и их количество
  pub list_indices: Section,
}

impl Header {
  /// Создает заголовок для пустого файла с указанным типом
  #[inline]
  pub fn new(signature: Signature) -> Self {
    Self::with_version(signature, Version::V3_2)
  }
  /// Создает заголовок для пустого файла с указанным типом и версией
  #[inline]
  pub fn with_version(signature: Signature, version: Version) -> Self {
    Header {
      signature,
      version,
      structs:       Section::default(),
      fields:        Section::default(),
      labels:        Section::default(),
      field_data:    Section::default(),
      field_indices: Section::default(),
      list_indices:  Section::default(),
    }
  }
  /// Читает значение GFF заголовка из потока
  pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
    Ok(Header {
      signature:     Signature::read(reader)?,
      version:       Version::read(reader)?,

      structs:       Section::read(reader)?,
      fields:        Section::read(reader)?,
      labels:        Section::read(reader)?,
      field_data:    Section::read(reader)?,
      field_indices: Section::read(reader)?,
      list_indices:  Section::read(reader)?,
    })
  }
  /// Записывает значение GFF заголовка в поток
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    self.signature.write(writer)?;
    self.version.write(writer)?;

    self.structs.write(writer)?;
    self.fields.write(writer)?;
    self.labels.write(writer)?;
    self.field_data.write(writer)?;
    self.field_indices.write(writer)?;
    self.list_indices.write(writer)
  }
  /// Возвращает нижнюю границу на количество токенов, которые может произвести
  /// данный файл
  #[inline]
  pub fn token_count(&self) -> usize {
    // Для каждой структуры - токен начала и окончания
    // Для каждого списка - токен начала и окончания
    let size = (self.structs.count + self.list_indices.count)*2;

    // Т.к. каждое поле может быть списком или структурой, то они уже подсчитываются
    // в списках и структурах. Поэтому минимальное количество вычисляем, как максимум
    // из того, что нам смогут дать поля или структуры со списками
    max(size, self.fields.count) as usize
  }
}
