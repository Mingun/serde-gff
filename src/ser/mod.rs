//! Сериализатор для формата Bioware GFF (Generic File Format)

use std::io::Write;
use byteorder::{LE, WriteBytesExt};
use indexmap::IndexSet;
use serde::ser::{self, Impossible, Serialize};

use Label;
use error::{Error, Result};
use header::{Header, Section, Signature, Version};
use index::LabelIndex;
use value::SimpleValueRef;
use raw::{self, FieldType};

/// Вспомогательная структура, описывающая индекс структуры, для типобезопасности
#[derive(Debug, Copy, Clone)]
struct StructIndex(usize);

/// Вспомогательная структура, описывающая индекс списка полей структуры, для типобезопасности.
/// Любая GFF структура, имеющая более двух полей, ссылается по такому индексу на список с
/// перечислением имеющихся у нее полей
#[derive(Debug, Copy, Clone)]
struct FieldListIndex(usize);

/// Вспомогательная структура, описывающая индекс списка элементов GFF списка, для типобезопасности
#[derive(Debug, Copy, Clone)]
struct ListIndex(usize);

/// Промежуточное представление сериализуемых структур. Содержит данные, которые после
/// небольшого преобразования, возможного только после окончания сериализации, могут
/// быть записаны в файл
#[derive(Debug)]
enum Struct {
  /// Структура без полей
  NoFields,
  /// Структура, состоящая только из одного поля, содержит индекс этого поля
  OneField(usize),
  /// Структура, состоящая из двух и более полей. Содержит индекс списка и количество полей
  MultiField { list: FieldListIndex, fields: u32 }
}
impl Struct {
  /// Преобразует промежуточное представление в окончательное, которое может быть записано в файл
  #[inline]
  fn into_raw(&self, offsets: &[u32]) -> raw::Struct {
    use self::Struct::*;

    match *self {
      NoFields                    => raw::Struct { tag: 0, offset: 0,               fields: 0 },
      OneField(index)             => raw::Struct { tag: 0, offset: index as u32,    fields: 1 },
      MultiField { list, fields } => raw::Struct { tag: 0, offset: offsets[list.0], fields },
    }
  }
}

/// Промежуточное представление сериализуемого поля структуры. Содержит данные, которые после
/// небольшого преобразования, возможного только после окончания сериализации, могут
/// быть записаны в файл
#[derive(Debug)]
enum Field {
  /// Поле, представленное значением без внутренней структуры. Содержит метку поля и его значение
  Simple { label: LabelIndex, value: SimpleValueRef },
  /// Поле, представленное значением с внутренней структурой. Содержит метку поля и индекс
  /// промежуточного представления структуры в массиве [`structs`](struct.Serializer.html#field.structs)
  Struct { label: LabelIndex, struct_: StructIndex },
  /// Поле, представленное списком значений. Содержит метку поля и индекс списка в массиве
  /// [`list_indices`](struct.Serializer.html#field.list_indices)
  List   { label: LabelIndex, list: ListIndex },
}
impl Field {
  /// Преобразует промежуточное представление в окончательное, которое может быть записано в файл
  #[inline]
  fn into_raw(&self, offsets: &[u32]) -> Result<raw::Field> {
    use self::Field::*;

    Ok(match self {
      Simple { label, value } => value.into_raw(label.0)?,
      Struct { label, struct_ } => {
        let mut data = [0u8; 4];
        (&mut data[..]).write_u32::<LE>(struct_.0 as u32)?;
        raw::Field { tag: FieldType::Struct as u32, label: label.0, data }
      },
      List { label, list } => {
        let offset = offsets[list.0];

        let mut data = [0u8; 4];
        (&mut data[..]).write_u32::<LE>(offset)?;
        raw::Field { tag: FieldType::List as u32, label: label.0, data }
      },
    })
  }
}

impl SimpleValueRef {
  /// Конвертирует возможно ссылочное значение в структуру, которая может быть записана в файл
  ///
  /// # Параметры
  /// - `label`: индекс метки для поля
  #[inline]
  fn into_raw(&self, label: u32) -> Result<raw::Field> {
    use self::SimpleValueRef::*;

    let mut data = [0u8; 4];
    let type_ = {
      let mut storage = &mut data[..];
      match *self {
        Byte(val)      => { storage.write_u8       (val  )?; FieldType::Byte      },
        Char(val)      => { storage.write_i8       (val  )?; FieldType::Char      },
        Word(val)      => { storage.write_u16::<LE>(val  )?; FieldType::Word      },
        Short(val)     => { storage.write_i16::<LE>(val  )?; FieldType::Short     },
        Dword(val)     => { storage.write_u32::<LE>(val  )?; FieldType::Dword     },
        Int(val)       => { storage.write_i32::<LE>(val  )?; FieldType::Int       },
        Dword64(val)   => { storage.write_u32::<LE>(val.0)?; FieldType::Dword64   },
        Int64(val)     => { storage.write_u32::<LE>(val.0)?; FieldType::Int64     },
        Float(val)     => { storage.write_f32::<LE>(val  )?; FieldType::Float     },
        Double(val)    => { storage.write_u32::<LE>(val.0)?; FieldType::Double    },
        String(val)    => { storage.write_u32::<LE>(val.0)?; FieldType::String    },
        ResRef(val)    => { storage.write_u32::<LE>(val.0)?; FieldType::ResRef    },
        LocString(val) => { storage.write_u32::<LE>(val.0)?; FieldType::LocString },
        Void(val)      => { storage.write_u32::<LE>(val.0)?; FieldType::Void      },
      }
    };
    Ok(raw::Field { tag: type_ as u32, label, data })
  }
}

