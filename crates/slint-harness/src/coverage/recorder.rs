//! What the run did, by identity strings only (no live handles).
//!
//! The harness feeds the recorder from three places: [`record_activation`] (a click, an
//! accessible default action or a smoke press reached the element), [`record_touch`] (a scenario
//! fetched the element to act on it: set a value, type, drag) and [`record_callback`] (a scenario
//! invoked a global callback directly and asserted its effect). The end-of-run gate
//! (`coverage::finish`) compares these sets with the static inventory.
//!
//! The state lives in a process-global [`Mutex`], so it accumulates across `run_to_report()`
//! calls in one process; that is what the coverage self-test relies on.

use crate::ElementHandle;
use std::{
    collections::BTreeSet,
    sync::{Mutex, MutexGuard, PoisonError},
};

/// What the run did, by identity strings only (no live handles).
pub(crate) struct Recorder {
    /// `Component::id` of elements a click / smoke activation reached.
    pub(crate) activated_ids: BTreeSet<String>,
    /// Accessible labels of those elements.
    pub(crate) activated_labels: BTreeSet<String>,
    /// `(element type name, accessible label)` of activated elements: how an instantiation site
    /// without an id is recognised.
    pub(crate) activated_pairs: BTreeSet<(String, String)>,
    /// Same for elements a scenario fetched to act on (`find*`).
    pub(crate) touched_ids: BTreeSet<String>,
    /// `(type, label)` of touched elements.
    pub(crate) touched_pairs: BTreeSet<(String, String)>,
    /// Labels of touched elements.
    pub(crate) touched_labels: BTreeSet<String>,
    /// `Model.name` of callbacks that were invoked.
    pub(crate) callbacks: BTreeSet<String>,
}

impl Recorder {
    const fn new() -> Self {
        Self {
            activated_ids: BTreeSet::new(),
            activated_labels: BTreeSet::new(),
            activated_pairs: BTreeSet::new(),
            touched_pairs: BTreeSet::new(),
            touched_ids: BTreeSet::new(),
            touched_labels: BTreeSet::new(),
            callbacks: BTreeSet::new(),
        }
    }
}

static RECORDER: Mutex<Recorder> = Mutex::new(Recorder::new());

/// Locks the process-global recorder (a poisoned lock is still usable: the sets stay valid).
pub(crate) fn recorder() -> MutexGuard<'static, Recorder> {
    RECORDER.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Normalised id: Slint treats `_` and `-` alike.
pub(crate) fn normalise_id(id: &str) -> String {
    id.replace('_', "-")
}

/// Records that the harness activated `handle` (click, accessible default action, smoke).
pub fn record_activation(handle: &ElementHandle) {
    record(handle, false);
}

/// Records that a scenario fetched `handle` to act on it (set a value, type, drag ...).
pub fn record_touch(handle: &ElementHandle) {
    record(handle, true);
}

fn record(handle: &ElementHandle, touched: bool) {
    let id = handle
        .id()
        .map(|i| normalise_id(&i))
        .filter(|i| !i.is_empty());
    let label = handle
        .accessible_label()
        .map(|l| l.to_string())
        .filter(|l| !l.trim().is_empty());
    let type_name = handle
        .type_name()
        .map(|t| t.to_string())
        .filter(|t| !t.is_empty());
    let mut rec = recorder();
    if let (Some(type_name), Some(label)) = (&type_name, &label) {
        let pair = (type_name.clone(), label.clone());
        if touched {
            rec.touched_pairs.insert(pair);
        } else {
            rec.activated_pairs.insert(pair);
        }
    }
    if let Some(id) = id {
        if touched {
            rec.touched_ids.insert(id);
        } else {
            rec.activated_ids.insert(id);
        }
    }
    if let Some(label) = label {
        if touched {
            rec.touched_labels.insert(label);
        } else {
            rec.activated_labels.insert(label);
        }
    }
}

/// Records that the callback `Model.name` (snake case) was invoked and its handler ran.
pub fn record_callback(key: &str) {
    recorder().callbacks.insert(key.replace('-', "_"));
}

/// The callbacks recorded so far.
#[must_use]
pub fn recorded_callbacks() -> BTreeSet<String> {
    recorder().callbacks.clone()
}
