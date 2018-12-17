//! Реализация потокового парсера GFF файла

use std::iter::FusedIterator;
use std::io::{Read, Seek, SeekFrom};
use std::mem::transmute;
use byteorder::{LE, ReadBytesExt};
use encoding::{EncodingRef, DecoderTrap};
use encoding::all::UTF_8;

use {Label, SubString, ResRef, StrRef};
use error::{Error, Result};
use header::Header;
use index::{Index, LabelIndex, U64Index, I64Index, F64Index, StringIndex, ResRefIndex, LocStringIndex, BinaryIndex};
use string::LocString;
use value::SimpleValueRef;

mod token;
mod states;

use self::states::State;
pub use self::token::Token;

/// Уникальный идентификатор типа структуры, хранимой в GFF-файле
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tag(u32);

/// Реализует потоковый (наподобие SAX) парсер GFF файла
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
    let id = self.read_u32()?;

    Ok(SubString {
      // Оба перечисления имеют С представление, поэтому transmute безопасен
      gender  : unsafe { transmute(id %  2) },
      language: unsafe { transmute(id >> 2) },
      string  : self.read_string_impl()?
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
