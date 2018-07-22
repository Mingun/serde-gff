//! Содержит реализацию структуры, описывающей сигнатуру GFF файла, реализацию типажей для
//! конвертации других типов данных в сигнатуру и обратно и известные форматы файлов

use std::io::{Read, Write, Result};

/// Определяет назначение файла. Сигнатура записана в первых 4-х байтах файла на диске
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signature {
  /// Информация о модуле
  IFO,

  /// Описание области
  ARE,
  /// Инстанции игровых объектов и динамические свойства области
  GIT,
  /// Комментарий к области
  GIC,

  /// Шаблон (blueprint) существа
  UTC,
  /// Шаблон (blueprint) двери
  UTD,
  /// Шаблон (blueprint) схватки (encounter)
  UTE,
  /// Шаблон (blueprint) предмета
  UTI,
  /// Шаблон (blueprint) размещаемого объекта окружения (placeable)
  UTP,
  /// Шаблон (blueprint) звука
  UTS,
  /// Шаблон (blueprint) магазина
  UTM,
  /// Шаблон (blueprint) триггера
  UTT,
  /// Шаблон (blueprint) навигационной точки (waypoint)
  UTW,

  /// Диалог
  DLG,
  /// Журнал заданий
  JRL,
  /// Описания фракций
  FAC,
  /// Палитра
  ITP,

  /// Файл мастера сценариев: plot instance/plot manager file
  PTM,
  /// Файл мастера сценариев: plot wizard blueprint
  PTT,

  /// Параметры существа или игрового персонажа, создаваемые игрой
  BIC,

  /// Прочие виды файлов
  Other([u8; 4]),
}

impl Signature {
  /// Читает из указанного потока 4 байта сигнатуры файла
  #[inline]
  pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
    let mut sig = [0u8; 4];
    reader.read_exact(&mut sig)?;
    Ok(sig.into())
  }
  /// Записывает 4 байта сигнатуры в поток
  #[inline]
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    writer.write_all(self.as_ref())
  }
}

impl From<[u8; 4]> for Signature {
  fn from(arr: [u8; 4]) -> Self {
    use self::Signature::*;

    match &arr {
      b"IFO " => IFO,

      b"ARE " => ARE,
      b"GIT " => GIT,
      b"GIC " => GIC,

      b"UTC " => UTC,
      b"UTD " => UTD,
      b"UTE " => UTE,
      b"UTI " => UTI,
      b"UTP " => UTP,
      b"UTS " => UTS,
      b"UTM " => UTM,
      b"UTT " => UTT,
      b"UTW " => UTW,

      b"DLG " => DLG,
      b"JRL " => JRL,
      b"FAC " => FAC,
      b"ITP " => ITP,

      b"PTM " => PTM,
      b"PTT " => PTT,

      b"BIC " => BIC,

      _ => Other(arr),
    }
  }
}

impl AsRef<[u8]> for Signature {
  fn as_ref(&self) -> &[u8] {
    use self::Signature::*;

    match *self {
      IFO => b"IFO ",

      ARE => b"ARE ",
      GIT => b"GIT ",
      GIC => b"GIC ",

      UTC => b"UTC ",
      UTD => b"UTD ",
      UTE => b"UTE ",
      UTI => b"UTI ",
      UTP => b"UTP ",
      UTS => b"UTS ",
      UTM => b"UTM ",
      UTT => b"UTT ",
      UTW => b"UTW ",

      DLG => b"DLG ",
      JRL => b"JRL ",
      FAC => b"FAC ",
      ITP => b"ITP ",

      PTM => b"PTM ",
      PTT => b"PTT ",

      BIC => b"BIC ",

      Other(ref sig) => sig,
    }
  }
}
