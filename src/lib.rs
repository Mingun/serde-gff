//! Реализация парсера файлов формата Bioware GFF, используемых в играх на движке Aurora
//! (Neverwinter Nights, The Witcher) и в игре Neverwinter Nights 2.
//!
//! Формат имеет некоторые ограничения:
//! - элементами верхнего уровня могут быть только структуры или перечисления Rust в unit или struct варианте
//! - имена полей структур не должны быть длиннее 16 байт в UTF-8. При нарушении при сериализации будет ошибка
//! - то же самое касается ключей карт. Кроме того, ключами могут быть только строки (`&str` или `String`)
//!
//! # Пример
//! ```rust
//! extern crate serde;
//! #[macro_use]
//! extern crate serde_derive;
//! extern crate serde_bytes;
//! extern crate serde_gff;
//!
//! use std::f32::consts::PI;
//! use std::f64::consts::E;
//! use std::io::Cursor;
//! use serde::Deserialize;
//!
//! use serde_gff::de::Deserializer;
//! use serde_gff::ser::to_vec;
//! use serde_gff::value::Value;
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct Item { u8: u8, i8: i8 }
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct Struct {
//!   f32: f32,
//!   f64: f64,
//!
//!   #[serde(with = "serde_bytes")]
//!   bytes: Vec<u8>,
//! }
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! #[allow(non_snake_case)]
//! struct Test {
//!   u16: u16,
//!   i16: i16,
//!   u32: u32,
//!   i32: i32,
//!   u64: u64,
//!   i64: i64,
//!
//!   string: String,
//!
//!   Struct: Struct,
//!   list: Vec<Item>,
//! }
//!
//! fn main() {
//!   let data = Test {
//!     u16: 1, i16: 2,
//!     u32: 3, i32: 4,
//!     u64: 5, i64: 6,
//!
//!     string: "String".into(),
//!
//!     Struct: Struct { f32: PI, f64: E, bytes: b"Vec<u8>".to_vec() },
//!     list: vec![
//!       Item { u8: 7, i8:  -8 },
//!       Item { u8: 9, i8: -10 },
//!     ],
//!   };
//!
//!   let mut vec = to_vec((*b"GFF ").into(), &data).expect("can't write data");
//!   // Важный нюанс - не забыть, что создание десериализатора читает заголовок и возвращает
//!   // Result, а не сам десериализатор, поэтому требуется распаковка результата
//!   let mut de = Deserializer::new(Cursor::new(vec)).expect("can't read GFF header");
//!   let val = Value::deserialize(&mut de).expect("can't deserialize data");
//!  
//!   println!("{:#?}", val);
//! }
//! ```
#![warn(missing_docs)]
extern crate byteorder;
extern crate encoding;
extern crate indexmap;
extern crate serde;
#[cfg(test)]
#[macro_use]
extern crate serde_derive;

// Модули описания заголовка
mod sig;
mod ver;
pub mod header;

pub mod parser;
pub mod index;
pub mod value;
pub mod error;
pub mod raw;

// Модули, чье содержимое реэкспортируется, разделено для удобства сопровождения
mod label;
mod resref;
mod string;

pub use label::*;
pub use resref::*;
pub use string::*;

// Модули для поддержки инфраструктуры serde
pub mod de;
pub mod ser;