/// Структура для сериализации значения Rust в Bioware GFF.
///
/// Формат поддерживает непосредственную сериализацию только структур, перечислений и отображений.
/// Остальные значения необходимо обернуть в одну из этих структур данных для возможности их
/// сериализации.
#[derive(Default, Debug)]
pub struct Serializer {
  /// Массив, содержащий описания структур в файле
  structs: Vec<Struct>,
  /// Массив, содержащий описания полей структур в файле
  fields: Vec<Field>,
  /// Множество, содержащие названия всех полей всех структур файла в порядке их добавления
  labels: IndexSet<Label>,
  /// Массив, содержащий данные комплексных полей
  field_data: Vec<u8>,
  /// Массив списков с индексами полей структур. Каждый элемент массива описывает набор
  /// полей одной структуры, которая содержит более одного поля
  field_indices: Vec<Vec<u32>>,
  /// Массив списков с индексами структур, содержащихся в каждом списке. Каждый элемент
  /// массива описывает набор структур, содержащихся в списке. Общее количество полей-списков
  /// равно размеру массива.
  list_indices: Vec<Vec<u32>>,
}

impl Serializer {
  /// Добавляет в список известных названий полей для сериализации указанное и возвращает
  /// его индекс в этом списке. Если такое поле уже имеется в индексе, не добавляет его
  /// повторно.
  ///
  /// В случае, если метка содержит более 16 байт в UTF-8 представлении, метод завершается
  /// с ошибкой.
  fn add_label(&mut self, label: &str) -> Result<LabelIndex> {
    let label = label.parse()?;
    self.labels.insert(label);
    // Мы только что вставили значение, ошибка может быть только в случае переполнения, что вряд ли случится
    let (index, _) = self.labels.get_full(&label).unwrap();
    Ok(LabelIndex(index as u32))
  }
  /// Добавляет в список структур новую структуру с указанным количеством полей.
  /// Корректная ссылка на данные еще не заполнена, ее нужно будет скорректировать
  /// после того, как содержимое структуры будет записано
  ///
  /// # Параметры
  /// - `fields`: Количество полей в структуре
  ///
  /// Возвращает пару индексов: добавленной структуры и списка с полями структуры,
  /// если полей несколько
  fn add_struct(&mut self, fields: usize) -> (StructIndex, FieldListIndex) {
    let index = StructIndex(self.structs.len());
    let list  = FieldListIndex(self.field_indices.len());

    match fields {
      0 => self.structs.push(Struct::NoFields),
      // Для структуры с одним полем записываем placeholder, он будет перезаписан после записи поля
      1 => self.structs.push(Struct::OneField(0)),
      _ => {
        self.field_indices.push(Vec::with_capacity(fields));
        self.structs.push(Struct::MultiField { list, fields: fields as u32 })
      }
    }
    (index, list)
  }
  /// Добавляет в список списков индексов с элементами новый элемент на указанное
  /// количество элементов и заполняет тип поля.
  ///
  /// # Параметры
  /// - `field_index`: индекс поля, которому нужно обновить тип
  /// - `len`: длина списка элементов, хранящемся в этом поле
  ///
  /// Возвращает индекс списка, в который нужно добавлять элементы в процессе их сериализации
  fn add_list(&mut self, label: LabelIndex, len: usize) -> ListIndex {
    let list = ListIndex(self.list_indices.len());
    self.list_indices.push(Vec::with_capacity(len));
    self.fields.push(Field::List { label, list });
    list
  }
  /// Создает заголовок файла на основе его содержания
  fn make_header(&self, signature: Signature, version: Version) -> Header {
    struct Builder {
      offset: u32,
    }
    impl Builder {
      #[inline]
      fn new() -> Self {
        // Версия, сигнатура и 6 секций
        Builder { offset: 4 + 4 + 8 * 6 }
      }
      #[inline]
      fn add_section(&mut self, count: usize, size: u32) -> Section {
        let section = Section { offset: self.offset, count: count as u32 };
        self.offset += section.count * size;
        section
      }
      /// Создает секцию, подсчитывая количество байт во всех списках массива `vec`
      #[inline]
      fn fields(&mut self, vec: &Vec<Vec<u32>>) -> Section {
        let cnt = vec.into_iter().fold(0, |sum, v| sum + v.len());
        self.add_section(cnt * 4, 1)// Количество в данной секции задается в байтах, а не элементах
      }
      #[inline]
      fn lists(&mut self, vec: &Vec<Vec<u32>>) -> Section {
        let cnt = vec.into_iter().fold(0, |sum, v| sum + v.len() + 1);
        self.add_section(cnt * 4, 1)// Количество в данной секции задается в байтах, а не элементах
      }
    }

    let mut builder = Builder::new();
    Header {
      signature:     signature,
      version:       version,
      structs:       builder.add_section(self.structs.len(), 3 * 4),// 3 * u32
      fields:        builder.add_section(self.fields.len(),  3 * 4),// 3 * u32
      labels:        builder.add_section(self.labels.len(), 16 * 1),// 16 * u8
      field_data:    builder.add_section(self.field_data.len(), 1), // 1 * u8
      field_indices: builder.fields(&self.field_indices),
      list_indices:  builder.lists(&self.list_indices),
    }
  }
  /// Записывает в поток все собранные данные
  pub fn write<W: Write>(&self, writer: &mut W, signature: Signature, version: Version) -> Result<()> {
    self.make_header(signature, version).write(writer)?;

    self.write_structs(writer)?;
    self.write_fields(writer)?;
    self.write_labels(writer)?;
    writer.write_all(&self.field_data)?;
    for ref list in &self.field_indices {
      self.write_indices(writer, list)?;
    }
    for ref list in &self.list_indices {
      writer.write_u32::<LE>(list.len() as u32)?;
      self.write_indices(writer, list)?;
    }
    Ok(())
  }
  /// Записывает в поток информацию о структурах файла
  #[inline]
  fn write_structs<W: Write>(&self, writer: &mut W) -> Result<()> {
    let offsets = self.calc_field_offsets();
    for e in self.structs.iter() {
      e.into_raw(&offsets).write(writer)?;
    }
    Ok(())
  }
  /// Записывает в поток информацию о полях файла
  #[inline]
  fn write_fields<W: Write>(&self, writer: &mut W) -> Result<()> {
    let offsets = self.calc_list_offsets();
    for e in self.fields.iter() {
      e.into_raw(&offsets)?.write(writer)?;
    }
    Ok(())
  }
  /// Записывает в поток информацию о метках файла
  #[inline]
  fn write_labels<W: Write>(&self, writer: &mut W) -> Result<()> {
    for label in self.labels.iter() {
      writer.write_all(label.as_ref())?;
    }
    Ok(())
  }
  /// Записывает в поток информацию об индексах файла
  #[inline]
  fn write_indices<W: Write>(&self, writer: &mut W, indices: &Vec<u32>) -> Result<()> {
    for index in indices.iter() {
      writer.write_u32::<LE>(*index)?;
    }
    Ok(())
  }
  /// Вычисляет смещения, на которые нужно заменить индексы в структурах для ссылки на списки их полей
  fn calc_field_offsets(&self) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(self.field_indices.len());
    let mut last_offset = 0;
    for elements in self.field_indices.iter() {
      offsets.push(last_offset as u32);
      last_offset += elements.len() * 4;
    }
    offsets
  }
  /// Вычисляет смещения, на которые нужно заменить индексы, хранимые в поле с типом List
  fn calc_list_offsets(&self) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(self.list_indices.len());
    let mut last_offset = 0;
    for elements in self.list_indices.iter() {
      offsets.push(last_offset as u32);
      // +1 для длины списка
      last_offset += (elements.len() + 1) * 4;
    }
    offsets
  }
}

