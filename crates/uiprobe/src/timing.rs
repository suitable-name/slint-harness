//! Where a run spends its real time: a process-global table of labelled samples.
//!
//! The backend records the waits it makes (`wait`, `settle`, `pump`); an app's harness code adds
//! its own with [`record`], [`record_timeout`] and [`time`] (for example one label per relaunch
//! or per shutdown). The runner resets the table when a scenario begins and takes it when the
//! scenario ends, so every scenario in the report carries its own numbers: the console prints a
//! `timing:` line under the scenario, `report.json` stores the table and the report files get a
//! "Timing" section. In a parallel run every worker keeps its own table and sends it to the
//! coordinator with the scenario's `done` message.
//!
//! A sample is a label and a real-time duration. A sample that hit its timeout counts as an
//! ordinary sample (count and total) and is also counted separately, so a label shows both how
//! much time it took and how often it gave up.

use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

/// How many labels the console line names.
const CONSOLE_LABELS: usize = 6;

/// What one label has collected: sample count, total time, and how many samples hit their
/// timeout with the time those took (those are included in `count` and `total_us`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) count: u64,
    pub(crate) total_us: u64,
    pub(crate) timeouts: u64,
    pub(crate) timeout_us: u64,
}

/// Label -> samples; the unit the runner stores per scenario.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Table {
    entries: BTreeMap<String, Entry>,
}

impl Table {
    pub(crate) const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn get(&self, label: &str) -> Option<&Entry> {
        self.entries.get(label)
    }

