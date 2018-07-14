//! Сериализатор для формата Bioware GFF (Generic File Format)

use std::io::Write;
use byteorder::{LE, WriteBytesExt};
use indexmap::IndexSet;
use serde::ser::{self, Impossible, Serialize, SerializeSeq, SerializeStruct, SerializeTuple, SerializeTupleStruct};

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
    unsupported!($ser_method($($type),*) -> Self::Ok);
  );
  ($ser_method:ident ( $($type:ty),* ) -> $result:ty) => (
    fn $ser_method(self, $(_: $type),*) -> Result<$result> {
      Err(Error::Serialize(concat!(
        "`", stringify!($ser_method), "` can't be implemented in GFF format. Wrap value to the struct and serialize struct"
      ).into()))
    }
  );
}

impl<'a> ser::Serializer for &'a mut Serializer {
  type Ok = ();
  type Error = Error;

  type SerializeSeq = ListSerializer<'a>;
  type SerializeTuple = Impossible<Self::Ok, Self::Error>;
  type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
  type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
  type SerializeMap = Impossible<Self::Ok, Self::Error>;
  type SerializeStruct = StructSerializer<'a>;
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

  #[inline]
  fn serialize_none(self) -> Result<Self::Ok> {
    self.serialize_unit()
  }
  fn serialize_some<T>(self, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    value.serialize(self)
  }
  //-----------------------------------------------------------------------------------------------
  // Сериализация структурных элементов
  //-----------------------------------------------------------------------------------------------
  #[inline]
  fn serialize_unit(self) -> Result<Self::Ok> {
    self.add_struct(0);
    Ok(())
  }
  #[inline]
  fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok> {
    self.serialize_unit()
  }
  fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    value.serialize(self)
  }
  fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
    Err(Error::Serialize(format!(
      "`serialize_tuple(len: {})` can't be implemented in GFF format. Wrap value to the struct and serialize struct",
      len
    )))
  }
  fn serialize_tuple_struct(self, name: &'static str, len: usize) -> Result<Self::SerializeTupleStruct> {
    Err(Error::Serialize(format!(
      "`serialize_tuple_struct(name: {}, len: {})` can't be implemented in GFF format. Wrap value to the struct and serialize struct",
      name, len
    )))
  }
  fn serialize_struct(self, _name: &'static str, len: usize) -> Result<Self::SerializeStruct> {
    let (struct_index, fields_index) = self.add_struct(len);
    Ok(StructSerializer { ser: self, struct_index, fields_index })
  }
  //-----------------------------------------------------------------------------------------------
  // Сериализация последовательностей и отображений
  //-----------------------------------------------------------------------------------------------
  unsupported!(serialize_seq(Option<usize>) -> Self::SerializeSeq);
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

/// Сериализатор, записывающий значение поля
struct FieldSerializer<'a> {
  /// Хранилище записываемых данных
  ser: &'a mut Serializer,
  /// Номер метки, ассоциированной с сериализуемым полем
  label: LabelIndex,
}
impl<'a> FieldSerializer<'a> {
  /// Добавляет в список структур новую структуру с указанным количеством полей, а
  /// в список полей -- новое поле типа "структура".
  ///
  /// Корректная ссылка на данные еще не заполнена, ее нужно будет скорректировать
  /// после того, как содержимое структуры будет записано
  ///
  /// Возвращает пару индексов: добавленной структуры и списка с полями структуры,
  /// если полей несколько
  fn add_struct(&mut self, fields: usize) -> Result<(StructIndex, FieldListIndex)> {
    // Добавляем запись о структуре
    let (struct_index, fields_index) = self.ser.add_struct(fields);

    self.ser.fields.push(Field::Struct {
      label: self.label,
      struct_: struct_index
    });
    Ok((struct_index, fields_index))
  }
}
/// Записывает значения поля, чей размер не превышает 4 байта
macro_rules! primitive {
  ($ser_method:ident, $type:ty, $tag:ident) => (
    #[inline]
    fn $ser_method(self, v: $type) -> Result<Self::Ok> {
      self.ser.fields.push(Field::Simple {
        label: self.label,
        value: SimpleValueRef::$tag(v)
      });
      Ok(())
    }
  );
}
/// Записывает значения поля, чей размер превышает 4 байта
macro_rules! complex {
  ($ser_method:ident, $type:ty, $tag:ident, $write_method:ident) => (
    #[inline]
    fn $ser_method(self, v: $type) -> Result<Self::Ok> {
      let offset = self.ser.field_data.len() as u32;
      // Записываем данные поля в сторонке
      self.ser.field_data.$write_method::<LE>(v)?;

      // Добавляем само поле
      self.ser.fields.push(Field::Simple {
        label: self.label,
        value: SimpleValueRef::$tag(offset.into())
      });
      Ok(())
    }
  );
  ($ser_method:ident, $type:ty, $tag:ident) => (
    #[inline]
    fn $ser_method(self, v: $type) -> Result<Self::Ok> {
      let offset = self.ser.field_data.len() as u32;
      // Записываем данные поля в сторонке
      self.ser.field_data.write_u32::<LE>(v.len() as u32)?;
      self.ser.field_data.write_all(v.as_ref())?;

      // Добавляем само поле
      self.ser.fields.push(Field::Simple {
        label: self.label,
        value: SimpleValueRef::$tag(offset.into())
      });
      Ok(())
    }
  );
}
impl<'a> ser::Serializer for FieldSerializer<'a> {
  type Ok = ();
  type Error = Error;

