//! Реализация парсера файлов формата Bioware GFF, используемых в играх на движке Aurora
//! (Neverwinter Nights, The Witcher) и в игре Neverwinter Nights 2.
#![warn(missing_docs)]
extern crate byteorder;

// Модули описания заголовка
mod sig;
mod ver;
pub mod header;