/// Сериализует значение в произвольный поток. Значение должно являться Rust структурой или перечислением
#[inline]
pub fn to_writer<W, T>(writer: &mut W, signature: Signature, value: &T) -> Result<()>
  where W: Write,
        T: Serialize + ?Sized,
{
  let mut s = Serializer::default();
  value.serialize(&mut s)?;
  s.write(writer, signature, Version::V3_2)
}
/// Сериализует значение в массив. Значение должно являться Rust структурой или перечислением
#[inline]
pub fn to_vec<T>(signature: Signature, value: &T) -> Result<Vec<u8>>
  where T: Serialize + ?Sized,
{
  let mut vec = Vec::new();
  to_writer(&mut vec, signature, value)?;
  Ok(vec)
}

/// Реализует метод, возвращающий ошибку при попытке сериализовать значение, с описанием
/// причины, что GFF не поддерживает данный тип на верхнем уровне и требуется обернуть его
/// в структуру
macro_rules! unsupported {
  ($ser_method:ident ( $($type:ty),* ) ) => (
    fn $ser_method(self, $(_: $type),*) -> Result<Self::Ok> {
      Err(Error::Serialize(concat!(
        "`", stringify!($ser_method), "` can't be implemented in GFF format. Wrap value to the struct and serialize struct"
      ).into()))
    }
  );
}

impl<'a> ser::Serializer for &'a mut Serializer {
  type Ok = ();
  type Error = Error;

  type SerializeSeq = Impossible<Self::Ok, Self::Error>;
  type SerializeTuple = Impossible<Self::Ok, Self::Error>;
  type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
  type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
  type SerializeMap = Impossible<Self::Ok, Self::Error>;
  type SerializeStruct = Impossible<Self::Ok, Self::Error>;
  type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

  unsupported!(serialize_i8(i8));
  unsupported!(serialize_u8(u8));
  unsupported!(serialize_i16(i16));
  unsupported!(serialize_u16(u16));
  unsupported!(serialize_i32(i32));
  unsupported!(serialize_u32(u32));
  unsupported!(serialize_i64(i64));
  unsupported!(serialize_u64(u64));

  unsupported!(serialize_f32(f32));
  unsupported!(serialize_f64(f64));

  unsupported!(serialize_bool(bool));
  unsupported!(serialize_char(char));

  unsupported!(serialize_str(&str));
  unsupported!(serialize_bytes(&[u8]));

  fn serialize_none(self) -> Result<Self::Ok> {
    unimplemented!("`serialize_none()`");
  }
  fn serialize_some<T>(self, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    unimplemented!("`serialize_some()`");
  }
  //-----------------------------------------------------------------------------------------------
  // Сериализация структурных элементов
  //-----------------------------------------------------------------------------------------------
  fn serialize_unit(self) -> Result<Self::Ok> {
    unimplemented!("`serialize_unit()`");
  }
  fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok> {
    unimplemented!("`serialize_unit_struct(name: {})`", name);
  }
  fn serialize_newtype_struct<T>(self, name: &'static str, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    unimplemented!("`serialize_newtype_struct(name: {})`", name);
  }
  fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
    unimplemented!("`serialize_tuple(len: {})`", len);
  }
  fn serialize_tuple_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeTupleStruct> {
    unimplemented!("`serialize_tuple_struct(name: {}, len: {})`", name, len);
  }
  fn serialize_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeStruct> {
    unimplemented!("`serialize_struct(name: {}, len: {})`", name, len);
  }
  //-----------------------------------------------------------------------------------------------
  // Сериализация последовательностей и отображений
  //-----------------------------------------------------------------------------------------------
  fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq> {
    unimplemented!("`serialize_seq(len: {:?})`", len);
  }
  fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap> {
    unimplemented!("`serialize_map(len: {:?})`", len);
  }
  //-----------------------------------------------------------------------------------------------
  // Сериализация компонентов перечисления
  //-----------------------------------------------------------------------------------------------
  fn serialize_unit_variant(self, name: &'static str, index: u32, variant: &'static str) -> Result<Self::Ok> {
    unimplemented!("`serialize_unit_variant(name: {}, index: {}, variant: {})`", name, index, variant);
  }
  fn serialize_newtype_variant<T>(self, name: &'static str, index: u32, variant: &'static str, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    unimplemented!("`serialize_newtype_variant(name: {}, index: {}, variant: {})`", name, index, variant);
  }
  fn serialize_tuple_variant(self, name: &'static str, index: u32, variant: &'static str, len: usize) -> Result<Self::SerializeTupleVariant> {
    unimplemented!("`serialize_tuple_variant(name: {}, index: {}, variant: {}, len: {})`", name, index, variant, len);
  }
  fn serialize_struct_variant(self, name: &'static str, index: u32, variant: &'static str, len: usize) -> Result<Self::SerializeStructVariant> {
    unimplemented!("`serialize_struct_variant(name: {}, index: {}, variant: {}, len: {})`", name, index, variant, len);
  }
}

