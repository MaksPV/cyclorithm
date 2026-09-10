//! Core logic for Cyclorithm: validation (E01–E09), lattice expansion,
//! ordering `(time, k, declaration order)`.

use std::fmt;

pub mod cond;
pub mod datetime;
pub mod duration;
pub mod expand;
pub mod imports;
pub mod schedule;
pub mod validate;

// ---------------------------------------------------------------------------
// Ошибка валидации: коды E01–E09 из §5 спеки.
// Печатается только `message` (примеры из таблицы спеки — без префикса кода).
// ---------------------------------------------------------------------------

/// Ошибка валидации уже разобранного AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Код из §5 (`"E01"`–`"E09"`).
    pub code: &'static str,
    /// Текст для stderr, дословно по таблице §5.
    pub message: String,
}

impl Error {
    fn coded(code: &'static str, message: String) -> Self {
        Self { code, message }
    }

    /// E05: `invalid duration '1h2h'`, переполнение, нулевой период root_cycle.
    pub fn e05(raw: &str) -> Self {
        Self::coded("E05", format!("invalid duration '{raw}'"))
    }

    /// E08: `invalid datetime '...'` (битый `start_time` или `--start`/`--end`).
    pub fn e08(raw: &str) -> Self {
        Self::coded("E08", format!("invalid datetime '{raw}'"))
    }

    /// E01: `unknown point 'PORT'`.
    pub fn e01(name: &str) -> Self {
        Self::coded("E01", format!("unknown point '{name}'"))
    }

    /// E02: `action 'arrive' not allowed for point 'DEPOT'`.
    pub fn e02(action: &str, point: &str) -> Self {
        Self::coded(
            "E02",
            format!("action '{action}' not allowed for point '{point}'"),
        )
    }

    /// E03: `unknown cycle 'NIGHT_ROUTE'`.
    pub fn e03(name: &str) -> Self {
        Self::coded("E03", format!("unknown cycle '{name}'"))
    }

    /// E04: `duplicate point 'DEPOT'` / `duplicate cycle 'R'`.
    /// `kind` — `"point"` или `"cycle"`.
    pub fn e04(kind: &str, name: &str) -> Self {
        Self::coded("E04", format!("duplicate {kind} '{name}'"))
    }

    /// E09: `point 'DEPOT' is not a cycle` (точку вызвали как цикл).
    pub fn e09_not_cycle(name: &str) -> Self {
        Self::coded("E09", format!("point '{name}' is not a cycle"))
    }

    /// E09: `cycle 'X' is not a point` (цикл вызвали как точку).
    pub fn e09_not_point(name: &str) -> Self {
        Self::coded("E09", format!("cycle '{name}' is not a point"))
    }

    /// E06: `recursive cycle 'A'`.
    pub fn e06(name: &str) -> Self {
        Self::coded("E06", format!("recursive cycle '{name}'"))
    }

    /// E07: `cycle 'CYCLE2' overruns 'CYCLE1' by 20m (80m > 60m)`.
    /// Суммы уже отформатированы (`excess`, `end`, `limit` — строки вида `20m`).
    pub fn e07_cycle(inner: &str, outer: &str, excess: &str, end: &str, limit: &str) -> Self {
        Self::coded(
            "E07",
            format!("cycle '{inner}' overruns '{outer}' by {excess} ({end} > {limit})"),
        )
    }

    /// E07 для действия точки (формат спеки задан только для циклов;
    /// сообщение симметрично: `action 'depart' overruns 'C' by 1m (61m > 60m)`).
    pub fn e07_action(action: &str, outer: &str, excess: &str, end: &str, limit: &str) -> Self {
        Self::coded(
            "E07",
            format!("action '{action}' overruns '{outer}' by {excess} ({end} > {limit})"),
        )
    }

    /// E07 для отрицательного смещения ниже нуля (§4 спеки):
    /// `offset '-2h' out of bounds (duration 1h20m)`.
    /// `offset_raw` — сырой текст со знаком (`'-2h'`), `duration_raw` — сырой
    /// текст объявленной длительности объемлющего цикла.
    pub fn e07_neg_offset(offset_raw: &str, duration_raw: &str) -> Self {
        Self::coded(
            "E07",
            format!("offset '{offset_raw}' out of bounds (duration {duration_raw})"),
        )
    }

