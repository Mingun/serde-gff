//! Сериализатор для формата Bioware GFF (Generic File Format)

use indexmap::IndexSet;
use serde::ser::{self, Impossible, Serialize};

use Label;
use error::{Error, Result};
use raw::{Struct, Field};

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