#[cfg(test)]
mod tests {
  extern crate serde_bytes;

  use serde::ser::Serialize;
  use super::to_vec as to_vec_;
  use self::serde_bytes::{Bytes, ByteBuf};

  /// Формирует байтовый массив, соответствующий сериализованной структуре с одним полем
  /// `value` заданного типа, который хранится в записи о самом поле.
  macro_rules! primitive_wrapped {
    ($type:expr; $b1:expr, $b2:expr, $b3:expr, $b4:expr) => (
      vec![
        // Заголовок
        b'G',b'F',b'F',b' ',// Тип файла
        b'V',b'3',b'.',b'2',// Версия
        56,0,0,0,   1,0,0,0,// Начальное смещение и количество структур
        68,0,0,0,   1,0,0,0,// Начальное смещение и количество полей (1 поле)
        80,0,0,0,   1,0,0,0,// Начальное смещение и количество меток (1 метка)
        96,0,0,0,   0,0,0,0,// Начальное смещение и количество байт данных (данных нет)
        96,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках полей (списков нет)
        96,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках структур (списков нет)

        // Структуры
        // тег     ссылка      кол-во
        //        на данные    полей
        0,0,0,0,   0,0,0,0,   1,0,0,0,// Структура 0 (тег 0, 1 поле)

        // Поля
        // тип          метка     значение
        $type,0,0,0,   0,0,0,0,   $b1,$b2,$b3,$b4,// Поле 1 (ссылается на метку 1)

        // Метки
        b'v',b'a',b'l',b'u',b'e',0,0,0,0,0,0,0,0,0,0,0,// Метка 1
      ]
    );
  }

  /// Заменяет дерево токенов указанный выражением
  macro_rules! replace_expr {
    ($_t:tt $sub:expr) => ($sub);
  }

  /// Подсчитывает количество деревьев токенов, переданных в макрос
  macro_rules! len {
    ($($tts:tt)*) => (
      <[()]>::len(&[$(replace_expr!($tts ())),*])
    );
  }
  /// Формирует байтовый массив, соответствующий сериализованной структуре с одним полем
  /// `value` заданного типа, который хранится в записи о самом поле.
  macro_rules! complex_wrapped {
    ($type:expr; $($bytes:expr),*) => ({
      let count = len!($($bytes)*) as u8;

      vec![
        // Заголовок
        b'G',b'F',b'F',b' ',// Тип файла
        b'V',b'3',b'.',b'2',// Версия
        56,0,0,0,   1,0,0,0,// Начальное смещение и количество структур
        68,0,0,0,   1,0,0,0,// Начальное смещение и количество полей (1 поле)
        80,0,0,0,   1,0,0,0,// Начальное смещение и количество меток (1 метка)
        96,0,0,0,   count,0,0,0,// Начальное смещение и количество байт данных ($count байт данных)
        96 + count,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках полей (списков нет)
        96 + count,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках структур (списков нет)

        // Структуры
        // тег     ссылка      кол-во
        //        на данные    полей
        0,0,0,0,   0,0,0,0,   1,0,0,0,// Структура 0 (тег 0, 1 поле)

        // Поля
        // тип          метка     значение
        $type,0,0,0,   0,0,0,0,   0,0,0,0,// Поле 1 (ссылается на метку 1, и на данные с начала массива с данными)

        // Метки
        b'v',b'a',b'l',b'u',b'e',0,0,0,0,0,0,0,0,0,0,0,// Метка 1

        // Данные
        $($bytes),*
      ]
    });
  }

  mod toplevel {
    //! Тестирует сериализацию различных значений, когда они не включены ни в какую структуру
    use super::*;
    use error::Result;

    #[inline]
    fn to_result<T>(value: T) -> Result<Vec<u8>>
      where T: Serialize,
    {
      to_vec_((*b"GFF ").into(), &value)
    }

    #[inline]
    fn is_err<T>(value: T) -> bool
      where T: Serialize,
    {
      to_result(value).is_err()
    }

    #[inline]
    fn to_vec<T>(value: T) -> Vec<u8>
      where T: Serialize,
    {
      to_result(value).expect("Serialization fail")
    }

    /// Формирует байтовый массив, соответствующий сериализованной структуре без полей
    macro_rules! unit {
      () => (
        vec![
          // Заголовок
          b'G',b'F',b'F',b' ',// Тип файла
          b'V',b'3',b'.',b'2',// Версия
          56,0,0,0,   1,0,0,0,// Начальное смещение и количество структур
          68,0,0,0,   0,0,0,0,// Начальное смещение и количество полей (полей нет)
          68,0,0,0,   0,0,0,0,// Начальное смещение и количество меток (меток нет)
          68,0,0,0,   0,0,0,0,// Начальное смещение и количество байт данных (данных нет)
          68,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках полей (списков нет)
          68,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках структур (списков нет)

          // Структуры
          // тег      ссылка      кол-во
          //         на данные    полей
          0,0,0,0,   0,0,0,0,   0,0,0,0,// Структура 0 (0 полей)
        ]
      );
    }

    /// Тестирует запись простых числовых значений, которые хранятся в файле в разделе полей рядом с типом поля
    #[test]
    fn test_simple_numbers() {
      assert!(is_err( 42u8 ));
      assert!(is_err( 42u16));
      assert!(is_err( 42u32));

      assert!(is_err(-42i8 ));
      assert!(is_err(-42i16));
      assert!(is_err(-42i32));

      assert!(is_err( 4.2f32));
      assert!(is_err( 0.0f32));
      assert!(is_err(-4.2f32));
    }

    /// Тестирует запись комплексных числовых значений, которых хранятся в файле отдельно от описания самого поля
    #[test]
    fn test_complex_numbers() {
      assert!(is_err( 42u64));
      assert!(is_err(-42i64));

      assert!(is_err( 4.2f64));
      assert!(is_err( 0.0f64));
      assert!(is_err(-4.2f64));
    }