    /// E07: горизонт `until` вне `[0, duration]`.
    pub fn e07_until(until_raw: &str, duration_raw: &str) -> Self {
        Self::coded(
            "E07",
            format!("until '{until_raw}' out of bounds (duration {duration_raw})"),
        )
    }

    /// E10: `repeat 0` и невлезающее в `u64` число.
    pub fn e10_repeat_count(raw: &str) -> Self {
        Self::coded("E10", format!("invalid repeat count '{raw}'"))
    }

    /// E10: `fill` по циклу нулевой длительности.
    pub fn e10_fill_zero(name: &str) -> Self {
        Self::coded("E10", format!("fill of zero-duration cycle '{name}'"))
    }

    /// E10: повтор действия точки (повторы только для циклов).
    pub fn e10_repeat_action(action: &str) -> Self {
        Self::coded(
            "E10",
            format!("repeat of point action '{action}' not allowed"),
        )
    }

    /// E11: неизвестное имя в условии.
    pub fn e11(name: &str) -> Self {
        Self::coded("E11", format!("unknown name '{name}'"))
    }

    /// E12: смешение числа и строки в условии.
    pub fn e12_mismatch() -> Self {
        Self::coded(
            "E12",
            "type mismatch: cannot mix number and string".to_owned(),
        )
    }

    /// E12: неверное число аргументов вызова в условии.
    pub fn e12_arity(name: &str) -> Self {
        Self::coded("E12", format!("wrong arguments for '{name}'"))
    }

    /// E12: деление на ноль в условии.
    pub fn e12_divzero() -> Self {
        Self::coded("E12", "division by zero".to_owned())
    }

    /// E12: рекурсивное определение.
    pub fn e12_recursive(name: &str) -> Self {
        Self::coded("E12", format!("recursive definition '{name}'"))
    }

    /// E12: вызов не-предиката в позиции условия.
    pub fn e12_not_pred(name: &str) -> Self {
        Self::coded("E12", format!("'{name}' is not a predicate"))
    }

    /// E12: число вне диапазона `i64` в условии.
    pub fn e12_range(raw: &str) -> Self {
        Self::coded("E12", format!("integer out of range '{raw}'"))
    }

    /// E12: кривой литерал даты в условии.
    pub fn e12_date(raw: &str) -> Self {
        Self::coded("E12", format!("invalid date '{raw}'"))
    }

    /// E15: `duplicate attribute 'a'` (дубль ключа в литерале мапы
    /// или в блоке действий).
    pub fn e15(name: &str) -> Self {
        Self::coded("E15", format!("duplicate attribute '{name}'"))
    }

    /// E12: доступ к отсутствующему полю (`subj.name`), к полю не-мапы
    /// и индекс не-массива — всё `unknown field 'name'`.
    pub fn e12_field(name: &str) -> Self {
        Self::coded("E12", format!("unknown field '{name}'"))
    }

    /// E12: индекс за границами массива (`tags[5]`, отрицательный `tags[-1]`).
    pub fn e12_index(raw: &str) -> Self {
        Self::coded("E12", format!("index out of bounds '{raw}'"))
    }

    /// E12: сравнение мап/массивов (`==`/`!=` между ними — «пока», см. черновик).
    pub fn e12_map_cmp() -> Self {
        Self::coded("E12", "cannot compare maps or arrays".to_owned())
    }

    /// E13: импорт не читается.
    pub fn e13_read(path: &str) -> Self {
        Self::coded("E13", format!("cannot read import '{path}'"))
    }

    /// E13: цикл импорта.
    pub fn e13_cycle(path: &str) -> Self {
        Self::coded("E13", format!("import cycle '{path}'"))
    }

    /// E14: расписание внутри импорта.
    pub fn e14_schedule(path: &str) -> Self {
        Self::coded("E14", format!("schedule not allowed in import '{path}'"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}
