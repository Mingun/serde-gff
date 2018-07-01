//! Сериализатор для формата Bioware GFF (Generic File Format)

use indexmap::IndexSet;
use serde::ser::{self, Impossible, Serialize};

use Label;
use error::{Error, Result};
use index::LabelIndex;
use value::SimpleValueRef;

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
