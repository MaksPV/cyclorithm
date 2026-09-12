//! Наивное время — §4 спеки («Время и арифметика»).
//!
//! Внутри — `i64` миллисекунд от unix epoch. Строки без таймзоны трактуются
//! 1:1, без сдвигов; таймзоны и DST не учитываются, сутки всегда 24h.
//! Формат входа и выхода: `YYYY-MM-DDTHH:MM:SS`, миллисекунды опциональны
//! (`.mmm`). Любое отклонение — invalid-datetime. Календарь — пролептический григорианский.

use crate::Error;

const MS_PER_DAY: i64 = 86_400_000;
const MS_PER_HOUR: i64 = 3_600_000;
const MS_PER_MIN: i64 = 60_000;
const MS_PER_SEC: i64 = 1_000;

/// Разбор наивной ISO-строки в миллисекунды epoch.
/// Ошибка — invalid-datetime с сырым текстом: `invalid datetime '...'`.
pub fn parse_datetime(s: &str) -> Result<i64, Error> {
    let b = s.as_bytes();
    let bad = || Error::invalid_datetime(s);
    // Строгая форма: 19 символов, плюс опциональные `.mmm`.
    if b.len() != 19 && b.len() != 23 {
        return Err(bad());
    }
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return Err(bad());
    }
    if b.len() == 23 && b[19] != b'.' {
        return Err(bad());
    }
    let y = digits(b, 0, 4).ok_or_else(bad)?;
    let mo = digits(b, 5, 2).ok_or_else(bad)?;
    let d = digits(b, 8, 2).ok_or_else(bad)?;
    let h = digits(b, 11, 2).ok_or_else(bad)?;
    let mi = digits(b, 14, 2).ok_or_else(bad)?;
    let se = digits(b, 17, 2).ok_or_else(bad)?;
    let milli = if b.len() == 23 {
        digits(b, 20, 3).ok_or_else(bad)?
    } else {
        0
    };
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 || se > 59
    {
        return Err(bad());
    }
    Ok(days_from_civil(y, mo, d) * MS_PER_DAY
        + h * MS_PER_HOUR
        + mi * MS_PER_MIN
        + se * MS_PER_SEC
        + milli)
}

/// Разбор даты из аргументов CLI: короткие формы и относительные дельты.
/// - `YYYY-MM-DD` → полночь, `YYYY-MM-DDTHH:MM` → нулевые секунды;
/// - `+DURATION` (`1d`, `2h30m`, `1w2d3h4m5s6ms` — сумма компонент) →
///   `anchor_ms` + длительность;
/// - иначе — строгий `parse_datetime`.
///
/// Ошибка — invalid-datetime с исходным текстом.
pub fn parse_cli_datetime(s: &str, anchor_ms: i64) -> Result<i64, Error> {
    if let Some(rest) = s.strip_prefix('+') {
        let delta = parse_cli_duration(rest).ok_or_else(|| Error::invalid_datetime(s))?;
        return anchor_ms
            .checked_add(delta)
            .ok_or_else(|| Error::invalid_datetime(s));
    }
    if s.len() == 10 {
        return parse_datetime(&format!("{s}T00:00:00")).map_err(|_| Error::invalid_datetime(s));
    }
    if s.len() == 16 {
        return parse_datetime(&format!("{s}:00")).map_err(|_| Error::invalid_datetime(s));
    }
    parse_datetime(s)
}

/// Длительность CLI: число + юнит (`w/d/h/m/s/ms`), компоненты суммируются.
/// Пусто, мусор и переполнение `i64` — `None`.
pub fn parse_cli_duration(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let b = s.as_bytes();
    let mut i = 0;
    let mut total: i128 = 0;
    while i < b.len() {
        let from = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if from == i {
            return None;
        }
        let n: i128 = s[from..i].parse().ok()?;
        // Голое число без юнита — миллисекунды (`--within 0`).
        if i >= b.len() {
            total += n;
            break;
        }
        let (mult, adv) = if s[i..].starts_with("ms") {
            (1, 2)
        } else {
            match b[i] {
                b'w' => (7 * 86_400_000, 1),
                b'd' => (86_400_000, 1),
                b'h' => (3_600_000, 1),
                b'm' => (60_000, 1),
                b's' => (1_000, 1),
                _ => return None,
            }
        };
        total += n * mult;
        i += adv;
    }
    i64::try_from(total).ok()
}

