//! Десугар `reverse`: производный цикл в обычный до валидации.
//!
//! `cycle BACK reverse FWD;` разворачивается в тело с зеркальными смещениями
//! (`o' = duration − o` от эффективного смещения) и остальным как есть:
//! вызовы, условия, повторы, параметры и длительность наследуются.
//! Дальше узел — обычный цикл: все проверки идут общим порядком
//! (см. docs/reference/semantics.md).

use cyclorithm_parser::{Cycle, Schedule, Stmt};

use crate::duration::{duration_from_ms, duration_ms, effective_offset_ms};
use crate::Error;

/// Заменить все `reverse`-узлы расписания обычными циклами.
/// Вызывать после разбора, до `validate_names`: дальше проверки
/// (duplicate/wrong-kind/unknown-cycle/recursive/границы/условия) идут
/// общим порядком, как для рукописных циклов. Узел заменяется на месте —
/// порядок объявления сохраняется (важен для «первая ошибка побеждает»).
pub fn materialize_reverse(schedule: &mut Schedule) -> Result<(), Error> {
    let pending: Vec<(usize, String)> = schedule
        .cycles
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.reverse_from.clone().map(|s| (i, s)))
        .collect();
    // Имена reverse-узлов — снапшотом исходного состояния: запрет цепочек
    // не зависит от порядка объявления, иначе `BACK` до `MID` падал бы,
    // а после — резолвился бы транзитивно. Угол с дубликатом имени
    // (явный цикл + reverse с тем же именем) тоже даёт recursive —
    // файл всё равно бит.
    let rev_nodes: Vec<String> = schedule
        .cycles
        .iter()
        .filter(|c| c.reverse_from.is_some())
        .map(|c| c.name.clone())
        .collect();
    for (idx, source_name) in &pending {
        if rev_nodes.iter().any(|n| n == source_name) {
            // Цепочки запрещены: источник обязан быть явным циклом.
            return Err(Error::recursive_cycle(source_name));
        }
        let src = find_source(schedule, source_name)?.clone();
        let limit = duration_ms(&src.duration)?;
        let mirrored = mirror_rows(&src, limit)?;
        let node = &mut schedule.cycles[*idx];
        node.params = src.params.clone();
        node.duration = src.duration.clone();
        node.stmts = mirrored;
        node.reverse_from = None;
    }
    Ok(())
}

/// Источник обязан быть явным циклом расписания. Порядок разрешения —
// как у вызовов: циклы, рутины, точки, неизвестное.
fn find_source<'a>(schedule: &'a Schedule, name: &str) -> Result<&'a Cycle, Error> {
    if let Some(src) = schedule.cycles.iter().find(|c| c.name == name) {
        debug_assert!(
            src.reverse_from.is_none(),
            "цепочки отсечены вызывателем по снапшоту"
        );
        return Ok(src);
    }
    if schedule.routines.iter().any(|r| r.name == name) {
        return Err(Error::routine_not_cycle(name));
    }
    if schedule.points.iter().any(|p| p.name == name) {
        return Err(Error::point_not_cycle(name));
    }
    Err(Error::unknown_cycle(name))
}

/// Зеркалить эффективные смещения (`negative` уже разрешён через
/// `effective_offset_ms`): результат всегда неотрицателен и пишется
/// обычной длительностью. Порядок строк — порядок объявления источника.
fn mirror_rows(src: &Cycle, limit: i64) -> Result<Vec<Stmt>, Error> {
    let limit_raw = src.duration.raw.clone();
    let mut stmts = Vec::with_capacity(src.stmts.len());
    for row in &src.stmts {
        let eff = effective_offset_ms(row, limit, &limit_raw)?;
        let (negative, magnitude) = if eff <= limit {
            (false, limit - eff)
        } else {
            // Источник уже невалиден (строка за длительностью): механически
            // переворачиваем знак, валидация результата добьёт общим порядком.
            (true, eff - limit)
        };
        stmts.push(Stmt {
            offset: duration_from_ms(magnitude),
            negative,
            repeat: row.repeat.clone(),
            condition: row.condition.clone(),
            invocation: row.invocation.clone(),
        });
    }
    Ok(stmts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirror_of(body: &str) -> cyclorithm_parser::Schedule {
        let src = format!(
            "schedule \"T\" {{ point A {{ actions = [x]; }} \
            cycle R duration = 1h {{ {body} }} \
            cycle BACK reverse R; \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h {{ 6h: BACK(); }} }}"
        );
        let mut schedule = cyclorithm_parser::parse(&src)
            .expect("фикстура обязана разбираться")
            .schedule;
        materialize_reverse(&mut schedule).expect("десугар обязан срабатывать");
        schedule
    }

    fn offsets(schedule: &cyclorithm_parser::Schedule) -> Vec<String> {
        schedule.cycles[1]
            .stmts
            .iter()
            .map(|s| s.offset_raw())
            .collect()
    }

    #[test]
    fn mirrors_offsets_around_duration() {
        // Пример из todo: 0/10/30/50 при 1h даёт 60/50/30/10.
        let s = mirror_of("0m: A.x(); 10m: A.x(); 30m: A.x(); 50m: A.x();");
        assert_eq!(offsets(&s), vec!["60m", "50m", "30m", "10m"]);
        let back = &s.cycles[1];
        assert_eq!(back.reverse_from, None);
        assert_eq!(back.duration.raw, "1h");
        assert_eq!(back.params, s.cycles[0].params);
    }

    #[test]
    fn mirrors_effective_negative_offsets() {
        // `-10m` при 1h — это 50m, зеркало — 10m обычной длительностью.
        let s = mirror_of("-10m: A.x();");
        assert_eq!(offsets(&s), vec!["10m"]);
        assert!(!s.cycles[1].stmts[0].negative);
    }

    #[test]
    fn rejects_unknown_source() {
        let src = "schedule \"T\" { point A { actions = [x]; } \
            cycle BACK reverse NOPE; \
            root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: BACK(); } }";
        let mut schedule = cyclorithm_parser::parse(src)
            .expect("фикстура обязана разбираться")
            .schedule;
        let err = materialize_reverse(&mut schedule).expect_err("источника нет");
        assert_eq!(
            (err.code, err.message.as_str()),
            ("unknown-cycle", "unknown cycle 'NOPE'")
        );
    }

    #[test]
    fn rejects_routine_and_chain_sources() {
        let head = "schedule \"T\" { point A { actions = [x]; } \
            routine M(TC) { 0m: A.x(); } ";
        let tail =
            " root_cycle start_time = \"2026-01-01T00:00:00\", duration = 24h { 6h: BACK(); } }";
        let src = format!("{head} cycle BACK reverse M; {tail}");
        let mut schedule = cyclorithm_parser::parse(&src)
            .expect("фикстура обязана разбираться")
            .schedule;
        let err = materialize_reverse(&mut schedule).expect_err("рутина не цикл");
        assert_eq!(
            (err.code, err.message.as_str()),
            ("wrong-kind", "routine 'M' is not a cycle")
        );
        // Цепочка: источник сам reverse — тоже wrong-kind через recursive.
        let src = format!(
            "{head} cycle MID reverse R; cycle BACK reverse MID; \
            cycle R duration = 1h {{ 0m: A.x(); }} {tail}"
        );
        let mut schedule = cyclorithm_parser::parse(&src)
            .expect("фикстура обязана разбираться")
            .schedule;
        let err = materialize_reverse(&mut schedule).expect_err("цепочка запрещена");
        assert_eq!(err.code, "recursive");
    }
}