    /// Тестирует запись булевых значений, которые не поддерживаются форматом нативно
    #[test]
    #[should_panic(expected = "`serialize_bool` can\\'t be implemented in GFF format. Wrap value to the struct and serialize struct")]
    fn test_bool_true() {
      to_result(true).unwrap();
    }
    /// Тестирует запись булевых значений, которые не поддерживаются форматом нативно
    #[test]
    #[should_panic(expected = "`serialize_bool` can\\'t be implemented in GFF format. Wrap value to the struct and serialize struct")]
    fn test_bool_false() {
      to_result(false).unwrap();
    }

    /// Тестирует запись строковых срезов
    #[test]
    #[should_panic(expected = "`serialize_str` can\\'t be implemented in GFF format. Wrap value to the struct and serialize struct")]
    fn test_str_slice() {
      to_result("юникод").unwrap();
    }
    /// Тестирует запись строк
    #[test]
    #[should_panic(expected = "`serialize_str` can\\'t be implemented in GFF format. Wrap value to the struct and serialize struct")]
    fn test_str_owned() {
      to_result("юникод".to_owned()).unwrap();
    }

    /// Тестирует запись байтовых срезов
    #[test]
    #[should_panic(expected = "`serialize_bytes` can\\'t be implemented in GFF format. Wrap value to the struct and serialize struct")]
    fn test_bytes_slice() {
      let array = b"Array with length more then 32 bytes";

      to_result(Bytes::new(array)).unwrap();
    }
    /// Тестирует запись байтовых массивов
    #[test]
    #[should_panic(expected = "`serialize_bytes` can\\'t be implemented in GFF format. Wrap value to the struct and serialize struct")]
    fn test_bytes_owned() {
      let array = b"Array with length more then 32 bytes";

      to_result(ByteBuf::from(array.as_ref())).unwrap();
    }

    /// Тестирует запись отсутствующего опционального значения
    #[test]
    fn test_none() {
      macro_rules! none_test {
        ($type:ty) => (
          let none: Option<$type> = None;
          assert_eq!(to_vec(none), unit!());
        );
      }
      none_test!(u8);
      none_test!(u16);
      none_test!(u32);
      none_test!(u64);

      none_test!(i8);
      none_test!(i16);
      none_test!(i32);
      none_test!(i64);

      none_test!(f32);
      none_test!(f64);

      none_test!(String);
      none_test!(Vec<u8>);

      none_test!(());
      none_test!((u32, f32));

      #[derive(Serialize)]
      struct Unit;
      none_test!(Unit);

      #[derive(Serialize)]
      struct Newtype(u32);
      none_test!(Newtype);

      #[derive(Serialize)]
      struct Tuple(u32, f32);
      none_test!(Tuple);

      #[derive(Serialize)]
      struct Struct { field1: u32, field2: f32 };
      none_test!(Struct);
    }

    /// Тестирует запись опционального значения
    #[test]
    fn test_some() {
      assert!(is_err(Some(0u8 )));
      assert!(is_err(Some(0u16)));
      assert!(is_err(Some(0u32)));
      assert!(is_err(Some(0u64)));

      assert!(is_err(Some(0i8 )));
      assert!(is_err(Some(0i16)));
      assert!(is_err(Some(0i32)));
      assert!(is_err(Some(0i64)));

      assert!(is_err(Some(0f32)));
      assert!(is_err(Some(0f64)));

      assert!(is_err(Some(true )));
      assert!(is_err(Some(false)));

      assert!(is_err(Some("string")));
      assert!(is_err(Some(Bytes::new(b"byte slice"))));
    }

    /// Тестирует запись структур без полей
    #[test]
    fn test_unit() {
      #[derive(Serialize)]
      struct Unit;

      assert_eq!(to_vec(Unit), unit!());
      assert_eq!(to_vec(()), unit!());
    }

    /// Тестирует запись значения, обернутого в новый тип
    #[test]
    fn test_newtype() {
      /// Первая форма тестирует, что указанный в параметре тип не сериализуется,
      /// инициализируя сериализуемое значение значением по умолчанию его типа.
      ///
      /// Вторая форма делает то же самое, но инициализатор передается явно
      ///
      /// Третья форма проверяет, что результат сериализуется без ошибок и результат
      /// сериализации обернутого в новый тип выражения равен такому же выражению, не
      /// обернутому в новый тип
      macro_rules! newtype_test {
        ($type:ty) => (
          newtype_test!($type, Default::default())
        );
        ($type:ty, $value:expr) => ({
          #[derive(Serialize)]
          struct Newtype($type);

          let test = Newtype($value);
          assert!(is_err(test));
        });
        ($type:ty = $value:expr) => ({
          #[derive(Serialize)]
          struct NewtypeX($type);

          let wrapped = NewtypeX($value);
          let clear   = $value;

          assert_eq!(to_vec(wrapped), to_vec(clear));
        });
      }

      newtype_test!(u8);
      newtype_test!(u16);
      newtype_test!(u32);
      newtype_test!(u64);

      newtype_test!(i8);
      newtype_test!(i16);
      newtype_test!(i32);
      newtype_test!(i64);

      newtype_test!(f32);
      newtype_test!(f64);

      newtype_test!(bool);

      newtype_test!(String, "some string".into());
      newtype_test!(ByteBuf, ByteBuf::from(b"some vector".to_vec()));

      newtype_test!(() = ());

      #[derive(Serialize, Clone, Copy)]
      struct Item1 { payload: u32 };
      #[derive(Serialize, Clone, Copy)]
      struct Item2 { value: u64 };

      let item1 = Item1 { payload: 123456789 };
      let item2 = Item2 { value: 0xDEAD_BEAF_00FF_FF00 };

      newtype_test!((Item1, Item2), (item1, item2));

      #[derive(Serialize)]
      struct Unit;
      newtype_test!(Unit = Unit);

      #[derive(Serialize)]
      struct Newtype(Item1);
      newtype_test!(Newtype = Newtype(item1));

      #[derive(Serialize)]
      struct Tuple(Item1, Item2);
      newtype_test!(Tuple, Tuple(item1, item2));

      #[derive(Serialize)]
      struct Struct { field1: u32, field2: f32 };
      newtype_test!(Struct = Struct { field1: 42, field2: 42.0 });
    }

