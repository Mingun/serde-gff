//! Вспомогательный модуль, содержащий описание структур, непосредственно хранимых
//! в GFF файле на диске. Обычно нет необходимости использовать данный модуль -- он
//! может понадобиться только при отладке
use std::fmt;
use std::io::{Cursor, Read, Seek, SeekFrom, Write, Result};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};

use header::Header;
use Label;

/// Типы полей, которые возможно встретить в GFF файле
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum FieldType {
  /// Беззнаковое байтовое значение (от 0 до 255), занимающее один байт
  Byte,
  /// Символ текста в диапазоне `0x00-0xFF`, занимающий один байт
  Char,
  /// Беззнаковое целое (от 0 до 65535), занимающее 2 байта
  Word,
  /// Знаковое целое (от -32768 до 32767), занимающее 2 байта
  Short,
  /// Беззнаковое целое (от 0 до 4294967296), занимающее 4 байта
  Dword,
  /// Знаковое целое (от -2147483648 до 2147483647), занимающее 4 байта
  Int,
  /// Беззнаковое целое (от 0 до примерно 18e+18), занимающее 8 байт
  Dword64,
  /// Знаковое целое (примерно от -9e+18 до +9e+18), занимающее 8 байт
  Int64,
  /// Число с плавающей запятой одинарной точности, занимающее 4 байта
  Float,
  /// Число с плавающей запятой двойной точности, занимающее 8 байт
  Double,
  /// Нелокализуемая строка.
  ///
  /// Предпочитаемый максимальный размер - 1024 символа. Это ограничение установлено в первую
  /// очередь для того, чтобы сохранить пропускную способность сети в том случае, если строку
  /// необходимо передавать с сервера на клиент.
  ///
  /// Данный вид строк не должен использоваться для текста, который может увидеть игрок, так как
  /// он будет одинаковым независимо от языка клиента игры. Область применения данного типа -
  /// текст для разработчиков/дизайнеров уровней, например, тегов объектов, используемых в скриптах.
  String,
  /// Имя файла ресурса, до 16 символов
  ResRef,
  /// Локализуемая строка. Содержит `StringRef` и несколько `CExoString`, каждую со своим номером языка
  LocString,
  /// Произвольные данные любой длины
  Void,
  /// Вложенная структура
  Struct,
  /// Список значений любой длины
  List,
}
impl FieldType {
  /// Возвращает `true`, если данные поля указанного типа хранятся не в структуре [`Field`], а
  /// в отдельной области полей GFF файла. Поля типа `Struct` и `List` хранятся совершенно отдельно
  /// и данный метод для них возвращает `false`
  ///
  /// [`Field`]: struct.Field.html
  #[inline]
  pub fn is_complex(&self) -> bool {
    use self::FieldType::*;

    match *self {
      Dword64 | Int64 | Double | String | ResRef | LocString | Void => true,
      _ => false
    }
  }
  /// Возвращает `true`, если данные поля указанного типа хранятся внутри структуры [`Field`]
  ///
  /// [`Field`]: struct.Field.html
  #[inline]
  pub fn is_simple(&self) -> bool {
    !self.is_complex() && *self != FieldType::Struct && *self != FieldType::List
  }
  //TODO: После стабилизации https://github.com/rust-lang/rust/issues/33417 полностью перенести в TryFrom
  #[inline]
  fn from_u32(value: u32) -> Option<Self> {
    use self::FieldType::*;

    Some(match value {
       0 => Byte,
       1 => Char,
       2 => Word,
       3 => Short,
       4 => Dword,
       5 => Int,
       6 => Dword64,
       7 => Int64,
       8 => Float,
       9 => Double,
      10 => String,
      11 => ResRef,
      12 => LocString,
      13 => Void,
      14 => Struct,
      15 => List,
      _ => return None,
    })
  }
}

#[cfg(nightly)]
impl TryFrom<u32> for FieldType {
  type Error = NoneError;

  #[inline]
  fn try_from(value: u32) -> Result<Self, Self::Error> {
    Ok(self.from_u32(value)?)
  }
}

