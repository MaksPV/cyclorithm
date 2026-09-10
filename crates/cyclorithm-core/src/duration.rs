//! Длительности в миллисекунды (`i64`) — §2 и §4 спеки.
//!
//! Фиксированные: `w = 7d`, `d = 24h`. Порядок компонентов строго по убыванию
//! (`w > d > h > m > s > ms`), каждый не более одного раза — иначе E05.
//! Значение обязано влезать в `i64` миллисекунд — иначе E05.
//! Ноль (`0m`) сам по себе валиден; запрет нулевого *периода* `root_cycle`
//! (деление на ноль в решётке) проверяется отдельно, тоже E05.

use cyclorithm_parser::{Duration, DurationUnit, RootCycle};

use crate::Error;

/// Миллисекунд в единице.
fn unit_ms(unit: DurationUnit) -> i128 {
    match unit {
        DurationUnit::Week => 7 * 24 * 60 * 60 * 1000,
        DurationUnit::Day => 24 * 60 * 60 * 1000,
        DurationUnit::Hour => 60 * 60 * 1000,
        DurationUnit::Minute => 60 * 1000,
        DurationUnit::Second => 1000,
        DurationUnit::Millisecond => 1,
    }
}

/// Ранг для проверки порядка: строго убывать `w > d > h > m > s > ms`.
fn unit_rank(unit: DurationUnit) -> u8 {
    match unit {
        DurationUnit::Week => 6,
        DurationUnit::Day => 5,
        DurationUnit::Hour => 4,
        DurationUnit::Minute => 3,
        DurationUnit::Second => 2,
        DurationUnit::Millisecond => 1,
    }
}

/// Сырые цифры в число. Переполнение `i128` невозможно для корректного
/// значения `i64` мс — значит, это E05.
fn parse_number(s: &str) -> Result<i128, ()> {
    let mut v: i128 = 0;
    for b in s.bytes() {
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add((b - b'0') as i128))
            .ok_or(())?;
    }
    Ok(v)
}

/// Сумма компонентов в миллисекундах (`i64`).
/// Ошибки — E05 с сырым текстом: `invalid duration '1h2h'`.
pub fn duration_ms(d: &Duration) -> Result<i64, Error> {
    if d.items.is_empty() {
        return Err(Error::e05(&d.raw));
    }
    let mut prev_rank = u8::MAX;
    let mut total: i128 = 0;
    for item in &d.items {
        let rank = unit_rank(item.unit);
        // Равный ранг = повтор (`1h2h`), больший = неверный порядок (`30s1d`).
        if rank >= prev_rank {
            return Err(Error::e05(&d.raw));
        }
        prev_rank = rank;
        let n = parse_number(&item.number).map_err(|_| Error::e05(&d.raw))?;
        let add = n
            .checked_mul(unit_ms(item.unit))
            .ok_or_else(|| Error::e05(&d.raw))?;
        total = total.checked_add(add).ok_or_else(|| Error::e05(&d.raw))?;
    }
    i64::try_from(total).map_err(|_| Error::e05(&d.raw))
}

/// Период `root_cycle` в миллисекундах. Ноль запрещён (деление на ноль
/// в решётке) — тоже E05 с сырым текстом длительности.
pub fn root_period_ms(root: &RootCycle) -> Result<i64, Error> {
    let ms = duration_ms(&root.duration)?;
    if ms == 0 {
        return Err(Error::e05(&root.duration.raw));
    }
    Ok(ms)
}

/// Эффективное смещение строки (§4 спеки): обычное — как есть,
/// отрицательное `-X` — как `parent_ms − X`, где `parent_ms` — объявленная
/// длительность непосредственно объемлющего цикла.
/// `X > parent_ms` — E07 (`offset '-2h' out of bounds (duration 1h20m)`);
/// `parent_raw` — сырой текст длительности родителя для сообщения.
/// Невалидная запись самого смещения (`1h2h`) — сначала E05.
pub fn effective_offset_ms(
    stmt: &cyclorithm_parser::Stmt,
    parent_ms: i64,
    parent_raw: &str,
) -> Result<i64, Error> {
    let x = duration_ms(&stmt.offset)?;
    if stmt.negative {
        if x > parent_ms {
            return Err(Error::e07_neg_offset(&stmt.offset_raw(), parent_raw));
        }
        Ok(parent_ms - x)
    } else {
        Ok(x)
    }
}