    /// Тестирует запись структуры с более чем одним полем
    #[test]
    fn test_struct() {
      #[derive(Serialize)]
      struct Struct {
        field1: String,
        field2: String,
      };

      let test = Struct {
        field1: "value".into(),
        field2: "another value".into(),
      };
      let expected = vec![
        // Заголовок
         b'G',b'F',b'F',b' ',// Тип файла
         b'V',b'3',b'.',b'2',// Версия
         56,0,0,0,   1,0,0,0,// Начальное смещение и количество структур (1 - корневая)
         68,0,0,0,   2,0,0,0,// Начальное смещение и количество полей (2 поля)
         92,0,0,0,   2,0,0,0,// Начальное смещение и количество меток (2 метки)
        124,0,0,0,  26,0,0,0,// Начальное смещение и количество байт данных ((4+5) + (4+13) байт)
        150,0,0,0,   8,0,0,0,// Начальное смещение и количество байт в списках полей (2*4 байт - 1 список с 2 полями)
        158,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках структур (списков нет)

        // Структуры
        // тег       ссылка      кол-во
        //          на данные    полей
          0,0,0,0,   0,0,0,0,   2,0,0,0,// Структура 0 (тег 0, 2 поля)

        // Поля
        // тип        метка     значение
        10,0,0,0,   0,0,0,0,   0,0,0,0,// Поле 1 (String) структуры 0 (ссылается на метку 1)
        10,0,0,0,   1,0,0,0,   9,0,0,0,// Поле 2 (String) структуры 0 (ссылается на метку 2)

        // Метки
        b'f',b'i',b'e',b'l',b'd',b'1',0,0,0,0,0,0,0,0,0,0,// Метка 1
        b'f',b'i',b'e',b'l',b'd',b'2',0,0,0,0,0,0,0,0,0,0,// Метка 2

        // Данные
          5,0,0,0, b'v',b'a',b'l',b'u',b'e',
        13,0,0,0, b'a',b'n',b'o',b't',b'h',b'e',b'r',b' ',b'v',b'a',b'l',b'u',b'e',

        // Списки полей структур
          0,0,0,0, 1,0,0,0,// Список 1, поля 1 и 2
      ];
      assert_eq!(to_vec(test), expected);
    }
  }

  mod as_field {
    //! Тестирует сериализацию различных значений, когда они включены как поле в структуру
    use super::*;
    use error::Result;

    /// Сериализует значение, оборачивая его в структуру, т.к. формат не поддерживает на
    /// верхнем уровне ничего, кроме структур
    #[inline]
    fn to_result<T>(value: T) -> Result<Vec<u8>>
      where T: Serialize,
    {
      #[derive(Serialize)]
      struct Storage<T: Serialize> {
        value: T
      }
      to_vec_((*b"GFF ").into(), &Storage { value })
    }

    /// Сериализует значение, оборачивая его в структуру, т.к. формат не поддерживает на
    /// верхнем уровне ничего, кроме структур
    #[inline]
    fn to_vec<T>(value: T) -> Vec<u8>
      where T: Serialize,
    {
      to_result(value).expect("Serialization fail")
    }

    /// Формирует байтовый массив, соответствующий сериализованной структуре без полей
    /// внутри корневой структуры в поле `value`
    macro_rules! unit {
      () => (
        vec![
          // Заголовок
           b'G',b'F',b'F',b' ',// Тип файла
           b'V',b'3',b'.',b'2',// Версия
           56,0,0,0,   2,0,0,0,// Начальное смещение и количество структур (2 структуры - корневая и тестируемая)
           80,0,0,0,   1,0,0,0,// Начальное смещение и количество полей (1 поле корневой структуры)
           92,0,0,0,   1,0,0,0,// Начальное смещение и количество меток (1 метка для поля корневой структуры)
          108,0,0,0,   0,0,0,0,// Начальное смещение и количество байт данных (данных нет)
          108,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках полей (списков нет)
          108,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках структур (списков нет)

          // Структуры
          // тег      ссылка      кол-во
          //         на данные    полей
          0,0,0,0,   0,0,0,0,   1,0,0,0,// Структура 0 (1 поле)
          0,0,0,0,   0,0,0,0,   0,0,0,0,// Структура 1 (полей нет)

          // Поля
          // тип      метка     значение
          14,0,0,0,  0,0,0,0,   1,0,0,0,// Поле структуры 0 (ссылаются на метку 1 и структуру 1)

          // Метки
          b'v',b'a',b'l',b'u',b'e',0,0,0,0,0,0,0,0,0,0,0,// Метка 1
        ]
      );
    }

    /// Тестирует запись простых числовых значений, которые хранятся в файле в разделе полей рядом с типом поля
    #[test]
    fn test_simple_numbers() {
      assert_eq!(to_vec( 42u8 ), primitive_wrapped![0; 0x2A,0,0,0]);
      assert_eq!(to_vec( 42u16), primitive_wrapped![2; 0x2A,0,0,0]);
      assert_eq!(to_vec( 42u32), primitive_wrapped![4; 0x2A,0,0,0]);

      assert_eq!(to_vec(-42i8 ), primitive_wrapped![1; 0xD6,0,0,0]);
      assert_eq!(to_vec(-42i16), primitive_wrapped![3; 0xD6,0xFF,0,0]);
      assert_eq!(to_vec(-42i32), primitive_wrapped![5; 0xD6,0xFF,0xFF,0xFF]);

      assert_eq!(to_vec( 42f32), primitive_wrapped![8; 0,0,0x28,0x42]);
      assert_eq!(to_vec(  0f32), primitive_wrapped![8; 0,0,0,0]);
      assert_eq!(to_vec(-42f32), primitive_wrapped![8; 0,0,0x28,0xC2]);
    }

