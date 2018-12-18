//! Содержит описание структур-индексов различных данных в GFF файле

use std::ops::Add;

use header::Header;

/// Типаж, реализуемый специальными структурами, хранящими индексы на записи в GFF-файле,
/// позволяющий преобразовать их в реальное смещение для чтения информации из файла.
pub trait Index {
  /// Получает смещение от начала GFF-файла, в котором находятся индексируемые данные
  fn offset(&self, header: &Header) -> u64;
}

/// Макрос для объявления типизированной обертки над числом (или числами),
/// представляющем(ими) индекс одной из структур данных в файле.
///
/// # Параметры 1
/// - `$name`: Имя генерируемой структуры.
/// - `$field`: Имя поля в заголовке, хранящее базовое смещение для структур,
///   к которым производится доступ по данному индексу
///
/// # Параметры 2
/// - `$name`: Имя генерируемой структуры. Структура реализует типаж `From` для
///   конструирования из `u32`
/// - `$field`: Имя поля в заголовке, хранящее базовое смещение для структур,
///   к которым производится доступ по данному индексу
/// - `$multiplier`: множитель для индекса, переводящий его в смещение в байтах
macro_rules! index {
  ($(#[$attrs:meta])* $name:ident, $field:ident) => (
    $(#[$attrs])*
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct $name(pub(crate) u32, pub(crate) u32);

    impl Index for $name {
      #[inline]
      fn offset(&self, header: &Header) -> u64 {
        let start  = header.$field.offset as u64;
        let offset = self.0 as u64 + self.1 as u64 * 4;

        start + offset
      }
    }
    impl Add<u32> for $name {
      type Output = Self;

      fn add(self, rhs: u32) -> Self {
        $name(self.0, self.1 + rhs)
      }
    }
  );

  ($(#[$attrs:meta])* $name:ident, $field:ident, $multiplier:expr) => (
    $(#[$attrs])*
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct $name(pub(crate) u32);
    impl Index for $name {
      #[inline]
      fn offset(&self, header: &Header) -> u64 {
        let start  = header.$field.offset as u64;
        let offset = self.0 as u64 * $multiplier;

        start + offset
      }
    }
    impl From<u32> for $name {
      fn from(value: u32) -> Self { $name(value) }
    }
  );
}

index!(
  /// Номер структуры в файле
  StructIndex, structs, 3*4
);
index!(
  /// Номер поля в массиве полей GFF файла. Каждая структура в файле состоит из набора полей,
  /// на которые ссылается по этим индексам.
  FieldIndex, fields, 3*4
);
index!(
  /// Номер метки для поля в общем массиве меток, хранящихся в GFF файле
  LabelIndex, labels, 16
);
index!(
  /// Двойной индекс -- номера списка с полями структуры структуры и номер поля в этом списке.
  /// Используется для указания на поля структуры, когда структура содержит несколько полей.
  FieldIndicesIndex, field_indices
);
index!(
  /// Двойной индекс -- номера списка элементов и номер элемента в этом списке.
  /// Используется для представления списков.
  ListIndicesIndex, list_indices
);

index!(
  /// Смещение в файле, по которому расположены данные поля типа `Dword64`
  U64Index, field_data, 1
);
index!(
  /// Смещение в файле, по которому расположены данные поля типа `Int64`
  I64Index, field_data, 1
);
index!(
  /// Смещение в файле, по которому расположены данные поля типа `Double64`
  F64Index, field_data, 1
);
index!(
  /// Смещение в файле, по которому расположены данные поля типа `String`
  StringIndex, field_data, 1
);
index!(
  /// Смещение в файле, по которому расположены данные поля типа `ResRef`
  ResRefIndex, field_data, 1
);
index!(
  /// Смещение в файле, по которому расположены данные поля типа `LocString`
  LocStringIndex, field_data, 1
);
index!(
  /// Смещение в файле, по которому расположены данные поля типа `Void`
  BinaryIndex, field_data, 1
);