    /// Adds one sample of `us` microseconds under `label`.
    pub(crate) fn add(&mut self, label: &str, us: u64, timed_out: bool) {
        let entry = self.entries.entry(label.to_owned()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.total_us = entry.total_us.saturating_add(us);
        if timed_out {
            entry.timeouts = entry.timeouts.saturating_add(1);
            entry.timeout_us = entry.timeout_us.saturating_add(us);
        }
    }

    /// Adds every label of `other` to this table.
    pub(crate) fn merge(&mut self, other: &Self) {
        for (label, theirs) in &other.entries {
            let entry = self.entries.entry(label.clone()).or_default();
            entry.count = entry.count.saturating_add(theirs.count);
            entry.total_us = entry.total_us.saturating_add(theirs.total_us);
            entry.timeouts = entry.timeouts.saturating_add(theirs.timeouts);
            entry.timeout_us = entry.timeout_us.saturating_add(theirs.timeout_us);
        }
    }

    /// The labels by total time, largest first (ties by name).
    pub(crate) fn sorted(&self) -> Vec<(&str, &Entry)> {
        let mut rows: Vec<(&str, &Entry)> = self
            .entries
            .iter()
            .map(|(label, entry)| (label.as_str(), entry))
            .collect();
        rows.sort_by(|a, b| b.1.total_us.cmp(&a.1.total_us).then_with(|| a.0.cmp(b.0)));
        rows
    }

    /// The wire form a worker sends to the coordinator (exact microseconds).
    pub(crate) fn to_json(&self) -> Value {
        let mut map = Map::new();
        for (label, entry) in &self.entries {
            map.insert(
                label.clone(),
                json!({
                    "count": entry.count,
                    "total_us": entry.total_us,
                    "timeouts": entry.timeouts,
                    "timeout_us": entry.timeout_us,
                }),
            );
        }
        Value::Object(map)
    }

    /// Reads [`Table::to_json`]; missing or malformed parts are skipped.
    pub(crate) fn from_json(value: Option<&Value>) -> Self {
        let mut table = Self::new();
        let Some(map) = value.and_then(Value::as_object) else {
            return table;
        };
        for (label, entry) in map {
            let number = |key: &str| entry.get(key).and_then(Value::as_u64).unwrap_or(0);
            table.entries.insert(
                label.clone(),
                Entry {
                    count: number("count"),
                    total_us: number("total_us"),
                    timeouts: number("timeouts"),
                    timeout_us: number("timeout_us"),
                },
            );
        }
        table
    }

    /// The form `report.json` stores (milliseconds, truncated).
    pub(crate) fn to_report_json(&self) -> Value {
        let mut map = Map::new();
        for (label, entry) in &self.entries {
            map.insert(
                label.clone(),
                json!({
                    "count": entry.count,
                    "total_ms": entry.total_us / 1000,
                    "timeouts": entry.timeouts,
                    "timeout_ms": entry.timeout_us / 1000,
                }),
            );
        }
        Value::Object(map)
    }

    /// `relaunch 31.2 s (45x), wait 12.0 s (210x, 3 timed out 9.0 s), ...`: the labels with
    /// the most time, at most six; `None` for an empty table.
    pub(crate) fn console_text(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let parts: Vec<String> = self
            .sorted()
            .into_iter()
            .take(CONSOLE_LABELS)
            .map(|(label, entry)| {
                let timeouts = if entry.timeouts > 0 {
                    format!(
                        ", {} timed out {}",
                        entry.timeouts,
                        format_us(entry.timeout_us)
                    )
                } else {
                    String::new()
                };
                format!(
                    "{label} {} ({}x{timeouts})",
                    format_us(entry.total_us),
                    entry.count
                )
            })
            .collect();
        Some(parts.join(", "))
    }
}

/// Microseconds as `NNN ms` below a second, else seconds with one decimal (`31.2 s`).
pub(crate) fn format_us(us: u64) -> String {
    if us < 1_000_000 {
        return format!("{} ms", us / 1000);
    }
    let tenths = us / 100_000;
    format!("{}.{} s", tenths / 10, tenths % 10)
}

fn micros(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

static TABLE: Mutex<Table> = Mutex::new(Table::new());

/// Locks the process-global table (a poisoned lock is still usable).
fn table() -> MutexGuard<'static, Table> {
    TABLE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records one sample of `elapsed` real time under `label`.
pub fn record(label: impl Into<String>, elapsed: Duration) {
    let label: String = label.into();
    table().add(&label, micros(elapsed), false);
}

/// Records a sample that hit its timeout (it counts as an ordinary sample too, and is also
/// counted as a timeout of `label`).
pub fn record_timeout(label: impl Into<String>, elapsed: Duration) {
    let label: String = label.into();
    table().add(&label, micros(elapsed), true);
}

/// Runs `f` and records its real time under `label`.
pub fn time<T>(label: impl Into<String>, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    record(label, start.elapsed());
    out
}

/// Records under a borrowed label (the backend's built-in samples).
pub(crate) fn record_str(label: &str, elapsed: Duration, timed_out: bool) {
    table().add(label, micros(elapsed), timed_out);
}

/// Empties the table (a scenario begins).
pub(crate) fn reset() {
    *table() = Table::new();
}

/// Takes the table and leaves it empty (a scenario ended).
pub(crate) fn take() -> Table {
    std::mem::take(&mut *table())
}

/// A copy of the table without emptying it.
#[cfg(test)]
pub(crate) fn snapshot() -> Table {
    table().clone()
}

#[cfg(test)]
mod tests {
    use super::{Table, format_us, record, record_timeout, snapshot, time};
    use std::time::Duration;

    #[test]
    fn samples_add_up_and_timeouts_count_twice() {
        let mut table = Table::new();
        table.add("wait", 1_500_000, false);
        table.add("wait", 500_000, true);
        table.add("settle", 10, false);
        let wait = table.get("wait").copied().unwrap_or_default();
        assert_eq!(
            (wait.count, wait.total_us, wait.timeouts, wait.timeout_us),
            (2, 2_000_000, 1, 500_000)
        );
        let order: Vec<&str> = table.sorted().into_iter().map(|(label, _)| label).collect();
        assert_eq!(order, ["wait", "settle"]);
    }

    #[test]
    fn merge_adds_every_field() {
        let mut one = Table::new();
        one.add("a", 100, false);
        one.add("b", 5, true);
        let mut two = Table::new();
        two.add("a", 200, true);
        two.merge(&one);
        let a = two.get("a").copied().unwrap_or_default();
        assert_eq!((a.count, a.total_us, a.timeouts), (2, 300, 1));
        assert!(two.get("b").is_some());
    }

    #[test]
    fn json_round_trip_is_exact() {
        let mut table = Table::new();
        table.add("relaunch", 31_234_567, false);
        table.add("wait", 3_000_001, true);
        let back = Table::from_json(Some(&table.to_json()));
        assert_eq!(back, table);
        assert!(Table::from_json(None).is_empty());
        assert!(Table::from_json(Some(&serde_json::json!([1, 2]))).is_empty());
    }

    #[test]
    fn report_json_uses_milliseconds() {
        let mut table = Table::new();
        table.add("wait", 2_500_999, true);
        let value = table.to_report_json();
        assert_eq!(value["wait"]["count"], 1);
        assert_eq!(value["wait"]["total_ms"], 2500);
        assert_eq!(value["wait"]["timeouts"], 1);
        assert_eq!(value["wait"]["timeout_ms"], 2500);
    }

    #[test]
    fn console_text_names_the_biggest_labels() {
        let mut table = Table::new();
        assert_eq!(table.console_text(), None);
        table.add("relaunch", 31_200_000, false);
        table.add("wait", 4_000_000, false);
        table.add("wait", 8_000_000, true);
        table.add("wait", 0, true);
        let text = table.console_text().unwrap_or_default();
        assert_eq!(
            text,
            "relaunch 31.2 s (1x), wait 12.0 s (3x, 2 timed out 8.0 s)"
        );
        for n in 0..10 {
            table.add(&format!("l{n}"), 1, false);
        }
        // Six labels in all: relaunch (1x), wait (3x, ...) and four of the 1-microsecond ones.
        let text = table.console_text().unwrap_or_default();
        assert_eq!(text.matches("(1x)").count(), 5, "{text}");
        assert!(text.contains("wait 12.0 s"), "{text}");
    }

    #[test]
    fn durations_format_in_ms_or_tenths_of_seconds() {
        assert_eq!(format_us(0), "0 ms");
        assert_eq!(format_us(999_999), "999 ms");
        assert_eq!(format_us(1_000_000), "1.0 s");
        assert_eq!(format_us(31_249_999), "31.2 s");
    }

    /// Only this test uses the process-global table, with labels no other test writes.
    #[test]
    fn the_global_functions_record() {
        record("unit-test-record", Duration::from_millis(3));
        record_timeout("unit-test-record", Duration::from_millis(2));
        let out = time("unit-test-time", || 7);
        assert_eq!(out, 7);
        let table = snapshot();
        let rec = table.get("unit-test-record").copied().unwrap_or_default();
        assert_eq!(
            (rec.count, rec.total_us, rec.timeouts, rec.timeout_us),
            (2, 5000, 1, 2000)
        );
        assert_eq!(table.get("unit-test-time").map(|e| e.count), Some(1));
    }
}