/// Описание структуры, как оно хранится в GFF файле
pub struct Struct {
  /// Идентификатор типа структуры. Игрой на самом деле почти никогда не используется.
  /// При записи сюда сериализатор всегда записывает сюда 0
  pub tag: u32,
  /// Или индекс в массив полей (если `self.fields == 1`), или в смещение в массиве индексов полей
  pub offset: u32,
  /// Количество полей структуры
  pub fields: u32,
}
impl Struct {
  /// Читает 12 байт значения структуры из потока
  #[inline]
  pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
    Ok(Struct {
      tag:    reader.read_u32::<LE>()?,
      offset: reader.read_u32::<LE>()?,
      fields: reader.read_u32::<LE>()?,
    })
  }
  /// Записывает 12 байт значения структуры в поток
  #[inline]
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    writer.write_u32::<LE>(self.tag)?;
    writer.write_u32::<LE>(self.offset)?;
    writer.write_u32::<LE>(self.fields)?;
    Ok(())
  }
}
impl fmt::Debug for Struct {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "Struct {{ tag: {:?}, offset: {:?}, fields: {:?} }}", self.tag, self.offset, self.fields)
  }
}

/// Описание поля структуры, как оно хранится в GFF файле
pub struct Field {
  /// Идентификатор типа поля
  pub tag: u32,
  /// Индекс в массив меток, определяющий метку, привязанную к данному полю
  pub label: u32,
  /// Сами данные для простых данных или смещение в массиве с данными для комплексных
  /// типов. Также, если поле представляет собой структуру, то это индекс в массиве
  /// структур, а если список -- байтовое смещение в массиве списков (хотя сам массив списков
  /// состоит из элементов размером 4 байта).
  pub data: [u8; 4],
}
impl Field {
  /// Читает 12 байт значения поля из потока
  #[inline]
  pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
    let tag   = reader.read_u32::<LE>()?;
    let label = reader.read_u32::<LE>()?;
    let mut data = [0u8; 4];
    reader.read_exact(&mut data)?;

    Ok(Field { tag, label, data })
  }
  /// Записывает 12 байт значения поля в поток
  #[inline]
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    writer.write_u32::<LE>(self.tag as u32)?;
    writer.write_u32::<LE>(self.label)?;
    writer.write_all(&self.data)?;
    Ok(())
  }
}
impl fmt::Debug for Field {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "Field {{ tag: {:?}, label: {:?}, data: {:?} }}", self.tag, self.label, self.data)
  }
}

/// Данные для одного поля, хранимые в поле [`Gff::field_data`]. Используется для улучшения отладочного вывода
///
/// [`Gff::field_data`]: ../struct.Gff.html#field.field_data
struct FieldData<'a>(&'a [u8]);
impl<'a> fmt::Debug for FieldData<'a> {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{:?}", self.0)
  }
}
/// Представление данных комплексных полей для отладочного вывода
#[derive(Debug)]
struct DebugFieldData<'a> {
  /// Данные, разбитые по принадлежности к отдельным полям, для удобства отладочного вывода
  by_field: Vec<FieldData<'a>>,
  /// Данные в том виде, в каком они хранятся в файле
  raw: &'a [u8],
}

/// Список индексов для одной структуры, хранимые в поле [`Gff::field_indices`]
///
/// [`Gff::field_indices`]: ../struct.Gff.html#field.field_indices
struct FieldIndex<'a>(&'a [u32]);
impl<'a> fmt::Debug for FieldIndex<'a> {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{:?}", self.0)
  }
}
/// Представление списка индексов полей структур для отладочного вывода
#[derive(Debug)]
struct DebugFieldIndex<'a> {
  /// Данные, разбитые по принадлежности к отдельным структурам, для удобства отладочного вывода
  by_struct: Vec<FieldIndex<'a>>,
  /// Данные в том виде, в каком они хранятся в файле
  raw: &'a [u32],
}

