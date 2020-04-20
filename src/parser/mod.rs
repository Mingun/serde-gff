//! Реализация потокового парсера GFF файла. См. описание структуры [`Parser`](struct.Parser.html)

use std::iter::FusedIterator;
use std::io::{Read, Seek, SeekFrom};
use byteorder::{LE, ReadBytesExt};
use encoding::{EncodingRef, DecoderTrap};
use encoding::all::UTF_8;

use crate::{Label, SubString, ResRef, StrRef};
use crate::error::{Error, Result};
use crate::header::Header;
use crate::index::{Index, LabelIndex, U64Index, I64Index, F64Index, StringIndex, ResRefIndex, LocStringIndex, BinaryIndex};
use crate::string::{LocString, StringKey};
use crate::value::{SimpleValue, SimpleValueRef};

mod token;
mod states;

use self::states::State;
pub use self::token::Token;

/// Уникальный идентификатор типа структуры, хранимой в GFF-файле
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tag(u32);

/// Реализует потоковый (наподобие SAX) парсер GFF файла. Парсер реализует интерфейс
/// итератора по [токенам]. Каждый вызов метода [`next_token`] возвращает следующий токен
/// из потока, который сразу же может быть использован для анализа или сохранен для
/// дальнейшего использования.
///
/// # События разбора
/// Парсер представляет собой pull-down парсер, т.е. для получения данных его нужно опрашивать внешним
/// циклом (в противоположность push-down парсеру, который испускает события при разборе очередного
/// элемента).
///
/// Так как GFF файл может быть представлен в XML виде, и эта структура проще для представления в тексте,
/// то ниже показан пример файла, в котором отмечены места после которых парсер генерирует токены при
/// разборе. В виде кода Rust описанная структура данных может быть представлена таким образом:
///
/// ```rust,no_run
/// struct Struct;
/// struct Item {
///   double: f64,
/// }
/// struct Root {
///   int: i32,
///   struc: Struct,
///   list: Vec<Item>,
/// }
/// ```
/// XML представление:
/// ```xml
/// <STRUCT tag='4294967295'>[1]
///   <FIELD label='int'[2] type='INT'>8</FIELD>[3]
///   <FIELD label='struc'[4] type='STRUCT'>
///     <STRUCT tag='1'>[5]
///     </STRUCT>[6]
///   </FIELD>
///   <FIELD label='list'[7] type='LIST'>[8]
///     <STRUCT tag='2'>[9]
///       <FIELD label='double'[10] type='DOUBLE'>0.000000</FIELD>[11]
///     </STRUCT>[12]
///   </FIELD>[13]
/// </STRUCT>[14]
/// ```
/// Токены, получаемые последовательным вызовом [`next_token`]:
/// 1. [`RootBegin`]. Прочитано описание корневой структуры -- в этом состоянии уже известен
///    тег типа структуры и количество полей в ней.
/// 2. [`Label`]. Прочитан индекс метки, по этому индексу может быть прочитано значение метки
/// 3. [`Value`]. Прочитано примитивное значение
/// 4. [`Label`]. Прочитан индекс метки, по этому индексу может быть прочитано значение метки
/// 5. [`StructBegin`]. Прочитано количество полей структуры и ее тег
/// 6. [`StructEnd`]
/// 7. [`Label`]. Прочитан индекс метки, по этому индексу может быть прочитано значение метки
/// 8. [`ListBegin`]. Прочитано количество элементов списка
/// 9. [`ItemBegin`]. Прочитано количество полей структуры, ее тег, а также предоставляется
///    информация о порядковом индексе элемента
/// 10. [`Label`]. Прочитан индекс метки, по этому индексу может быть прочитано значение метки
/// 11. [`Value`]. Прочитан индекс большого значения (больше 4-х байт), по этому индексу само
///     значение может быть прочитано отдельным вызовом
/// 12. [`ItemEnd`]. Элемент списка прочитан
/// 13. [`ListEnd`]. Весь список прочитан
/// 14. [`RootEnd`]. Файл прочитан
///
/// # Пример
/// В данном примере читается файл с диска, и в потоковом режиме выводится на экран, формируя
/// что-то, напоминающее JSON.
///
/// ```rust
/// use std::fs::File;
/// use serde_gff::parser::Parser;
/// use serde_gff::parser::Token::*;
///
/// // Читаем файл с диска и создаем парсер. При создании парсер сразу же читает небольшую
/// // порцию данных -- заголовок, которая нужна ему для правильного разрешения ссылок
/// let file = File::open("test-data/all.gff").expect("test file not exist");
/// let mut parser = Parser::new(file).expect("reading GFF header failed");
/// let mut indent = 0;
/// loop {
///   // В данном случае мы используем методы типажа Iterator для итерирования по файлу, так
///   // как мы полагаем, что ошибок в процессе чтения не возникнет. Если же они интересны,
///   // следует использовать метод `next_token`
///   if let Some(token) = parser.next() {
///     match token {
///       RootBegin {..} | RootEnd => {},
///       // Обрамляем структуры в `{ ... }`
///       StructBegin {..} => { indent += 1; println!("{{"); },
///       StructEnd        => { indent -= 1; println!("{:indent$}}}", "", indent=indent*2); },
///       // Обрамляем списки в `[ ... ]`
///       ListBegin {..}   => { indent += 1; println!("["); },
///       ListEnd          => { indent -= 1; println!("{:indent$}]", "", indent=indent*2); },
///       // Обрамляем элементы списков в `[index]: { ... }`
///       ItemBegin { index, .. } => {
///         println!("{:indent$}[{}]: {{", "", index, indent=indent*2);
///         indent += 1;
///       },
///       ItemEnd => {
///         indent -= 1;
///         println!("{:indent$}}}", "", indent=indent*2);
///       },
///
///       Label(index) => {
///         // Физически значение меток хранится в другом месте файла. Так как при итерировании они
///         // могут быть нам неинтересны, то токен содержит только индекс используемой метки (имени
///         // поля). В данном же случае они нас интересуют, поэтому выполняем полное чтение
///         let label = parser.read_label(index).expect(&format!("can't read label {:?}", index));
///         print!("{:indent$}{}: ", "", label, indent=indent*2)
///       },
///
///       // Аналогично со значениями. Некоторые значения доступны сразу (те, чей размер не превышает
///       // 4 байта), другие хранятся с других частях файла и должны быть явно прочитаны.
///       // Также, если вас интересует только какое-то конкретное значение, может быть использован
///       // один из методов `read_*` парсера
///       Value(value) => println!("{:?}", parser.read_value(value).expect("can't read value")),
///     }
///     continue;
///   }
///   // Как только итератор возвращает None, файл закончился, либо произошла ошибка; завершаем работу
///   break;
/// }
/// ```
///
/// [токенам]: enum.Token.html
/// [`next_token`]: struct.Parser.html#method.next_token
/// [`RootBegin`]: enum.Token.html#variant.RootBegin
/// [`RootEnd`]: enum.Token.html#variant.RootEnd
/// [`StructBegin`]: enum.Token.html#variant.StructBegin
/// [`StructEnd`]: enum.Token.html#variant.StructEnd
/// [`ListBegin`]: enum.Token.html#variant.ListBegin
/// [`ListEnd`]: enum.Token.html#variant.ListEnd
/// [`ItemBegin`]: enum.Token.html#variant.ItemBegin
/// [`ItemEnd`]: enum.Token.html#variant.ItemEnd
/// [`Label`]: enum.Token.html#variant.Label
/// [`Value`]: enum.Token.html#variant.Value
pub struct Parser<R: Read + Seek> {
  /// Источник данных для чтения элементов GFF-файла
  reader: R,
  /// Заголовок GFF файла, содержащий информацию о местоположении различных секций файла
  header: Header,
  /// Кодировка, используемая для декодирования строк
  encoding: EncodingRef,
  /// Способ обработки ошибок декодирования строк
  trap: DecoderTrap,
  /// Текущее состояние разбора
  state: State,
}

