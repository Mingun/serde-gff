//! Содержит реализации структур, описывающих строки, хранящиеся в GFF файле
use std::fmt;

/// Маска, определяющая идентификатор строки
const USER_TLK_MASK: u32 = 0x8000_0000;

/// Индекс в файле `dialog.tlk`, содержащий локализованный текст
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrRef(pub(crate) u32);

impl StrRef {
  /// Определяет, является ли строка индексом не из основного TLK файла игры, а из TLK
  /// файла модуля. Строка является строкой из TLK файла модуля, если старший бит в ее
  /// идентификаторе взведен
  #[inline]
  pub fn is_user(&self) -> bool { self.0 & USER_TLK_MASK != 0 }

  /// Определяет индекс строки в TLK файле
  #[inline]
  pub fn code(&self) -> u32 { self.0 & !USER_TLK_MASK }
}

impl fmt::Debug for StrRef {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "code: {}, user: {}", self.code(), self.is_user())
  }
}

/// Виды языков, на которых могут храниться локализованные строки в объекте `LocString`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub enum Language {
  /// Английский язык
  English = 0,
  /// Французский язык
  French  = 1,
  /// Немецкий язык
  German  = 2,
  /// Итальянский язык
  Italian = 3,
  /// Испанский язык
  Spanish = 4,
  /// Польский язык
  Polish  = 5,
  /// Корейский язык
  Korean  = 128,
  /// Традиционный китайский
  ChineseTraditional = 129,
  /// Упрощенный китайский
  ChineseSimplified  = 130,
  /// Японский
  Japanese= 131,
}

/// Виды пола персонажа, на которых могут храниться локализованные строки в объекте `LocString`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub enum Gender {
  /// Строка предназначена для персонажа мужского или неопределенного пола
  Male = 0,
  /// Строка предназначена для персонажа женского пола
  Female = 1,
}

/// Часть локализованной строки, хранящая информацию для одного языка и пола
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubString {
  /// Язык, на котором записан текст этой части многоязыковой строки
  pub language: Language,
  /// Пол персонажа, для которого написан текст этой части многоязыковой строки
  pub gender: Gender,
  /// Текст многоязыковой строки для указанного поля и языка
  pub string: String,
}

/// Локализуемая строка
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocString {
  /// Индекс в TLK файле, содержащий локализованный текст
  pub str_ref: StrRef,
  /// Список локализованных строк для каждого языка и пола
  pub strings: Vec<SubString>,
}
