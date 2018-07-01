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
