//! Core logic for Cyclorithm: validation (slugs from the errors chapter),
//! lattice expansion, ordering `(time, k, declaration order)`.

use std::fmt;

pub mod cond;
pub mod datetime;
pub mod duration;
pub mod expand;
pub mod imports;
pub mod schedule;
pub mod validate;

// ---------------------------------------------------------------------------
// Ошибка валидации: слаг ситуации из главы ошибок вики.
// Печатается `слаг: сообщение` (сообщение — дословно по таблице главы).
// ---------------------------------------------------------------------------

/// Ошибка валидации уже разобранного AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// Слаг ситуации (`"unknown-point"`, `"cycle-overruns"` и т.п.).
    pub code: &'static str,
    /// Текст без слага, дословно по таблице главы ошибок.
    pub message: String,
}

impl Error {
    fn coded(code: &'static str, message: String) -> Self {
        Self { code, message }
    }

    /// `invalid-duration`: `invalid duration '1h2h'`, переполнение, нулевой период root_cycle.
    pub fn invalid_duration(raw: &str) -> Self {
        Self::coded("invalid-duration", format!("invalid duration '{raw}'"))
    }

    /// `invalid-datetime`: `invalid datetime '...'` (битый `start_time` или `--start`/`--end`).
    pub fn invalid_datetime(raw: &str) -> Self {
        Self::coded("invalid-datetime", format!("invalid datetime '{raw}'"))
    }

    /// `unknown-point`: `unknown point 'PORT'`.
    pub fn unknown_point(name: &str) -> Self {
        Self::coded("unknown-point", format!("unknown point '{name}'"))
    }

    /// `action-not-allowed`: `action 'arrive' not allowed for point 'DEPOT'`.
    pub fn action_not_allowed(action: &str, point: &str) -> Self {
        Self::coded(
            "action-not-allowed",
            format!("action '{action}' not allowed for point '{point}'"),
        )
    }

    /// `unknown-cycle`: `unknown cycle 'NIGHT_ROUTE'`.
    pub fn unknown_cycle(name: &str) -> Self {
        Self::coded("unknown-cycle", format!("unknown cycle '{name}'"))
    }

    /// `duplicate`: `duplicate point 'DEPOT'` / `duplicate cycle 'R'`.
    /// `kind` — `"point"` или `"cycle"`.
    pub fn duplicate(kind: &str, name: &str) -> Self {
        Self::coded("duplicate", format!("duplicate {kind} '{name}'"))
    }

    /// `wrong-kind`: `point 'DEPOT' is not a cycle` (точку вызвали как цикл).
    pub fn point_not_cycle(name: &str) -> Self {
        Self::coded("wrong-kind", format!("point '{name}' is not a cycle"))
    }

    /// `wrong-kind`: `cycle 'X' is not a point` (цикл вызвали как точку).
    pub fn cycle_not_point(name: &str) -> Self {
        Self::coded("wrong-kind", format!("cycle '{name}' is not a point"))
    }

    /// `wrong-kind`: `point 'X' is not a routine` (точку вызвали с таблицей).
    pub fn point_not_routine(name: &str) -> Self {
        Self::coded("wrong-kind", format!("point '{name}' is not a routine"))
    }

    /// `wrong-kind`: `routine 'X' is not a point` (рутину вызвали как точку).
    pub fn routine_not_point(name: &str) -> Self {
        Self::coded("wrong-kind", format!("routine '{name}' is not a point"))
    }

    /// `wrong-kind`: `routine 'X' is not a cycle` (имя занято и рутиной, и циклом).
    pub fn routine_not_cycle(name: &str) -> Self {
        Self::coded("wrong-kind", format!("routine '{name}' is not a cycle"))
    }

    /// `recursive`: `recursive cycle 'A'`.
    pub fn recursive_cycle(name: &str) -> Self {
        Self::coded("recursive", format!("recursive cycle '{name}'"))
    }

    /// `recursive`: рекурсия через рутину — `recursive routine 'M'`.
    pub fn recursive_routine(name: &str) -> Self {
        Self::coded("recursive", format!("recursive routine '{name}'"))
    }

    /// `recursive`: рекурсия через таблицу — `recursive table 'T'`
    /// (пожар `->` инстанцирует рутину с той же таблицей).
    pub fn recursive_table(name: &str) -> Self {
        Self::coded("recursive", format!("recursive table '{name}'"))
    }

    /// `cycle-overruns`: `cycle 'CYCLE2' overruns 'CYCLE1' by 20m (80m > 60m)`.
    /// Суммы уже отформатированы (`excess`, `end`, `limit` — строки вида `20m`).
    pub fn cycle_overruns(inner: &str, outer: &str, excess: &str, end: &str, limit: &str) -> Self {
        Self::coded(
            "cycle-overruns",
            format!("cycle '{inner}' overruns '{outer}' by {excess} ({end} > {limit})"),
        )
    }

    /// `action-overruns` для действия точки (формат главы задан только для циклов;
    /// сообщение симметрично: `action 'depart' overruns 'C' by 1m (61m > 60m)`).
    pub fn action_overruns(
        action: &str,
        outer: &str,
        excess: &str,
        end: &str,
        limit: &str,
    ) -> Self {
        Self::coded(
            "action-overruns",
            format!("action '{action}' overruns '{outer}' by {excess} ({end} > {limit})"),
        )
    }

    /// `offset-out-of-bounds` для отрицательного смещения ниже нуля (§4 спеки):
    /// `offset '-2h' out of bounds (duration 1h20m)`.
    /// `offset_raw` — сырой текст со знаком (`'-2h'`), `duration_raw` — сырой
    /// текст объявленной длительности объемлющего цикла.
    pub fn offset_out_of_bounds(offset_raw: &str, duration_raw: &str) -> Self {
        Self::coded(
            "offset-out-of-bounds",
            format!("offset '{offset_raw}' out of bounds (duration {duration_raw})"),
        )
    }