    /// Тестирует запись комплексных числовых значений, которых хранятся в файле отдельно от описания самого поля
    #[test]
    fn test_complex_numbers() {
      assert_eq!(to_vec( 42u64), complex_wrapped![6; 0x2A,0,0,0,0,0,0,0]);
      assert_eq!(to_vec(-42i64), complex_wrapped![7; 0xD6,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF]);

      assert_eq!(to_vec( 42f64), complex_wrapped![9; 0,0,0,0,0,0,0x45,0x40]);
      assert_eq!(to_vec(  0f64), complex_wrapped![9; 0,0,0,0,0,0,0,0]);
      assert_eq!(to_vec(-42f64), complex_wrapped![9; 0,0,0,0,0,0,0x45,0xC0]);
    }

    /// Тестирует запись булевых значений, которые не поддерживаются форматом нативно
    #[test]
    fn test_bool() {
      assert_eq!(to_vec(true ), primitive_wrapped![0; 1,0,0,0]);
      assert_eq!(to_vec(false), primitive_wrapped![0; 0,0,0,0]);
    }

    /// Тестирует запись строковых срезов и строк
    #[test]
    fn test_string() {
      let slice = "юникод";
      let expected = complex_wrapped![10; 12,0,0,0, 0xD1,0x8E, 0xD0,0xBD, 0xD0,0xB8, 0xD0,0xBA, 0xD0,0xBE, 0xD0,0xB4];
      assert_eq!(to_vec(slice), expected);
      assert_eq!(to_vec(slice.to_owned()), expected);
    }

    /// Тестирует запись байтовых срезов и байтовых массивов
    #[test]
    fn test_bytes() {
      let array = b"Array with length more then 32 bytes";
      let expected = complex_wrapped![13; 36,0,0,0,
                                          b'A',b'r',b'r',b'a',b'y',b' ',
                                          b'w',b'i',b't',b'h',b' ',
                                          b'l',b'e',b'n',b'g',b't',b'h',b' ',
                                          b'm',b'o',b'r',b'e',b' ',
                                          b't',b'h',b'e',b'n',b' ',
                                          b'3',b'2',b' ',
                                          b'b',b'y',b't',b'e',b's'];

      #[derive(Serialize)]
      struct StorageRef<'a> {
        #[serde(with = "serde_bytes")]
        value: &'a [u8]
      }
      let storage = StorageRef { value: array.as_ref() };
      assert_eq!(to_vec_((*b"GFF ").into(), &storage).expect("Serialization fail"), expected);

      #[derive(Serialize)]
      struct StorageVec {
        #[serde(with = "serde_bytes")]
        value: Vec<u8>
      }
      let storage = StorageVec { value: array.to_vec() };
      assert_eq!(to_vec_((*b"GFF ").into(), &storage).expect("Serialization fail"), expected);
    }

    /// Тестирует запись отсутствующего опционального значения
    #[test]
    fn test_none() {
      macro_rules! none_test {
        ($type:ty) => (
          let none: Option<$type> = None;
          assert_eq!(to_vec(none), unit!());
        );
      }
      none_test!(u8);
      none_test!(u16);
      none_test!(u32);
      none_test!(u64);

      none_test!(i8);
      none_test!(i16);
      none_test!(i32);
      none_test!(i64);

      none_test!(f32);
      none_test!(f64);

      none_test!(String);
      none_test!(Vec<u8>);

      none_test!(());
      none_test!((u32, f32));

      #[derive(Serialize)]
      struct Unit;
      none_test!(Unit);

      #[derive(Serialize)]
      struct Newtype(u32);
      none_test!(Newtype);

      #[derive(Serialize)]
      struct Tuple(u32, f32);
      none_test!(Tuple);

      #[derive(Serialize)]
      struct Struct { field1: u32, field2: f32 };
      none_test!(Struct);
    }

    /// Тестирует запись присутствующего опционального значения
    #[test]
    fn test_some() {
      assert_eq!(to_vec(Some( 42u8 )), primitive_wrapped![0; 0x2A,0,0,0]);
      assert_eq!(to_vec(Some( 42u16)), primitive_wrapped![2; 0x2A,0,0,0]);
      assert_eq!(to_vec(Some( 42u32)), primitive_wrapped![4; 0x2A,0,0,0]);
      assert_eq!(to_vec(Some( 42u64)), complex_wrapped![6; 0x2A,0,0,0,0,0,0,0]);

      assert_eq!(to_vec(Some(-42i8 )), primitive_wrapped![1; 0xD6,0,0,0]);
      assert_eq!(to_vec(Some(-42i16)), primitive_wrapped![3; 0xD6,0xFF,0,0]);
      assert_eq!(to_vec(Some(-42i32)), primitive_wrapped![5; 0xD6,0xFF,0xFF,0xFF]);
      assert_eq!(to_vec(Some(-42i64)), complex_wrapped![7; 0xD6,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF]);

      assert_eq!(to_vec(Some(42f32)), primitive_wrapped![8; 0,0,0x28,0x42]);
      assert_eq!(to_vec(Some(42f64)), complex_wrapped![9; 0,0,0,0,0,0,0x45,0x40]);

      assert_eq!(to_vec(Some(true )), primitive_wrapped![0; 1,0,0,0]);
      assert_eq!(to_vec(Some(false)), primitive_wrapped![0; 0,0,0,0]);

      assert_eq!(to_vec(Some("string")), complex_wrapped![10; 6,0,0,0, b's',b't',b'r',b'i',b'n',b'g']);
      assert_eq!(to_vec(Some(Bytes::new(b"bytes"))), complex_wrapped![13; 5,0,0,0, b'b',b'y',b't',b'e',b's']);
    }

    /// Тестирует запись структур без полей
    #[test]
    fn test_unit() {
      #[derive(Serialize)]
      struct Unit;

      assert_eq!(to_vec(Unit), unit!());
      assert_eq!(to_vec(()), unit!());
    }