  type SerializeSeq = ListSerializer<'a>;
  type SerializeTuple = Self::SerializeSeq;
  type SerializeTupleStruct = Self::SerializeSeq;
  type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
  type SerializeMap = Impossible<Self::Ok, Self::Error>;
  type SerializeStruct = StructSerializer<'a>;
  type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

  primitive!(serialize_u8 , u8 , Byte);
  primitive!(serialize_i8 , i8 , Char);
  primitive!(serialize_u16, u16, Word);
  primitive!(serialize_i16, i16, Short);
  primitive!(serialize_u32, u32, Dword);
  primitive!(serialize_i32, i32, Int);
  complex!  (serialize_u64, u64, Dword64, write_u64);
  complex!  (serialize_i64, i64, Int64, write_i64);

  primitive!(serialize_f32, f32, Float);
  complex!  (serialize_f64, f64, Double, write_f64);

  /// Формат не поддерживает сериализацию булевых значений, поэтому значение сериализуется,
  /// как `u8`: `true` представляется в виде `1`, а `false` -- в виде `0`.
  #[inline]
  fn serialize_bool(self, v: bool) -> Result<Self::Ok> {
    self.serialize_u8(if v { 1 } else { 0 })
  }
  /// Формат не поддерживает сериализацию произвольных символов как отдельную сущность, поэтому
  /// они сериализуются, как строка из одного символа
  #[inline]
  fn serialize_char(self, v: char) -> Result<Self::Ok> {
    let mut data = [0u8; 4];
    self.serialize_str(v.encode_utf8(&mut data))
  }

  complex!(serialize_str  , &str , String);
  complex!(serialize_bytes, &[u8], Void);

  #[inline]
  fn serialize_none(self) -> Result<Self::Ok> {
    self.serialize_unit()
  }
  #[inline]
  fn serialize_some<T>(self, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    value.serialize(self)
  }
  //-----------------------------------------------------------------------------------------------
  // Сериализация структурных элементов
  //-----------------------------------------------------------------------------------------------
  #[inline]
  fn serialize_unit(mut self) -> Result<Self::Ok> {
    self.add_struct(0)?;
    Ok(())
  }
  #[inline]
  fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok> {
    self.serialize_unit()
  }
  #[inline]
  fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    value.serialize(self)
  }
  #[inline]
  fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple> {
    let list_index = self.ser.add_list(self.label, len);
    Ok(ListSerializer { ser: self.ser, list_index })
  }
  #[inline]
  fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<Self::SerializeTupleStruct> {
    self.serialize_tuple(len)
  }
  #[inline]
  fn serialize_struct(mut self, _name: &'static str, len: usize) -> Result<Self::SerializeStruct> {
    let (struct_index, fields_index) = self.add_struct(len)?;
    Ok(StructSerializer { ser: self.ser, struct_index, fields_index })
  }
  //-----------------------------------------------------------------------------------------------
  // Сериализация последовательностей и отображений
  //-----------------------------------------------------------------------------------------------
  #[inline]
  fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq> {
    self.serialize_tuple(len.unwrap_or(0))
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

/// Сериализует все поля структуры, заполняя массив с индексами полей (если полей
/// в структуре несколько) или обновляя индекс в структуре на поле, если поле одно.
pub struct StructSerializer<'a> {
  /// Хранилище записываемых данных
  ser: &'a mut Serializer,
  /// Номер структуры в массиве `ser.structs`, которую нужно обновить по завершении
  /// сериализации структуры
  struct_index: StructIndex,
  /// Номер списка полей в массиве `ser.field_indices`, в который необходимо помещать
  /// индексы полей по мере их сериализации
  fields_index: FieldListIndex,
}
impl<'a> SerializeStruct for StructSerializer<'a> {
  type Ok = ();
  type Error = Error;

