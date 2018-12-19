//! Реализация парсера файлов формата Bioware GFF, используемых в играх на движке Aurora
//! (Neverwinter Nights, The Witcher) и в игре Neverwinter Nights 2.
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