    /// Тестирует запись значения, обернутого в новый тип
    #[test]
    fn test_newtype() {
      /// Первая форма тестирует, что указанный в параметре тип не сериализуется,
      /// инициализируя сериализуемое значение значением по умолчанию его типа.
      ///
      /// Вторая форма делает то же самое, но инициализатор передается явно
      ///
      /// Третья форма проверяет, что результат сериализуется без ошибок и результат
      /// сериализации обернутого в новый тип выражения равен такому же выражению, не
      /// обернутому в новый тип
      macro_rules! newtype_test {
        ($type:ty = $value:expr) => ({
          #[derive(Serialize)]
          struct NewtypeX($type);

          let wrapped = NewtypeX($value);
          let clear   = $value;

          assert_eq!(to_vec(wrapped), to_vec(clear));
        });
      }

      newtype_test!(u8  = 42u8);
      newtype_test!(u16 = 42u16);
      newtype_test!(u32 = 42u32);
      newtype_test!(u64 = 42u64);

      newtype_test!(i8  = -42i8);
      newtype_test!(i16 = -42i16);
      newtype_test!(i32 = -42i32);
      newtype_test!(i64 = -42i64);

      newtype_test!(f32 = 42f32);
      newtype_test!(f64 = 42f64);

      newtype_test!(bool = true);
      newtype_test!(bool = false);

      newtype_test!(String = String::from("some string"));
      newtype_test!(ByteBuf = ByteBuf::from(b"some vector".to_vec()));

      newtype_test!(() = ());

      #[derive(Serialize, Clone, Copy)]
      struct Item1 { payload: u32 };
      #[derive(Serialize, Clone, Copy)]
      struct Item2 { value: u64 };

      let item1 = Item1 { payload: 123456789 };
      let item2 = Item2 { value: 0xDEAD_BEAF_00FF_FF00 };

      newtype_test!((Item1, Item2) = (item1, item2));

      #[derive(Serialize)]
      struct Unit;
      newtype_test!(Unit = Unit);

      #[derive(Serialize)]
      struct Newtype(Item1);
      newtype_test!(Newtype = Newtype(item1));

      #[derive(Serialize)]
      struct Tuple(Item1, Item2);
      newtype_test!(Tuple = Tuple(item1, item2));

      #[derive(Serialize)]
      struct Struct { field1: u32, field2: f32 };
      newtype_test!(Struct = Struct { field1: 42, field2: 42.0 });
    }

    /// Тестирует запись структуры с более чем одним полем - в структурах должны быть байтовые
    /// смещения на списке полей, а не индексы (хотя размер элементов известен и индексы были бы
    /// логичнее)
    #[test]
    fn test_struct() {
      #[derive(Serialize)]
      struct Nested {
        field1: String,
        field2: String,
      }
      #[derive(Serialize)]
      struct Struct {
        field1: String,
        field2: Nested,
        field3: String,
      }

      let test = Struct {
        field1: "value 1".into(),
        field2: Nested {
          field1: "value 2".into(),
          field2: "value 3".into(),
        },
        field3: "value 4".into(),
      };
      let expected = vec![
        // Заголовок
         b'G',b'F',b'F',b' ',// Тип файла
         b'V',b'3',b'.',b'2',// Версия
         56,0,0,0,   2,0,0,0,// Начальное смещение и количество структур (2 структуры - Struct + Nested)
         80,0,0,0,   5,0,0,0,// Начальное смещение и количество полей (5 полей)
        140,0,0,0,   3,0,0,0,// Начальное смещение и количество меток (3 метки)
        188,0,0,0,  44,0,0,0,// Начальное смещение и количество байт данных ((4+7)*4 байт)
        232,0,0,0,  20,0,0,0,// Начальное смещение и количество байт в списках полей (3*4 + 2*4 байт)
        252,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках структур (списков нет)

        // Структуры (2)
        // тег     ссылка      кол-во
        //        на данные    полей
        0,0,0,0,   0,0,0,0,   3,0,0,0,// Структура 0 (3 поля - смешение 0)
        0,0,0,0,  12,0,0,0,   2,0,0,0,// Структура 1 (2 поля - смещение 12 = 3*4)

        // Поля (5)
        // тип        метка     значение
        10,0,0,0,  0,0,0,0,   0,0,0,0,// Struct.field1 (String) - метка 0, смещение данных 0
        14,0,0,0,  1,0,0,0,   1,0,0,0,// Struct.field2 (Nested) - метка 1, структура 1
        10,0,0,0,  0,0,0,0,  11,0,0,0,// Nested.field1 (String) - метка 0, смещение данных 11
        10,0,0,0,  1,0,0,0,  22,0,0,0,// Nested.field2 (String) - метка 1, смещение данных 22
        10,0,0,0,  2,0,0,0,  33,0,0,0,// Struct.field3 (Strung) - метка 2, смещение данных 33

        // Метки (3)
        b'f',b'i',b'e',b'l',b'd',b'1',0,0,0,0,0,0,0,0,0,0,// Метка 1
        b'f',b'i',b'e',b'l',b'd',b'2',0,0,0,0,0,0,0,0,0,0,// Метка 2
        b'f',b'i',b'e',b'l',b'd',b'3',0,0,0,0,0,0,0,0,0,0,// Метка 3

        // Данные (44 байта)
         7,0,0,0, b'v',b'a',b'l',b'u',b'e',b' ',b'1',
         7,0,0,0, b'v',b'a',b'l',b'u',b'e',b' ',b'2',
         7,0,0,0, b'v',b'a',b'l',b'u',b'e',b' ',b'3',
         7,0,0,0, b'v',b'a',b'l',b'u',b'e',b' ',b'4',

        // Списки полей структур (20 байт)
        0,0,0,0,   1,0,0,0,   4,0,0,0,// Список 1 (для Struct), поля 0, 1 и 4
        2,0,0,0,   3,0,0,0,           // Список 2 (для Nested), поля 2 и 3
      ];
      assert_eq!(to_vec_((*b"GFF ").into(), &test).expect("Serialization fail"), expected);
    }
  }
}