/// Миллисекунды epoch обратно в наивную ISO-строку (см. §6 вывода).
/// `.mmm` — только при ненулевых миллисекундах.
pub fn format_datetime(ms: i64) -> String {
    let days = ms.div_euclid(MS_PER_DAY);
    let rem = ms.rem_euclid(MS_PER_DAY);
    let (y, mo, d) = civil_from_days(days);
    let h = rem / MS_PER_HOUR;
    let mi = rem % MS_PER_HOUR / MS_PER_MIN;
    let se = rem % MS_PER_MIN / MS_PER_SEC;
    let milli = rem % MS_PER_SEC;
    if milli == 0 {
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}")
    } else {
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}.{milli:03}")
    }
}

/// Каноническая форма для сравнения дат как строк (§4.13): всегда 23 символа
/// `YYYY-MM-DDTHH:MM:SS.mmm`. Вне годов `0000…9999` формы нет — `None`.
pub fn format_datetime_full(ms: i64) -> Option<String> {
    let days = ms.div_euclid(MS_PER_DAY);
    let rem = ms.rem_euclid(MS_PER_DAY);
    let (y, mo, d) = civil_from_days(days);
    if !(0..=9999).contains(&y) {
        return None;
    }
    let h = rem / MS_PER_HOUR;
    let mi = rem % MS_PER_HOUR / MS_PER_MIN;
    let se = rem % MS_PER_MIN / MS_PER_SEC;
    let milli = rem % MS_PER_SEC;
    Some(format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}.{milli:03}"
    ))
}

/// Ровно `len` ASCII-цифр с позиции `from`. Индексы безопасны: длина входа
/// уже проверена вызывателем (19 или 23).
fn digits(b: &[u8], from: usize, len: usize) -> Option<i64> {
    let mut v: i64 = 0;
    for &c in &b[from..from + len] {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as i64;
    }
    Some(v)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

pub(crate) fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

/// Ошибка сборки даты из компонентов (`mkdate`, §4.13 спеки).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DateBuildErr {
    /// Кривые компоненты: месяц вне 1..12, день вне месяца (високосный
    /// февраль учтён), время суток вне границ.
    Invalid,
    /// День в миллисекундах не влез в `i64` (годы — весь диапазон).
    Overflow,
}

/// Дата и время из компонентов в миллисекунды epoch (наивное, как везде).
/// Порядок проверок: месяц → день → время суток → checked-сборка.
pub(crate) fn make_datetime(
    y: i64,
    mo: i64,
    d: i64,
    h: i64,
    mi: i64,
    s: i64,
    ms: i64,
) -> Result<i64, DateBuildErr> {
    use DateBuildErr::{Invalid, Overflow};
    // Месяц раньше дня: `days_in_month` на месяце вне 1..12 врёт `_`-веткой.
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) {
        return Err(Invalid);
    }
    if !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=59).contains(&s)
        || !(0..=999).contains(&ms)
    {
        return Err(Invalid);
    }
    let day_ms = days_from_civil(y, mo, d)
        .checked_mul(MS_PER_DAY)
        .ok_or(Overflow)?;
    // Время суток влезает всегда (< суток); переполнение — только от даты.
    day_ms
        .checked_add(h * MS_PER_HOUR + mi * MS_PER_MIN + s * MS_PER_SEC + ms)
        .ok_or(Overflow)
}