/// Список индексов для одного списка, хранимый в поле [`Gff::list_indices`]
///
/// [`Gff::list_indices`]: ../struct.Gff.html#field.list_indices
struct ListIndex<'a>(&'a [u32]);
impl<'a> fmt::Debug for ListIndex<'a> {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{:?}", self.0)
  }
}
/// Представление списка индексов полей структур для отладочного вывода
#[derive(Debug)]
struct DebugListIndex<'a> {
  /// Данные, разбитые по принадлежности к отдельным спискам и без длины, для удобства отладочного вывода
  by_list: Vec<ListIndex<'a>>,
  /// Данные в том виде, в каком они хранятся в файле
  raw: &'a [u32],
}

/// Описание всей структуры GFF файла, как она хранится в GFF файле
pub struct Gff {
  /// Заголовок файла, содержащий метаинформацию о нем: тип содержимого, версию структуры,
  /// количество и местоположение структур в файле
  pub header:        Header,
  /// Список структур внутри GFF файла. Структура -- группирующий элемент, состоящий
  /// из [полей], помеченных [метками]
  ///
  /// [полей]: struct.Field.html
  /// [метками]: ../struct.Label.html
  pub structs:       Vec<Struct>,
  /// Список полей из всех структур GFF файла. Каждое поле ссылается на [метку] и данные,
  /// а также имеет некоторый тип
  ///
  /// [метку]: ../struct.Label.html
  pub fields:        Vec<Field>,
  /// Список меток из всех полей GFF файла. В корректном GFF файле каждая метка должна быть
  /// уникальна, однако неуникальность не является фатальной ошибкой -- просто неэффективным
  /// расходованием места
  pub labels:        Vec<Label>,
  /// Данные для значений полей, которые не влезают в 4 байта и не могут храниться в структуре
  /// [поля](struct.Field.html)
  pub field_data:    Vec<u8>,
  /// Плоский массив, содержащий индексы полей, которые входят в каждую структуру, содержащую
  /// более одного поля. Например, при наличии двух структур, первая из которых ссылается на поля
  /// 0 и 1, а вторая на поля 2, 3 и 4, массив может содержать `[0, 1, 2, 3, 4]` или `[2, 3, 4, 0, 1]`,
  /// в зависимости от того, в каком порядке будут записаны структуры
  pub field_indices: Vec<u32>,
  /// Плоский массив индексов структуры, которые входят в списки. Каждый элемент массива описывает
  /// или индекс структуры, или количество следующих индексов в массиве, которые относятся к одному
  /// списку. Например, если файл содержит два поля-списка, первое из которых состоит из структур
  /// 1 и 3, а второй -- из структур 0, 2 и 4, то массив может содержать `[2, 1, 3, 3, 0, 2, 4]`
  /// или `[3, 0, 2, 4, 2, 1, 3]` в зависимости от порядка записи списков. Каждый подсписок начинается
  /// с числа, указывающего его размер: в данном примере `[2| 1, 3]` и `[3| 0, 2, 4]`
  pub list_indices:  Vec<u32>,
}

macro_rules! read_exact {
  ($reader:expr, $section:expr, $type:ident) => ({
    $reader.seek(SeekFrom::Start($section.offset as u64))?;
    let mut vec = Vec::with_capacity($section.count as usize);
    for _ in 0..$section.count {
      vec.push($type::read($reader)?);
    }
    vec
  });
}

macro_rules! read_into {
  ($reader:expr, $section:expr) => ({
    $reader.seek(SeekFrom::Start($section.offset as u64))?;
    let mut vec = Vec::with_capacity($section.count as usize);
    unsafe { vec.set_len(($section.count / 4) as usize); }
    $reader.read_u32_into::<LE>(&mut vec[..])?;
    vec
  });
}

macro_rules! write_all {
  ($writer:expr, $list:expr) => (
    for elem in &$list {
      elem.write($writer)?;
    }
  );
  ($writer:expr, $list:expr, LE) => (
    for elem in &$list {
      $writer.write_u32::<LE>(*elem)?;
    }
  );
}