    /// `until-out-of-bounds`: горизонт `until` вне `[0, duration]`.
    pub fn until_out_of_bounds(until_raw: &str, duration_raw: &str) -> Self {
        Self::coded(
            "until-out-of-bounds",
            format!("until '{until_raw}' out of bounds (duration {duration_raw})"),
        )
    }

    /// `invalid-repeat-count`: `repeat 0` и невлезающее в `u64` число.
    pub fn invalid_repeat_count(raw: &str) -> Self {
        Self::coded(
            "invalid-repeat-count",
            format!("invalid repeat count '{raw}'"),
        )
    }

    /// `fill-zero-duration`: `fill` по циклу нулевой длительности.
    pub fn fill_zero_duration(name: &str) -> Self {
        Self::coded(
            "fill-zero-duration",
            format!("fill of zero-duration cycle '{name}'"),
        )
    }

    /// `repeat-point-action`: повтор действия точки (повторы только для циклов).
    pub fn repeat_point_action(action: &str) -> Self {
        Self::coded(
            "repeat-point-action",
            format!("repeat of point action '{action}' not allowed"),
        )
    }

    /// `unknown-name`: неизвестное имя в условии.
    pub fn unknown_name(name: &str) -> Self {
        Self::coded("unknown-name", format!("unknown name '{name}'"))
    }

    /// `type-mismatch`: смешение числа и строки в условии.
    pub fn type_mismatch() -> Self {
        Self::coded(
            "type-mismatch",
            "type mismatch: cannot mix number and string".to_owned(),
        )
    }

    /// `wrong-arguments`: неверное число аргументов вызова в условии.
    pub fn wrong_arguments(name: &str) -> Self {
        Self::coded("wrong-arguments", format!("wrong arguments for '{name}'"))
    }

    /// `no-table-parameter`: у рутины нет табличного параметра (`params[0]` — таблица).
    pub fn no_table_parameter(name: &str) -> Self {
        Self::coded(
            "no-table-parameter",
            format!("routine '{name}' has no table parameter"),
        )
    }

    /// `division-by-zero`: деление на ноль в условии.
    pub fn division_by_zero() -> Self {
        Self::coded("division-by-zero", "division by zero".to_owned())
    }

    /// `recursive-definition`: рекурсивное определение.
    pub fn recursive_definition(name: &str) -> Self {
        Self::coded(
            "recursive-definition",
            format!("recursive definition '{name}'"),
        )
    }

    /// `not-a-predicate`: вызов не-предиката в позиции условия.
    pub fn not_a_predicate(name: &str) -> Self {
        Self::coded("not-a-predicate", format!("'{name}' is not a predicate"))
    }

    /// `integer-out-of-range`: число вне диапазона `i64` в условии.
    pub fn integer_out_of_range(raw: &str) -> Self {
        Self::coded(
            "integer-out-of-range",
            format!("integer out of range '{raw}'"),
        )
    }

    /// `invalid-date`: кривой литерал даты в условии.
    pub fn invalid_date(raw: &str) -> Self {
        Self::coded("invalid-date", format!("invalid date '{raw}'"))
    }

    /// `duplicate-attribute`: `duplicate attribute 'a'` (дубль ключа в литерале мапы
    /// или в блоке действий).
    pub fn duplicate_attribute(name: &str) -> Self {
        Self::coded(
            "duplicate-attribute",
            format!("duplicate attribute '{name}'"),
        )
    }

    /// `unknown-field`: доступ к отсутствующему полю (`subj.name`), к полю не-мапы
    /// и индекс не-массива — всё `unknown field 'name'`.
    pub fn unknown_field(name: &str) -> Self {
        Self::coded("unknown-field", format!("unknown field '{name}'"))
    }

    /// `index-out-of-bounds`: индекс за границами массива (`tags[5]`, отрицательный `tags[-1]`).
    pub fn index_out_of_bounds(raw: &str) -> Self {
        Self::coded(
            "index-out-of-bounds",
            format!("index out of bounds '{raw}'"),
        )
    }

    /// `maps-not-comparable`: сравнение мап/массивов (`==`/`!=` между ними — «пока», см. черновик).
    pub fn maps_not_comparable() -> Self {
        Self::coded(
            "maps-not-comparable",
            "cannot compare maps or arrays".to_owned(),
        )
    }

    /// `cannot-read-import`: импорт не читается.
    pub fn cannot_read_import(path: &str) -> Self {
        Self::coded("cannot-read-import", format!("cannot read import '{path}'"))
    }

    /// `import-cycle`: цикл импорта.
    pub fn import_cycle(path: &str) -> Self {
        Self::coded("import-cycle", format!("import cycle '{path}'"))
    }

    /// `schedule-in-import`: расписание внутри импорта.
    pub fn schedule_in_import(path: &str) -> Self {
        Self::coded(
            "schedule-in-import",
            format!("schedule not allowed in import '{path}'"),
        )
    }

    /// `unknown-table`: `unknown table 'SHORT'` — таблицы с таким именем нет.
    pub fn unknown_table(name: &str) -> Self {
        Self::coded("unknown-table", format!("unknown table '{name}'"))
    }

    /// `unknown-slot`: `unknown slot '8th'` — метки нет в таблице вызова.
    pub fn unknown_slot(label: &str) -> Self {
        Self::coded("unknown-slot", format!("unknown slot '{label}'"))
    }

    /// `invalid-table-argument`: первый аргумент вызова рутины — не имя таблицы.
    pub fn invalid_table_argument(name: &str) -> Self {
        Self::coded(
            "invalid-table-argument",
            format!("invalid table argument for '{name}'"),
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