  #[inline]
  fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    use self::Struct::*;

    // Добавляем запись о метке
    let label = self.ser.add_label(key)?;
    let index = self.ser.fields.len();
    value.serialize(FieldSerializer { ser: self.ser, label })?;
    // Обновляем ссылки из записи о структуре
    let struct_ = &mut self.ser.structs[self.struct_index.0];
    match struct_ {
      // Если полей нет, ничего делать не нужно
      NoFields => {},
      // Если поле одно, то структура хранит ссылку на само поле
      OneField(ref mut idx) => *idx = index,
      MultiField {..} => {
        // Если полей несколько, то структура содержит ссылку на список с полями. Добавляем
        // индекс этого поля в нее
        let fields = &mut self.ser.field_indices[self.fields_index.0];
        fields.push(index as u32);
      },
    };
    Ok(())
  }

  #[inline]
  fn end(self) -> Result<Self::Ok> { Ok(()) }
}

/// Сериализует все поля списка или кортежа, заполняя массив с индексами элементов списка
pub struct ListSerializer<'a> {
  /// Хранилище записываемых данных
  ser: &'a mut Serializer,
  /// Индекс в массиве `ser.list_indices`, определяющий заполняемый данным сериализатором
  /// список с индексами структур, составляющих элементы списка.
  list_index: ListIndex,
}

impl<'a> SerializeSeq for ListSerializer<'a> {
  type Ok = ();
  type Error = Error;

  #[inline]
  fn serialize_element<T>(&mut self, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    let index = self.ser.structs.len() as u32;
    {
      let list = &mut self.ser.list_indices[self.list_index.0];
      list.push(index);
    }
    value.serialize(&mut *self.ser)
  }

  #[inline]
  fn end(self) -> Result<()> { Ok(()) }
}

impl<'a> SerializeTuple for ListSerializer<'a> {
  type Ok = ();
  type Error = Error;

  #[inline]
  fn serialize_element<T>(&mut self, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    <Self as SerializeSeq>::serialize_element(self, value)
  }
  #[inline]
  fn end(self) -> Result<()> { <Self as SerializeSeq>::end(self) }
}

impl<'a> SerializeTupleStruct for ListSerializer<'a> {
  type Ok = ();
  type Error = Error;

  #[inline]
  fn serialize_field<T>(&mut self, value: &T) -> Result<Self::Ok>
    where T: ?Sized + Serialize,
  {
    <Self as SerializeSeq>::serialize_element(self, value)
  }
  #[inline]
  fn end(self) -> Result<()> { <Self as SerializeSeq>::end(self) }
}

#[cfg(test)]
mod tests {
  extern crate serde_bytes;

  use std::collections::BTreeMap;
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
  /// Формирует байтовый массив, соответствующий сериализованной структуре с одним полем
  /// `value` со списком элементов заданного типа, который хранится в записи о самом поле.
  macro_rules! list_wrapped {
    ($($type:expr; $b1:expr, $b2:expr, $b3:expr, $b4:expr;)*) => ({
      let cnt = len!($($type)*) as u8;
      let mut i = 0u8;// Номер поля структуры
      let mut j = 0u8;// Номер структуры в списке
      let len = (1 + len!($($type)*) as u8) * 4;

      vec![
        // Заголовок
        b'G',b'F',b'F',b' ',// Тип файла
        b'V',b'3',b'.',b'2',// Версия
        56,0,0,0,   1 + cnt,0,0,0,// Начальное смещение и количество структур (cnt + 1 - корневая)
        68 + cnt*4*3  ,0,0,0,   1 + cnt,0,0,0,// Начальное смещение и количество полей (cnt + 1 поле - по одному на каждую структуру)
        80 + cnt*4*3*2,0,0,0,   1,0,0,0,// Начальное смещение и количество меток (1 метка - у всех структур поля называются одинаково)
        96 + cnt*4*3*2,0,0,0,   0,0,0,0,// Начальное смещение и количество байт данных (байт данных нет)
        96 + cnt*4*3*2,0,0,0,   0,0,0,0,// Начальное смещение и количество байт в списках полей (списков нет)
        96 + cnt*4*3*2,0,0,0, len,0,0,0,// Начальное смещение и количество байт в списках структур (x списков)

        // Структуры
        // тег     ссылка      кол-во
        //        на данные    полей
        0,0,0,0,   0,0,0,0,   1,0,0,0,// Структура 0 (тег 0, 1 поле)
        $(
          0,0,0,0,  replace_expr!($type {i+=1; i}),0,0,0,  1,0,0,0,// Структура i (тег 0, 1 поле)
        )*

        // Поля
        // тип      метка     значение
        15,0,0,0,  0,0,0,0,   0,0,0,0,// Поле структуры 0 (ссылаются на метку 1 и список 1)
        $(
          $type,0,0,0,  0,0,0,0,  $b1,$b2,$b3,$b4,// Поля (ссылаются на метку 1)
        )*

        // Метки
        b'v',b'a',b'l',b'u',b'e',0,0,0,0,0,0,0,0,0,0,0,// Метка 1

        // Ссылки на элементы списков
        cnt,0,0,0, $(replace_expr!($type {j+=1; j}),0,0,0,)*// Список 1
      ]
    });
  }