/// Миллисекунды в человеческую строку для сообщений E07.
///
/// Формат прибит примером из §5: `by 20m (80m > 60m)` — все три числа
/// в минутах, хотя `60m` это ровно `1h`. Отсюда каскад: целые минуты —
/// суммарно в `m`, иначе целые секунды — в `s`, иначе `s+ms`/`ms`.
/// Часы и крупнее никогда не печатаются (иначе пример не сходится).
pub fn format_duration(ms: i64) -> String {
    if ms % 60_000 == 0 {
        format!("{}m", ms / 60_000)
    } else if ms % 1_000 == 0 {
        format!("{}s", ms / 1_000)
    } else if ms / 1_000 > 0 {
        format!("{}s{}ms", ms / 1_000, ms % 1_000)
    } else {
        format!("{ms}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dur(raw: &str, items: &[(&str, DurationUnit)]) -> Duration {
        Duration {
            raw: raw.to_owned(),
            items: items
                .iter()
                .map(|(n, u)| cyclorithm_parser::DurationItem {
                    number: n.to_string(),
                    unit: *u,
                })
                .collect(),
        }
    }

    fn ms(raw: &str, items: &[(&str, DurationUnit)]) -> i64 {
        duration_ms(&dur(raw, items)).expect("длительность обязана быть корректной")
    }

    fn e05(raw: &str, items: &[(&str, DurationUnit)]) -> Error {
        duration_ms(&dur(raw, items)).expect_err("ожидалась E05")
    }

    #[test]
    fn converts_units() {
        use DurationUnit::*;
        assert_eq!(ms("1ms", &[("1", Millisecond)]), 1);
        assert_eq!(ms("40m", &[("40", Minute)]), 2_400_000);
        assert_eq!(ms("1h20m", &[("1", Hour), ("20", Minute)]), 4_800_000);
        assert_eq!(ms("24h", &[("24", Hour)]), 86_400_000);
        assert_eq!(ms("1d2s", &[("1", Day), ("2", Second)]), 86_402_000);
        assert_eq!(
            ms(
                "1w2d3h4m5s6ms",
                &[
                    ("1", Week),
                    ("2", Day),
                    ("3", Hour),
                    ("4", Minute),
                    ("5", Second),
                    ("6", Millisecond)
                ]
            ),
            788_645_006
        );
    }

    #[test]
    fn zero_is_valid() {
        use DurationUnit::*;
        assert_eq!(ms("0m", &[("0", Minute)]), 0);
    }

    #[test]
    fn formats_for_e07_messages() {
        // Прибито примером §5: `by 20m (80m > 60m)`.
        assert_eq!(format_duration(1_200_000), "20m");
        assert_eq!(format_duration(4_800_000), "80m");
        assert_eq!(format_duration(3_600_000), "60m");
        // Каскад ниже минут.
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(60_000), "1m");
        assert_eq!(format_duration(90_000), "90s");
        assert_eq!(format_duration(1_500), "1s500ms");
        assert_eq!(format_duration(500), "500ms");
        // Часы и крупнее суммарно в минутах — следствие примера.
        assert_eq!(format_duration(90_000_000), "1500m");
    }

    #[test]
    fn resolves_negative_offsets() {
        use cyclorithm_parser::Stmt;
        let stmt = |negative: bool, raw: &str, items: &[(&str, DurationUnit)]| Stmt {
            offset: dur(raw, items),
            negative,
            repeat: cyclorithm_parser::Repeat::Once,
            condition: None,
            invocation: cyclorithm_parser::Invocation::CycleCall {
                name: "R".to_owned(),
                args: Vec::new(),
            },
        };
        use DurationUnit::*;
        // Родитель 1h20m = 4_800_000 мс.
        let parent = 4_800_000;
        // Середина: -10m ≡ 70m.
        let st = stmt(true, "10m", &[("10", Minute)]);
        assert_eq!(effective_offset_ms(&st, parent, "1h20m"), Ok(4_200_000));
        // Встык: -0m ≡ конец.
        let st = stmt(true, "0m", &[("0", Minute)]);
        assert_eq!(effective_offset_ms(&st, parent, "1h20m"), Ok(4_800_000));
        // Эффективный ноль: -1h20m ≡ 0m.
        let st = stmt(true, "1h20m", &[("1", Hour), ("20", Minute)]);
        assert_eq!(effective_offset_ms(&st, parent, "1h20m"), Ok(0));
        // Положительное — как есть.
        let st = stmt(false, "40m", &[("40", Minute)]);
        assert_eq!(effective_offset_ms(&st, parent, "1h20m"), Ok(2_400_000));
    }

    #[test]
    fn rejects_negative_out_of_bounds() {
        use cyclorithm_parser::Stmt;
        use DurationUnit::*;
        // -2h при родителе 1h20m: эффективное -40m — E07, код и сообщение по §5.
        let st = Stmt {
            offset: dur("2h", &[("2", Hour)]),
            negative: true,
            repeat: cyclorithm_parser::Repeat::Once,
            condition: None,
            invocation: cyclorithm_parser::Invocation::CycleCall {
                name: "R".to_owned(),
                args: Vec::new(),
            },
        };
        let err = effective_offset_ms(&st, 4_800_000, "1h20m").expect_err("вылет ниже нуля");
        assert_eq!(err.code, "E07");
        assert_eq!(err.message, "offset '-2h' out of bounds (duration 1h20m)");
    }

    #[test]
    fn rejects_zero_root_period() {
        use cyclorithm_parser::{RootCycle, Stmt};
        use DurationUnit::*;
        let root = |raw: &str, items: &[(&str, DurationUnit)]| RootCycle {
            start_time: "2026-01-01T00:00:00".to_owned(),
            duration: dur(raw, items),
            stmts: Vec::<Stmt>::new(),
        };
        // Ноль в любом виде — E05; ненулевой период проходит.
        for (raw, items) in [
            ("0m", vec![("0", Minute)]),
            ("0h0m", vec![("0", Hour), ("0", Minute)]),
        ] {
            let err = root_period_ms(&root(raw, &items)).expect_err("нулевой период запрещён");
            assert_eq!(err.code, "E05");
            assert_eq!(err.message, format!("invalid duration '{raw}'"));
        }
        assert_eq!(
            root_period_ms(&root("24h", &[("24", Hour)])),
            Ok(86_400_000)
        );
    }

    #[test]
    fn rejects_duplicate_and_order() {
        use DurationUnit::*;
        // Повтор: `1h2h`. Порядок: `30s1d`, `1ms1m` (ms — младший).
        for (raw, items) in [
            ("1h2h", vec![("1", Hour), ("2", Hour)]),
            ("30s1d", vec![("30", Second), ("1", Day)]),
            ("1ms1m", vec![("1", Millisecond), ("1", Minute)]),
        ] {
            let err = e05(raw, &items);
            assert_eq!(err.code, "E05");
            assert_eq!(err.message, format!("invalid duration '{raw}'"));
        }
        // А так можно: убывание `m > ms`, `s > ms`.
        assert_eq!(ms("1m1ms", &[("1", Minute), ("1", Millisecond)]), 60_001);
    }

    #[test]
    fn rejects_overflow() {
        use DurationUnit::*;
        // Граница лимита i64 из спеки: `106751991167d` влезает,
        // `106751991168d` — первое значение, которое уже нет.
        assert_eq!(
            ms("106751991167d", &[("106751991167", Day)]),
            9_223_372_036_828_800_000
        );
        let err = e05("106751991168d", &[("106751991168", Day)]);
        assert_eq!(err.code, "E05");
        assert_eq!(err.message, "invalid duration '106751991168d'");
        // Мусорные цифры длиной в километр — тоже E05, а не паника.
        let err = e05(
            "99999999999999999999999h",
            &[("99999999999999999999999", Hour)],
        );
        assert_eq!(err.code, "E05");
    }
}
