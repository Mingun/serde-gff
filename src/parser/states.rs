//! Состояния парсера GFF-формата

use std::io::{Read, Seek};
use index::{FieldIndex, FieldIndicesIndex, LabelIndex, ListIndicesIndex, StructIndex};
use error::{Error, Result};
use parser::{Parser, Token, Tag};
use self::State::*;

/// Возможные состояния, в которых может находиться парсер
#[derive(Debug, Clone)]
pub enum State {
  /// Состояние, из которого начинается разбор GFF-файла.
  /// Переход из данного состояния генерирует токен [`RootBegin`].
  ///
  /// [`RootBegin`]: ../struct.Token.html#variant.RootBegin
  Start(ReadStruct<Root>),
  /// Состояние, в котором читается метка поля структуры и идет подготовка к чтению значения.
  /// Переход из данного состояния генерирует токен [`Label`].
  ///
  /// [`Label`]: ../struct.Token.html#variant.Label
  ReadLabel(ReadLabel),
  /// Состояние, в котором читается одно из полей структуры.
  /// Переход из данного состояния генерирует токены [`Value`], [`StructBegin`] и [`ListBegin`].
  ///
  /// [`Value`]: ../struct.Token.html#variant.Value
  /// [`StructBegin`]: ../struct.Token.html#variant.StructBegin
  /// [`ListBegin`]: ../struct.Token.html#variant.ListBegin
  ReadField(ReadField),
  /// Состояние, в котором по цепочке читаются все поля структуры, одно за другим.
  /// Парсер остается в данном состоянии, пока в структуре есть поля для чтения.
  ///
  /// Выход из данного состояния производится после того, как все поля будут прочитаны.
  ReadFields(ReadFields),
  /// Состояние, в котором по цепочке читаются все элементы списка, один за другим.
  /// Парсер остается в данном состоянии, пока в списке есть элементы для чтения.
  ///
  /// Выход из данного состояния производится после того, как все элементы будут прочитаны.
  /// Переход из данного состояния генерирует токены [`ItemBegin`] и [`ListEnd`].
  ///
  /// [`ItemBegin`]: ../struct.Token.html#variant.ItemBegin
  /// [`ListEnd`]: ../struct.Token.html#variant.ListEnd
  ReadItems(ReadItems),

  EndRoot(EndStruct<Root>),
  EndItem(EndStruct<Item>),
  EndStruct(EndStruct<Struct>),
  /// Состояние, на котором заканчивается разбор GFF-файла.
  /// Дальнейшие попытки сменить состояние ни к чему не приводят.
  Finish,
}
impl State {
  /// Преобразует данное состояние в следующее, поглощая необходимые для этого данные
  /// из `parser`. В случае успеха возвращает полученный из считывателя токен и следующее
  /// состояние.
  pub fn next<R: Read + Seek>(self, parser: &mut Parser<R>) -> Result<(Token, State)> {
    match self {
      Start(state)      => state.next(parser),
      ReadLabel(state)  => state.next(parser),
      ReadField(state)  => state.next(parser),
      ReadFields(state) => state.next(parser),
      ReadItems(state)  => state.next(parser),
      EndRoot(state)    => state.next(),
      EndStruct(state)  => state.next(),
      EndItem(state)    => state.next(),
      Finish => Err(Error::ParsingFinished),
    }
  }
}
impl Default for State {
  fn default() -> Self {
    State::Start(ReadStruct::<Root>::default())
  }
}
//--------------------------------------------------------------------------------------------------
pub trait TokenEmitter {
  /// Производит токен, открывающий структуру
  ///
  /// # Параметры
  /// - `tag`: Уникальный идентификатор типа структуры
  /// - `count`: Количество полей внутри данной структуры
  fn begin(&self, tag: Tag, count: u32) -> Token;
  /// Производит токен, закрывающий структуру
  fn end(&self) -> Token;
  /// Возвращает завершающее состояние, в которое необходимо перейти после испускания
  /// последнего токена
  fn next(self, state: Box<State>) -> State;
}

/// Корневая структура, представляющая весь GFF-документ
#[derive(Debug, Clone, Copy)]
pub struct Root;
impl TokenEmitter for Root {
  fn begin(&self, tag: Tag, count: u32) -> Token {
    Token::RootBegin { tag, count }
  }
  fn end(&self) -> Token { Token::RootEnd }
  fn next(self, state: Box<State>) -> State {
    State::EndRoot(EndStruct::<Self> {
      state: state,
      data:  self,
    })
  }
}