  /// Создает отображение ключей на значения
  macro_rules! map {
    () => (
      BTreeMap::new()
    );
    ($($k:expr => $v:expr),+) => (
      map!($($k => $v,)*)
    );
    ($($k:expr => $v:expr,)+) => (
      {
        let mut m = BTreeMap::new();
        $(
          m.insert($k, $v);
        )+
        m
      }
    );
  }

  macro_rules! map_tests {
    () => (
      /// Тестирует запись отображения строк на значения
      #[test]
      fn test_map() {
        // Пустая карта аналогична пустой или Unit-структуре
        let empty: BTreeMap<String, ()> = map![];
        assert_eq!(to_vec(empty), to_vec(()));

        #[derive(Serialize)]
        struct S {
          field1: u32,
          field2: u32,
        }

        // Карта с полями аналогична структуре
        let map = map![
          "field1".to_string() => 1u32,
          "field2".to_string() => 2u32,
        ];
        assert_eq!(to_vec(map), to_vec(S { field1: 1, field2: 2 }));

        // карта с не строковыми ключами не может быть сериализована
        // TODO: Ослабить ограничение до типажа AsStr<str>
        let map = map![
          1u32 => 1u32,
          2u32 => 2u32,
        ];
        assert!(is_err(map));
      }
    );
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

    /// Тестирует запись кортежа из значений разных типов, не все из которых являются структурами
    #[test]
    fn test_tuple_with_non_struct_item() {
      #[derive(Serialize, Clone, Copy)]
      struct Item { value: u32 }
      #[derive(Serialize)]
      struct Tuple1(u32, f32);
      #[derive(Serialize)]
      struct Tuple2(Item, f32);

      let item = Item { value: 42 };

      assert!(is_err((42u32, 42f32)));
      assert!(is_err((item, 42f32)));
      assert!(is_err(Tuple1(42, 42.0)));
      assert!(is_err(Tuple2(item, 42.0)));
    }

    /// Тестирует запись кортежа из значений структур разных типов
    #[test]
    fn test_tuple_with_struct_item() {
      #[derive(Serialize, Clone, Copy)]
      struct Item1 { value: u32 }
      #[derive(Serialize, Clone, Copy)]
      struct Item2 { value: f32 }
      #[derive(Serialize)]
      struct Tuple(Item1, Item2);

      let item1 = Item1 { value: 42 };
      let item2 = Item2 { value: 42.0 };

      assert!(is_err((item1, item2)));
      assert!(is_err(Tuple(item1, item2)));
    }

    /// Тестирует запись списков с элементом не структурой. Запись таких списков невозможна
    #[test]
    fn test_list_with_non_struct_item() {
      let array = [
        41u8,
        42u8,
        43u8,
      ];
      let owned = array.to_vec();

      assert!(is_err(owned));
      assert!(is_err(&array[..]));
      assert!(is_err(array));
    }

    /// Тестирует запись списков с элементом-структурой. Только такие списки могут быть записаны
    #[test]
    fn test_list_with_struct_item() {
      #[derive(Serialize, Clone)]
      struct Item<T: Serialize + Clone> {
        value: T
      }

      let array = [
        Item { value: 41u8 },
        Item { value: 42u8 },
        Item { value: 43u8 },
      ];
      let owned = array.to_vec();

      assert!(is_err(owned));
      assert!(is_err(&array[..]));
      assert!(is_err(array));
    }
    map_tests!();
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

    #[inline]
    fn is_err<T>(value: T) -> bool
      where T: Serialize,
    {
      to_result(value).is_err()
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

    /// Тестирует запись кортежа из значений разных типов, не все из которых являются структурами
    #[test]
    fn test_tuple_with_non_struct_item() {
      #[derive(Serialize, Clone, Copy)]
      struct Item { value: u32 }
      #[derive(Serialize)]
      struct Tuple1(u32, f32);
      #[derive(Serialize)]
      struct Tuple2(Item, f32);

      let item = Item { value: 42 };

      assert!(is_err((42u32, 42f32)));
      assert!(is_err((item, 42f32)));
      assert!(is_err(Tuple1(42, 42.0)));
      assert!(is_err(Tuple2(item, 42.0)));
    }

    /// Тестирует запись кортежа из значений структур разных типов
    #[test]
    fn test_tuple_with_struct_item() {
      #[derive(Serialize, Clone, Copy)]
      struct Item1 { value: u32 }
      #[derive(Serialize, Clone, Copy)]
      struct Item2 { value: f32 }
      #[derive(Serialize)]
      struct Tuple(Item1, Item2);

      let item1 = Item1 { value: 42 };
      let item2 = Item2 { value: 42.0 };

      let expected = list_wrapped![
        4; 42,0,0,0;
        8; 0,0,0x28,0x42;
      ];
      assert_eq!(to_vec((item1, item2)), expected);
      assert_eq!(to_vec(Tuple(item1, item2)), expected);
    }

    /// Тестирует запись списков с элементом не структурой. Запись таких списков невозможна
    #[test]
    fn test_list_with_non_struct_item() {
      let array = [
        41u8,
        42u8,
        43u8,
      ];
      let owned = array.to_vec();

      assert!(is_err(owned));
      assert!(is_err(&array[..]));
      assert!(is_err(array));
    }

    /// Тестирует запись списков с элементом-структурой. Только такие списки могут быть записаны
    #[test]
    fn test_list_with_struct_item() {
      #[derive(Serialize, Copy, Clone)]
      struct Item { value: u8 }

      let array = [
        Item { value: 41 },
        Item { value: 42 },
        Item { value: 43 },
      ];
      let owned = array.to_vec();

      let expected = list_wrapped![
        0; 41,0,0,0;
        0; 42,0,0,0;
        0; 43,0,0,0;
      ];

      assert_eq!(to_vec(owned), expected);
      assert_eq!(to_vec(&array[..]), expected);
      assert_eq!(to_vec(array), expected);
    }
    /// Тестирует, что ссылки на списки элементов идут по байтовым смещениям, а не по индексам
    /// элементов в списке
    #[test]
    fn test_multilist_with_struct_item() {
      #[derive(Serialize)]
      struct Item;

      #[derive(Serialize)]
      struct List {
        list1: Vec<Item>,
        list2: Vec<Item>,
      }

      let list = List {
        list1: vec![Item, Item, Item],
        list2: vec![Item, Item],
      };

      let expected = vec![
        b'G',b'F',b'F',b' ',
        b'V',b'3',b'.',b'2',
         56,0,0,0,   6,0,0,0,// 6 структур - List + 5*Item
        128,0,0,0,   2,0,0,0,// 2 поля - list1, list2
        152,0,0,0,   2,0,0,0,// 2 метки - list1, list2
        184,0,0,0,   0,0,0,0,// Байт данных нет
        184,0,0,0,   8,0,0,0,// 8 = 2*4 байт в списках индексов полей структур для структуры List с 2 полями
        192,0,0,0,  28,0,0,0,// 28 = (3+1 + 2+1)*4 байт в списках индексов элементов списков

        // Структуры (6)
        // тег       ссылка      кол-во
        //          на данные    полей
          0,0,0,0,   0,0,0,0,   2,0,0,0,// List - 2 поля
          0,0,0,0,   0,0,0,0,   0,0,0,0,// Item 0 - 0 полей
          0,0,0,0,   0,0,0,0,   0,0,0,0,// Item 1 - 0 полей
          0,0,0,0,   0,0,0,0,   0,0,0,0,// Item 2 - 0 полей
          0,0,0,0,   0,0,0,0,   0,0,0,0,// Item 3 - 0 полей
          0,0,0,0,   0,0,0,0,   0,0,0,0,// Item 4 - 0 полей

        // Поля (2)
        // тип        метка     значение
         15,0,0,0,   0,0,0,0,   0,0,0,0,// list1 - смещение 0
         15,0,0,0,   1,0,0,0,  16,0,0,0,// list2 - смещение 16 = (3+1)*4

        // Метки (2)
         b'l',b'i',b's',b't',b'1',0,0,0,0,0,0,0,0,0,0,0,// list1
         b'l',b'i',b's',b't',b'2',0,0,0,0,0,0,0,0,0,0,0,// list2

        // Данных нет

        // Списки полей структур (8 байт)
          0,0,0,0,   1,0,0,0,// Список 1 (для List) - поля 0 и 1

        // Списки элементов в списках (28 байт)
          3,0,0,0,   1,0,0,0,   2,0,0,0,   3,0,0,0,// Список 1 (3 элемента) - структуры 1, 2, 3
          2,0,0,0,   4,0,0,0,   5,0,0,0            // Список 2 (2 элемента) - структуры 4, 5
      ];
      assert_eq!(to_vec_((*b"GFF ").into(), &list).expect("Serialization fail"), expected);
    }
    /// Тестирует, что ссылки на списки элементов корректны при наличии вложенных списков
    #[test]
    fn test_nested_list_with_struct_item() {
      #[derive(Serialize)]
      struct Item {
        list: Vec<()>,
      }

      #[derive(Serialize)]
      struct List {
        list: Vec<Item>,
      }

      let list = List {
        list: vec![
          Item { list: vec![] },
          Item { list: vec![(), ()] },
          Item { list: vec![] },
        ],
      };

      let expected = vec![
        b'G',b'F',b'F',b' ',
        b'V',b'3',b'.',b'2',
         56,0,0,0,   6,0,0,0,// 6 структур - List + 3*Item + 2*()
        128,0,0,0,   4,0,0,0,// 4 поля - list в 4-х структурах
        176,0,0,0,   1,0,0,0,// 1 метка - list1, list2
        192,0,0,0,   0,0,0,0,// Байт данных нет
        192,0,0,0,   0,0,0,0,// Списков полей нет - Все структуры с одним полем
        192,0,0,0,  36,0,0,0,// 36 = (3+1 + 0+1 + 2+1 + 0+1)*4 байт в списках индексов элементов списков

        // Структуры (6)
        // тег       ссылка      кол-во
        //          на данные    полей
          0,0,0,0,   0,0,0,0,   1,0,0,0,// List - 1 поле
          0,0,0,0,   1,0,0,0,   1,0,0,0,// Item 0 (поле 1) - 0 полей
          0,0,0,0,   2,0,0,0,   1,0,0,0,// Item 1 (поле 2) - 0 полей
          0,0,0,0,   0,0,0,0,   0,0,0,0,// () 0 - 0 полей
          0,0,0,0,   0,0,0,0,   0,0,0,0,// () 1 - 0 полей
          0,0,0,0,   3,0,0,0,   1,0,0,0,// Item 2 (поле 3) - 0 полей

        // Поля (4)
        // тип        метка     значение
         15,0,0,0,   0,0,0,0,   0,0,0,0,// list (в List) - смещение 0
         15,0,0,0,   0,0,0,0,  16,0,0,0,// list (в Item 0) - смещение 16 = (3+1)*4
         15,0,0,0,   0,0,0,0,  20,0,0,0,// list (в Item 1) - смещение 20 = (3+1 + 0+1)*4
         15,0,0,0,   0,0,0,0,  32,0,0,0,// list (в Item 2) - смещение 32 = (3+1 + 0+1 + 2+1)*4

        // Метки (1)
         b'l',b'i',b's',b't',0,0,0,0,0,0,0,0,0,0,0,0,// list

        // Данных нет

        // Списков полей структур нет

        // Списки элементов в списках (36 байт)
          3,0,0,0,   1,0,0,0,   2,0,0,0,   5,0,0,0,// Список 1 (3 элемента) - структуры 1, 2, 5
          0,0,0,0,                                 // Список 2 (0 элементов)
          2,0,0,0,   3,0,0,0,   4,0,0,0,           // Список 3 (2 элемента) - структуры 3, 4
          0,0,0,0,                                 // Список 4 (0 элементов)
      ];
      assert_eq!(to_vec_((*b"GFF ").into(), &list).expect("Serialization fail"), expected);
    }
    map_tests!();
  }
}