/// Дни от unix epoch (алгоритм Хиннанта; `div_euclid` корректен и до epoch).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Обратное преобразование дней в дату.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> i64 {
        parse_datetime(s).expect("дата обязана разбираться")
    }

    fn bad(s: &str) -> Error {
        parse_datetime(s).expect_err("ожидалась invalid-datetime")
    }

    #[test]
    fn parses_known_instants() {
        // 2026-01-01T00:00:00Z = 1767225600 c epoch.
        assert_eq!(ok("2026-01-01T00:00:00"), 1_767_225_600_000);
        assert_eq!(ok("2026-01-10T06:00:00"), 1_768_024_800_000);
        assert_eq!(ok("1970-01-01T00:00:00"), 0);
        assert_eq!(ok("2026-01-01T00:00:00.123"), 1_767_225_600_123);
    }

    #[test]
    fn respects_leap_years() {
        ok("2024-02-29T12:00:00");
        ok("2000-02-29T00:00:00");
        assert_eq!(bad("2023-02-29T00:00:00").code, "invalid-datetime");
        assert_eq!(bad("1900-02-29T00:00:00").code, "invalid-datetime");
    }

    #[test]
    fn rejects_malformed() {
        // Фикстура bad_invalid-datetime + нарушения формы и диапазонов.
        for s in [
            "not-a-datetime",
            "2026-1-1T0:0:0",
            "2026-01-01 00:00:00",
            "2026-01-01T00:00:00.",
            "2026-01-01T00:00:00.12",
            "2026-13-01T00:00:00",
            "2026-00-10T00:00:00",
            "2026-01-32T00:00:00",
            "2026-04-31T00:00:00",
            "2026-01-01T24:00:00",
            "2026-01-01T00:60:00",
            "2026-01-01T00:00:60",
            "",
        ] {
            let err = bad(s);
            assert_eq!(err.code, "invalid-datetime", "для {s:?}");
            assert_eq!(err.message, format!("invalid datetime '{s}'"), "для {s:?}");
        }
    }

    #[test]
    fn cli_shorthands_and_deltas() {
        let base = ok("2026-09-07T10:00:00");
        let cli = |s: &str| parse_cli_datetime(s, base).expect("дата обязана разбираться");
        // Короткие формы.
        assert_eq!(cli("2026-09-07"), ok("2026-09-07T00:00:00"));
        assert_eq!(cli("2026-09-07T09:30"), ok("2026-09-07T09:30:00"));
        assert_eq!(cli("2026-09-07T09:30:00"), base - 30 * 60_000);
        // Дельты от якоря.
        assert_eq!(cli("+1d"), base + 86_400_000);
        assert_eq!(cli("+2h30m"), base + 9_000_000);
        assert_eq!(cli("+1w2d3h4m5s6ms"), base + 788_645_006);
        assert_eq!(cli("+0d"), base);
        assert_eq!(cli("+0"), base);
        assert_eq!(cli("+1500"), base + 1500);
        assert_eq!(cli("+1d2"), base + 86_400_002);
        // Битые — invalid-datetime с исходным текстом.
        for s in [
            "2026-09-07T09",
            "+1x",
            "+",
            "+1d2x",
            "2026-13-40",
            "+999999999999999999999d",
        ] {
            let err = parse_cli_datetime(s, base).expect_err("ожидалась invalid-datetime");
            assert_eq!(err.code, "invalid-datetime", "для {s:?}");
            assert_eq!(err.message, format!("invalid datetime '{s}'"), "для {s:?}");
        }
    }

    #[test]
    fn formats_back() {
        assert_eq!(format_datetime(0), "1970-01-01T00:00:00");
        assert_eq!(format_datetime(1_767_225_600_000), "2026-01-01T00:00:00");
        assert_eq!(
            format_datetime(1_767_225_600_123),
            "2026-01-01T00:00:00.123"
        );
        assert_eq!(format_datetime(1_768_024_800_000), "2026-01-10T06:00:00");
        // Круговой обход строка → мс → строка.
        for s in [
            "2026-01-01T00:00:00",
            "2024-02-29T12:34:56.789",
            "1970-01-01T00:00:01.001",
        ] {
            assert_eq!(format_datetime(ok(s)), s);
        }
    }
}