/// Структура-поле другой структуры, имеющая метку с названием поля
#[derive(Debug, Clone, Copy)]
pub struct Struct;
impl TokenEmitter for Struct {
  fn begin(&self, tag: Tag, count: u32) -> Token {
    Token::StructBegin { tag, count }
  }
  fn end(&self) -> Token { Token::StructEnd }
  fn next(self, state: Box<State>) -> State {
    State::EndStruct(EndStruct::<Self> {
      state: state,
      data:  self,
    })
  }
}

/// Структура-элемент списка
#[derive(Debug, Clone, Copy)]
pub struct Item {
  /// Порядковый номер элемента в списке
  index: u32,
}
impl TokenEmitter for Item {
  fn begin(&self, tag: Tag, count: u32) -> Token {
    Token::ItemBegin { tag, count, index: self.index }
  }
  fn end(&self) -> Token { Token::ItemEnd }
  fn next(self, state: Box<State>) -> State {
    State::EndItem(EndStruct::<Self> {
      state: state,
      data:  self,
    })
  }
}

/// Состояние для чтения одной указанной в индексе структуры данных.
///
/// В состоянии осуществляется переход к месту хранения структуры в файле,
/// чтение индекса поля или списка полей и переход в состояние [`ReadField`],
/// если поле одно, или [`ReadFields`], если их несколько.
#[derive(Debug, Clone)]
pub struct ReadStruct<Data: TokenEmitter> {
  /// Индекс структуры, которую необходимо прочитать
  index: StructIndex,
  /// Состояние, в которое нужно вернуться
  state: Box<State>,
  /// Дополнительные данные
  data: Data,
}
impl<Data: TokenEmitter> ReadStruct<Data> {
  /// # Возвращаемое значение
  /// Возвращает генерируемый в процессе разбора токен и новое состояние парсера
  fn next<R: Read + Seek>(self, parser: &mut Parser<R>) -> Result<(Token, State)> {
    // Переходим к структуре в списке структур и читаем его
    parser.seek(self.index)?;
    let tag   = parser.read_u32()?;
    let index = parser.read_u32()?;
    let count = parser.read_u32()?;

    let token = self.data.begin(Tag(tag), count);
    let next  = self.data.next(self.state);
    let state = match count {
      0 => next,
      1 => State::ReadLabel(ReadLabel { index: FieldIndex(index), state: next.into() }),
      _ => State::ReadFields(ReadFields { index: FieldIndicesIndex(index, 0), count, state: next.into() }),
    };

    Ok((token, state))
  }
}
impl Default for ReadStruct<Root> {
  fn default() -> Self {
    ReadStruct::<Root> {
      index: StructIndex(0),
      state: Finish.into(),
      data:  Root,
    }
  }
}
//--------------------------------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct EndStruct<Data: TokenEmitter> {
  /// Состояние, в которое нужно вернуться
  state: Box<State>,
  /// Дополнительные данные
  data: Data,
}
impl<Data: TokenEmitter> EndStruct<Data> {
  fn next(self) -> Result<(Token, State)> {
    Ok((self.data.end(), *self.state))
  }
}
//--------------------------------------------------------------------------------------------------
/// Состояние чтения метки поля. Осуществляет переход к нужному полю, чтение метки и типа значения,
/// затем переход в состояние чтения значения
#[derive(Debug, Clone)]
pub struct ReadLabel {
  /// Индекс поля, которое необходимо прочитать
  index: FieldIndex,
  /// Состояние, в которое нужно вернуться
  state: Box<State>,
}
impl ReadLabel {
  /// # Возвращаемое значение
  /// Возвращает генерируемый в процессе разбора токен и новое состояние парсера
  fn next<R: Read + Seek>(self, parser: &mut Parser<R>) -> Result<(Token, State)> {
    // Переходим к полю в списке полей и читаем его
    parser.seek(self.index)?;
    let tag   = parser.read_u32()?;
    let label = LabelIndex(parser.read_u32()?);

    let token = Token::Label(label);
    let state = ReadField { tag, state: self.state };

    Ok((token, State::ReadField(state)))
  }
}
/// Состояние чтения значения поля. В зависимости от типа значения возвращает токен
/// простого значения, начала списка или структуры
#[derive(Debug, Clone)]
pub struct ReadField {
  /// Идентификатор типа поля, которое требуется прочитать
  tag: u32,
  /// Состояние, в которое нужно вернуться
  state: Box<State>,
}
impl ReadField {
  /// # Возвращаемое значение
  /// Возвращает генерируемый в процессе разбора токен и новое состояние парсера
  fn next<R: Read + Seek>(self, parser: &mut Parser<R>) -> Result<(Token, State)> {
    match self.tag {
      14 => {// Структура
        let next = ReadStruct::<Struct> {
          index: StructIndex(parser.read_u32()?),
          state: self.state,
          data:  Struct,
        };
        next.next(parser)
      },
      15 => {// Список элементов
        let next = ReadList {
          index: ListIndicesIndex(parser.read_u32()?, 0),
          state: self.state,
        };
        next.next(parser)
      },
      _ => {
        let value = parser.read_value_ref(self.tag)?;
        let token = Token::Value(value);

        Ok((token, *self.state))
      },
    }
  }
}
//--------------------------------------------------------------------------------------------------
/// Состояние чтения списка полей. Осуществляет переход к индексу списка и чтение поля
#[derive(Debug, Clone)]
pub struct ReadFields {
  /// Индекс поля, которое необходимо прочитать
  index: FieldIndicesIndex,
  /// Количество полей, которое нужно прочитать
  count: u32,
  /// Состояние, в которое нужно вернуться
  state: Box<State>,
}
impl ReadFields {
  /// # Возвращаемое значение
  /// Возвращает генерируемый в процессе разбора токен и новое состояние парсера
  fn next<R: Read + Seek>(self, parser: &mut Parser<R>) -> Result<(Token, State)> {
    if self.count == 0 {
      return self.state.next(parser);
    }
    // Переходим к индексу в таблице индексов полей структур и читаем его
    parser.seek(self.index)?;
    let field = parser.read_u32()?;

    let state = ReadLabel {
      index: FieldIndex(field),
      state: State::ReadFields(ReadFields {
        index: self.index + 1,
        count: self.count - 1,
        state: self.state,
      }).into(),
    };

    state.next(parser)
  }
}
//--------------------------------------------------------------------------------------------------
/// Псевдо-состояние для чтения указанного списка элементов.
///
/// Не является полноценным состоянием, т.к. парсер никогда в нем не останавливается.
/// Выделено в отдельную структуру просто для упрощения кода. После завершения чтения
/// идет переход в заранее указанное следующее состояние.
#[derive(Debug, Clone)]
struct ReadList {
  /// Индекс в таблице индексов, содержащий структуру-элемент для чтения
  index: ListIndicesIndex,
  /// Состояние, в которое нужно перейти после завершения чтения списка
  state: Box<State>,
}
impl ReadList {
  /// # Возвращаемое значение
  /// Возвращает генерируемый в процессе разбора токен и новое состояние парсера
  fn next<R: Read + Seek>(self, parser: &mut Parser<R>) -> Result<(Token, State)> {
    // Переходим к списку индексов структур-элементов списка и читаем его размер
    parser.seek(self.index)?;
    let count = parser.read_u32()?;

    // Сообщаем о начале списка и переходим в состояние чтения первого элемента
    let token = Token::ListBegin(count);
    let state = ReadItems {
      index: self.index + 1,
      count: count,
      state: self.state,
    };

    Ok((token, State::ReadItems(state)))
  }
}
//--------------------------------------------------------------------------------------------------
/// Подготовительное состояние для чтения элемента списка. Читает из файла индекс
/// структуры-элемента и переходит в состояние его чтения.
#[derive(Debug, Clone)]
pub struct ReadItems {
  /// Индекс в таблице индексов, содержащий структуру-элемент для чтения
  index: ListIndicesIndex,
  /// Количество элементов, которое нужно прочитать
  count: u32,
  /// Состояние, в которое нужно перейти после завершения чтения списка
  state: Box<State>,
}
impl ReadItems {
  /// # Возвращаемое значение
  /// Возвращает генерируемый в процессе разбора токен и новое состояние парсера
  fn next<R: Read + Seek>(self, parser: &mut Parser<R>) -> Result<(Token, State)> {
    // Если весь список прочитан, сообщаем об окончании списка и возвращаемся
    // в состояние, из которого начали читать список
    if self.count == 0 {
      return Ok((Token::ListEnd, *self.state));
    }
    // Переходим к индексу в таблице индексов элементов списков и читаем его
    parser.seek(self.index)?;
    let struc = parser.read_u32()?;

    let state = ReadStruct::<Item> {
      index: StructIndex(struc),
      state: State::ReadItems(ReadItems {
        index: self.index + 1,
        count: self.count - 1,
        state: self.state,
      }).into(),
      data: Item { index: self.index.1 },
    };

    state.next(parser)
  }
}