impl Gff {
  /// Осуществляет чтение GFF формата из указанного источника данных
  pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Gff> {
    let header  = Header::read(reader)?;
    let structs = read_exact!(reader, header.structs, Struct);
    let fields  = read_exact!(reader, header.fields , Field);

    reader.seek(SeekFrom::Start(header.labels.offset as u64))?;
    let mut labels = Vec::with_capacity(header.labels.count as usize);
    for _ in 0..header.labels.count {
      let mut label = [0u8; 16];
      reader.read_exact(&mut label)?;
      labels.push(label.into());
    }

    reader.seek(SeekFrom::Start(header.field_data.offset as u64))?;
    let mut field_data = Vec::with_capacity(header.field_data.count as usize);
    unsafe { field_data.set_len(header.field_data.count as usize); }
    reader.read_exact(&mut field_data[..])?;

    let field_indices = read_into!(reader, header.field_indices);
    let list_indices  = read_into!(reader, header.list_indices);

    Ok(Gff { header, structs, fields, labels, field_data, field_indices, list_indices })
  }
  /// Записывает всю GFF структуру в указанный поток
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    self.header.write(writer)?;
    write_all!(writer, self.structs);
    write_all!(writer, self.fields);
    for label in &self.labels {
      writer.write_all(label.as_ref())?;
    }
    writer.write_all(&self.field_data)?;
    write_all!(writer, self.field_indices, LE);
    write_all!(writer, self.list_indices, LE);
    Ok(())
  }

  /// Разделяет плоский список с данными полей на массив, содержащий по порции данных на
  /// каждое поле. Вспомогательный массив `offsets` содержит смещения внутри массива с данными
  /// для каждого поля
  fn split_data<'a>(data: &'a [u8], offsets: &[u32]) -> DebugFieldData<'a> {
    let it1 = offsets.iter();
    let it2 = offsets.iter().skip(1);

    let mut vec = Vec::with_capacity(offsets.len() - 1);
    for (s, e) in it1.zip(it2) {
      vec.push(FieldData(&data[*s as usize .. *e as usize]));
    }
    DebugFieldData { by_field: vec, raw: data }
  }
  /// Разделяет плоский список с индексами полей структур на массив, содержащий по списку
  /// полей на каждую структуру. Вспомогательный массив `offsets` содержит для каждой структуры
  /// номер первого элемента и количество элементов
  fn split_fields<'a>(data: &'a [u32], offsets: &[(u32, u32)]) -> DebugFieldIndex<'a> {
    let mut vec = Vec::with_capacity(offsets.len() - 1);
    for (s, cnt) in offsets {
      vec.push(FieldIndex(&data[*s as usize .. (*s + cnt) as usize]));
    }
    DebugFieldIndex { by_struct: vec, raw: data }
  }
  /// Разделяет плоский список с индексами элементов списка на массив, содержащий по списку
  /// структур на каждый GFF список
  fn split_lists<'a>(data: &'a [u32]) -> DebugListIndex<'a> {
    let mut vec = Vec::new();
    let mut it = data.iter().enumerate();
    while let Some((i, cnt)) = it.next() {
      let from = i + 1;
      let to = from + *cnt as usize;
      vec.push(ListIndex(&data[from..to]));
      while let Some((j, _)) = it.next() {
        if j >= to { break; }
      }
    }
    DebugListIndex { by_list: vec, raw: data }
  }
}

impl fmt::Debug for Gff {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    let data_offsets: Vec<_> = self.fields.iter()
      // Оставляем только те поля, для которых данных хранятся в массиве field_data
      .filter(|f| FieldType::from_u32(f.tag).as_ref().map(FieldType::is_complex).unwrap_or(false))
      .map(|f| Cursor::new(f.data).read_u32::<LE>().unwrap())
      .collect();
    let field_offsets: Vec<_> = self.structs.iter()
      // Списки полей используются только для структур, которые имеют более 2-х полей
      .filter(|s| s.fields > 1)
      // Смещение указано в байтах, а нам требуется в элементах, поэтому делим на размер элемента
      .map(|s| (s.offset / 4, s.fields))
      .collect();

    f.debug_struct("Gff")
      .field("header",        &self.header)
      .field("structs",       &self.structs)
      .field("fields",        &self.fields)
      .field("labels",        &self.labels)
      .field("field_data",    &Self::split_data(&self.field_data, &data_offsets))
      .field("field_indices", &Self::split_fields(&self.field_indices, &field_offsets))
      .field("list_indices",  &Self::split_lists(&self.list_indices))
      .finish()
  }
}