impl<R: Read + Seek> Parser<R> {
  /// Создает парсер для чтения GFF файла из указанного источника данных с использованием
  /// кодировки `UTF-8` для декодирования строк и генерацией ошибки в случае, если декодировать
  /// набор байт, как строку в этой кодировке, не удалось.
  ///
  /// # Параметры
  /// - `reader`: Источник данных для чтения файла
  pub fn new(reader: R) -> Result<Self> {
    Self::with_encoding(reader, UTF_8, DecoderTrap::Strict)
  }
  /// Создает парсер для чтения GFF файла из указанного источника данных с использованием
  /// указанной кодировки для декодирования строк.
  ///
  /// # Параметры
  /// - `reader`: Источник данных для чтения файла
  /// - `encoding`: Кодировка для декодирования символов в строках
  /// - `trap`: Способ обработки символов в строках, которые не удалось декодировать с
  ///   использованием выбранной кодировки
  pub fn with_encoding(mut reader: R, encoding: EncodingRef, trap: DecoderTrap) -> Result<Self> {
    let header = Header::read(&mut reader)?;

    Ok(Parser { header, reader, encoding, trap, state: State::default() })
  }
  /// Возвращает следующий токен или ошибку, если данных не осталось или при их чтении возникли
  /// проблемы.
  pub fn next_token(&mut self) -> Result<Token> {
    let (token, next) = self.state.clone().next(self)?;
    self.state = next;
    Ok(token)
  }
  /// Быстро пропускает всю внутреннюю структуру, переводя парсер в состояние, при котором
  /// вызов [`next_token`] вернет следующий структурный элемент после пропущенного (следующее
  /// поле структуры или элемент списка).
  ///
  /// # Параметры
  /// - `token`: Токен, полученный предшествующим вызовом [`next_token`]
  ///
  /// [`next_token`]: #method.next_token
  #[inline]
  pub fn skip_next(&mut self, token: Token) {
    self.state = self.state.clone().skip(token);
  }
//-------------------------------------------------------------------------------------------------
// Завершение чтения комплексных данных
//-------------------------------------------------------------------------------------------------
  /// Читает из файла значение метки по указанному индексу.
  /// Не меняет позицию чтения в файле
  pub fn read_label(&mut self, index: LabelIndex) -> Result<Label> {
    let old = self.offset()?;
    self.seek(index)?;

    let mut label = [0u8; 16];
    self.reader.read_exact(&mut label)?;

    self.reader.seek(old)?;
    Ok(label.into())
  }
  /// Читает из файла значение поля по указанному индексу. Побочный эффект -- переход по указанному адресу
  pub fn read_u64(&mut self, index: U64Index) -> Result<u64> {
    self.seek(index)?;
    self.reader.read_u64::<LE>().map_err(Into::into)
  }
  /// Читает из файла значение поля по указанному индексу. Побочный эффект -- переход по указанному адресу
  pub fn read_i64(&mut self, index: I64Index) -> Result<i64> {
    self.seek(index)?;
    self.reader.read_i64::<LE>().map_err(Into::into)
  }
  /// Читает из файла значение поля по указанному индексу. Побочный эффект -- переход по указанному адресу
  pub fn read_f64(&mut self, index: F64Index) -> Result<f64> {
    self.seek(index)?;
    self.reader.read_f64::<LE>().map_err(Into::into)
  }
  /// Читает 4 байта длины и следующие за ними байты строки, интерпретирует их в соответствии с
  /// кодировкой декодера и возвращает полученную строку. Побочный эффект -- переход по указанному адресу
  pub fn read_string(&mut self, index: StringIndex) -> Result<String> {
    self.seek(index)?;
    self.read_string_impl()
  }
  /// Читает 1 байт длины и следующие за ними байты массива, возвращает прочитанный массив,
  /// обернутый в `ResRef`. Побочный эффект -- переход по указанному адресу
  pub fn read_resref(&mut self, index: ResRefIndex) -> Result<ResRef> {
    self.seek(index)?;

    let size = self.reader.read_u8()? as usize;
    let mut bytes = Vec::with_capacity(size);
    unsafe { bytes.set_len(size); }

    self.reader.read_exact(&mut bytes)?;
    Ok(ResRef(bytes))
  }
  /// Читает из файла значение поля по указанному индексу. Побочный эффект -- переход по указанному адресу
  pub fn read_loc_string(&mut self, index: LocStringIndex) -> Result<LocString> {
    self.seek(index)?;

    let _total_size = self.read_u32()?;
    let str_ref     = StrRef(self.read_u32()?);
    let count       = self.read_u32()?;

    let mut strings = Vec::with_capacity(count as usize);
    for _i in 0..count {
      strings.push(self.read_substring()?);
    }

    Ok(LocString { str_ref, strings })
  }
  /// Читает 4 байта длины и следующие за ними байты массива, возвращает прочитанный массив.
  /// Побочный эффект -- переход по указанному адресу
  pub fn read_byte_buf(&mut self, index: BinaryIndex) -> Result<Vec<u8>> {
    self.seek(index)?;
    self.read_bytes()
  }
  /// Если `value` содержит еще не прочитанные поля (т.е. содержащие [индексы]), читает их.
  /// В противном случае просто преобразует тип значения в `SimpleValue`.
  ///
  /// Данный метод меняет внутреннюю позицию чтения парсера, однако это не несет за собой
  /// негативных последствий, если сразу после вызова данного метода выполнить переход к
  /// следующему токену при итерации по токенам парсера. См. пример в описании структуры
  /// [`Parser`].
  ///
  /// [индексы]: ../index/trait.Index.html
  /// [`Parser`]: struct.Parser.html
  pub fn read_value(&mut self, value: SimpleValueRef) -> Result<SimpleValue> {
    use self::SimpleValueRef::*;

    Ok(match value {
      Byte(val)     => SimpleValue::Byte(val),
      Char(val)     => SimpleValue::Char(val),
      Word(val)     => SimpleValue::Word(val),
      Short(val)    => SimpleValue::Short(val),
      Dword(val)    => SimpleValue::Dword(val),
      Int(val)      => SimpleValue::Int(val),
      Dword64(val)  => SimpleValue::Dword64(self.read_u64(val)?),
      Int64(val)    => SimpleValue::Int64(self.read_i64(val)?),
      Float(val)    => SimpleValue::Float(val),
      Double(val)   => SimpleValue::Double(self.read_f64(val)?),
      String(val)   => SimpleValue::String(self.read_string(val)?),
      ResRef(val)   => SimpleValue::ResRef(self.read_resref(val)?),
      LocString(val)=> SimpleValue::LocString(self.read_loc_string(val)?),
      Void(val)     => SimpleValue::Void(self.read_byte_buf(val)?),
    })
  }
//-------------------------------------------------------------------------------------------------
  /// Позиционирует нижележащий считыватель в место, указуемое данным индексом данных GFF.
  /// Возвращает старую позицию в файле, для того, чтобы можно было затем вернуться в нее.
  #[inline]
  fn seek<I: Index>(&mut self, index: I) -> Result<()> {
    let offset = index.offset(&self.header);
    self.reader.seek(SeekFrom::Start(offset))?;
    Ok(())
  }
  /// Получает текущую позицию в файле
  #[inline]
  fn offset(&mut self) -> Result<SeekFrom> {
    Ok(SeekFrom::Start(self.reader.seek(SeekFrom::Current(0))?))
  }
//-------------------------------------------------------------------------------------------------
// Чтение вспомогательных данных
//-------------------------------------------------------------------------------------------------
  /// Читает 4 байта из текущей позиции и интерпретирует их, как беззнаковое целое
  #[inline]
  fn read_u32(&mut self) -> Result<u32> {
    Ok(self.reader.read_u32::<LE>()?)
  }
//-------------------------------------------------------------------------------------------------
// Чтение значений
//-------------------------------------------------------------------------------------------------
  /// Читает 4 байта длины и следующие за ними байты массива, возвращает прочитанный массив
  #[inline]
  fn read_bytes(&mut self) -> Result<Vec<u8>> {
    let size = self.read_u32()? as usize;
    let mut bytes = Vec::with_capacity(size);
    unsafe { bytes.set_len(size); }

    self.reader.read_exact(&mut bytes)?;
    Ok(bytes)
  }
  /// Читает 4 байта длины и следующие за ними байты строки, интерпретирует их в соответствии с
  /// кодировкой декодера и возвращает полученную строку
  #[inline]
  fn read_string_impl(&mut self) -> Result<String> {
    let bytes = self.read_bytes()?;

    Ok(self.encoding.decode(&bytes, self.trap)?)
  }
  #[inline]
  fn read_substring(&mut self) -> Result<SubString> {
    Ok(SubString {
      key   : StringKey(self.read_u32()?),
      string: self.read_string_impl()?,
    })
  }
  /// Читает из потока примитивное значение в соответствии с указанным тегом
  ///
  /// # Параметры
  /// - `tag`: Вид данных, которые требуется прочитать. Известные данные расположены в
  ///   диапазоне `[0; 13]`, для остальных значений возвращается ошибка [`Error::UnknownValue`]
  ///
  /// # Возвращаемое значение
  /// Возвращает лениво читаемое значение. Если данные хранятся непосредственно за тегом, то
  /// они будут уже прочитаны, в противном случае читается только адрес их местонахождения в файле.
  /// Таким образом, если данные не нужны, лишних чтений не будет
  ///
  /// [`Error::UnknownValue`]: ../../error/enum.Error.html#variant.UnknownValue
  fn read_value_ref(&mut self, tag: u32) -> Result<SimpleValueRef> {
    use self::SimpleValueRef::*;

    let value = match tag {
      0 => Byte (self.reader.read_u8()?),
      1 => Char (self.reader.read_i8()?),
      2 => Word (self.reader.read_u16::<LE>()?),
      3 => Short(self.reader.read_i16::<LE>()?),
      4 => Dword(self.reader.read_u32::<LE>()?),
      5 => Int  (self.reader.read_i32::<LE>()?),
      8 => Float(self.reader.read_f32::<LE>()?),

      6 => Dword64   (U64Index(self.read_u32()?)),
      7 => Int64     (I64Index(self.read_u32()?)),
      9 => Double    (F64Index(self.read_u32()?)),
      10 => String   (StringIndex(self.read_u32()?)),
      11 => ResRef   (ResRefIndex(self.read_u32()?)),
      12 => LocString(LocStringIndex(self.read_u32()?)),
      13 => Void     (BinaryIndex(self.read_u32()?)),
      tag => return Err(Error::UnknownValue { tag, value: self.read_u32()? }),
    };
    Ok(value)
  }
}

impl<R: Read + Seek> Iterator for Parser<R> {
  type Item = Token;

  fn next(&mut self) -> Option<Token> {
    if let State::Finish = self.state {
      return None;
    }
    let res = self.next_token();
    if let Err(Error::ParsingFinished) = res {
      return None;
    }
    Some(res.expect("Can't read token"))
  }

  #[inline]
  fn size_hint(&self) -> (usize, Option<usize>) {
    (self.header.token_count(), None)
  }
}

impl<R: Read + Seek> FusedIterator for Parser<R> {}
