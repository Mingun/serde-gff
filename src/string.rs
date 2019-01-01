//! Содержит реализации структур, описывающих строки, хранящиеся в GFF файле
use std::fmt;
use std::mem::transmute;
use std::collections::HashMap;

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
#[repr(u32)]
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
#[repr(u32)]
pub enum Gender {
  /// Строка предназначена для персонажа мужского или неопределенного пола
  Male = 0,
  /// Строка предназначена для персонажа женского пола
  Female = 1,
}

/// Ключ, используемый для индексации локализуемых строк во внутреннем представлении
/// строк (когда строки внедрены в GFF файл, а не используются ссылки на строки в TLK
/// файле).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StringKey(pub(crate) u32);
impl StringKey {
  /// Язык, на котором записан текст этой части многоязыковой строки
  pub fn language(&self) -> Language { unsafe { transmute(self.0 >> 1) } }
  /// Пол персонажа, для которого написан текст этой части многоязыковой строки
  pub fn gender(&self) -> Gender { unsafe { transmute(self.0 % 2) } }
}
impl From<(Language, Gender)> for StringKey {
  #[inline]
  fn from(value: (Language, Gender)) -> Self {
    StringKey(((value.0 as u32) << 1) | value.1 as u32)
  }
}
/// Преобразует ключ в число, в котором он храниться в GFF файле по формуле:
/// ```rust,ignore
/// ((self.language() as u32) << 1) | self.gender() as u32
/// ```
impl Into<u32> for StringKey {
  #[inline]
  fn into(self) -> u32 { self.0 }
}

/// Часть локализованной строки, хранящая информацию для одного языка и пола
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubString {
  /// Язык, на котором записан текст этой части многоязыковой строки, и пол
  /// персонажа, для которого он написан
  pub key: StringKey,
  /// Текст многоязыковой строки для указанного пола и языка
  pub string: String,
}
impl From<(StringKey, String)> for SubString {
  #[inline]
  fn from(value: (StringKey, String)) -> Self {
    SubString { key: value.0, string: value.1 }
  }
}
impl Into<(StringKey, String)> for SubString {
  #[inline]
  fn into(self) -> (StringKey, String) {
    (self.key, self.string)
  }
}

/// Локализуемая строка, содержащая в себе все данные, которые могут храниться в GFF файле.
/// Может содержать логически некорректные данные, поэтому, если не требуется анализировать
/// непосредственное содержимое GFF файла без потерь, лучше сразу преобразовать ее в
/// [`GffString`], используя `into()`, и работать с ней.
///
/// [`GffString`]: enum.GffString.html
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocString {
  /// Индекс в TLK файле, содержащий локализованный текст
  pub str_ref: StrRef,
  /// Список локализованных строк для каждого языка и пола
  pub strings: Vec<SubString>,
}

/// Локализуемая строка, представленная в виде, в котором некорректные значения
/// непредставимы.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GffString {
  /// Внешнее представление строки в виде индекса в TLK файле, содержащем локализованный
  /// текст. В зависимости от локализации текст будет разным
  External(StrRef),
  /// Внутреннее представление строки, хранимое внутри самого файла -- по строке для каждого
  /// языка и пола персонажа
  Internal(HashMap<StringKey, String>),
}
impl From<LocString> for GffString {
  /// Преобразует вариант строки, наиболее приближенный к хранимому в файле варианту (и, таким
  /// образом, хранящий без потерь все содержимое файла) в вариант строки, в котором компилятор
  /// Rust гарантирует корректность данных -- либо ссылка на внешнюю строку, либо список строк
  /// для каждого языка и пола, причем для каждой пары существует лишь один вариант строки --
  /// последний из `LocString.strings`, если их там окажется несколько.
  ///
  /// Метод возвращает внутреннее представление, если `LocString.str_ref == StrRef(0xFFFFFFFF)`,
  /// в противном случае возвращается внешнее представление. Все строки из массива `LocString.strings`
  /// в этом случае игнорируются.
  fn from(value: LocString) -> Self {
    use self::GffString::*;

    match value.str_ref {
      StrRef(0xFFFFFFFF) => Internal(value.strings.into_iter().map(Into::into).collect()),
      _                  => External(value.str_ref),
    }
  }
}
impl From<GffString> for LocString {
  /// Преобразует представление локализованной строки, корректность данных в котором гарантируется
  /// компилятором Rust в представление, приближенное к хранимому в GFF файле.
  ///
  /// При преобразовании внешнего представления строки в `LocString.strings` записывается пустой массив.
  /// При преобразовании внутреннего представления в `LocString.str_ref` записывается `StrRef(0xFFFFFFFF)`.
  fn from(value: GffString) -> Self {
    use self::GffString::*;

    match value {
      External(str_ref) => LocString { str_ref, strings: vec![] },
      Internal(strings) => {
        let strings = strings.into_iter().map(Into::into).collect();
        LocString { str_ref: StrRef(0xFFFFFFFF), strings }
      },
    }
  }
}
