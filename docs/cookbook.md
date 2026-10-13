# slint-harness cookbook

Recipes for situations the README does not cover. Every snippet is a scenario body or a builder
call against the API in `crates/slint-harness/src`; they are not compiled by the doc tests, so
read them next to the rustdoc.

## Secondary windows and dialogs

A window the app creates itself is wrapped with `ctx.attach(name, component)` and then behaves
like a launched one: `ctx.view(&dialog)`, `ctx.sweep(&dialog, "settings open")`. A dialog that
lives inside the main window is a subtree: find its root by id and build a `WindowView` over it
with `WindowView::subtree`, so the sweep checks it against the window size. Both kinds can be
returned from a smoke `restore`; the first window of the list is the main one.

```rust
.scenario("settings", |ctx| {
    let app = ctx.launch(MainWindow::new)?;
    ctx.view(&app).click("Settings")?;
    // `open_settings` is the app's own function that creates and shows the dialog window.
    let dialog = my_app::open_settings().map_err(|e| Finding::simple("launch", e.to_string()))?;
    let settings = ctx.attach("Settings", dialog);
    ctx.sweep(&settings, "settings open");

    // An in-window dialog: a subtree of the main window.
    let view = ctx.view(&app);
    let root = view.find_id("MainWindow::confirm")?;
    let confirm = WindowView::subtree("Confirm", root, view.size);
    ctx.sweep_view(&confirm, "confirm open");
    ctx.shutdown(settings);
    ctx.shutdown(app);
    Ok(())
})
```

## Apps with threads

When the window is built from a larger app handle that owns worker threads, keep the handle
alive with the window: `Attached::keep` stores any value until the window is dropped. Background
work runs in real time while timers run in mock time, so wait with `ctx.wait_for` (a `timeout`
finding on failure) or `settle`, both of which pump. Stop the threads in `Harness::teardown`, which
runs before the profile is deleted; on Linux the harness warns when threads remain.

```rust
use std::time::Duration;

Harness::new()
    .teardown(|| stop_worker_threads())
    .scenario("load", |ctx| {
        let app = ctx.launch(|| build_app().map(|app| app.ui.clone()))?.keep(app_state());
        let loaded = ctx.wait_for("document loaded", Duration::from_secs(5), || is_loaded());
        if loaded {
            settle(100); // lets the last queued callbacks run
        }
        ctx.shutdown(app);
        Ok(())
    })
    .run()
```

## Sticky notices

An error notice that does not hide itself would be reported again after every click of the
smoke pass. Register two per-scenario routines: `ctx.on_error_notice(window, ..)` reads the
notice (kind and text, `None` when nothing shows) and `ctx.clear_notice_with(..)` hides it.
Keep a weak handle of the current window in a shared slot, because `restore` builds a new window
every time. A state that is expected to warn lists the texts in `Smoke::allowed_notices`.

```rust
use std::cell::RefCell;
use std::rc::{Rc, Weak};

type NoticeSlot = Rc<RefCell<Option<Weak<NotesWindow>>>>;

fn register_notice(ctx: &mut Ctx<'_>, slot: &NoticeSlot) {
    let reader = Rc::clone(slot);
    ctx.on_error_notice("NotesWindow", move || {
        let window = reader.borrow().as_ref()?.upgrade()?;
        let kind = window.get_notice_kind().to_string();
        (!kind.is_empty()).then(|| (kind, window.get_notice_text().to_string()))
    });
    let clearer = Rc::clone(slot);
    ctx.clear_notice_with(move || {
        if let Some(window) = clearer.borrow().as_ref().and_then(Weak::upgrade) {
            window.set_notice_kind("".into());
        }
    });
}

// In the smoke restore, remember the new window: *slot.borrow_mut() = Some(window.as_weak());
let smoke = Smoke::new("startup").allowed_notices(["nothing saved"]);
```

## Deny rules from data

The default deny list (`DEFAULT_DENY`) skips controls whose label says delete, quit and the
like. Words your team keeps in a file become rules with `DenyRule::label_owned`; ids use
`DenyRule::id` (or `id_owned`). The list is a public field, so `smoke.deny = Vec::new()` drops
the defaults entirely. If a layer that covers a quarter of the window is not a modal backdrop,
`skip_blocked(false)` stops the pass from leaving the controls under it out.

```rust
let words = std::fs::read_to_string("tests/deny.txt").unwrap_or_default();
let rules: Vec<DenyRule> = words
    .lines()
    .filter(|line| !line.trim().is_empty())
    .map(|line| DenyRule::label_owned(DenyMatch::Contains, line.trim().to_owned(), "from deny.txt"))
    .collect();
let mut smoke = Smoke::new("startup").deny_rules(&rules).skip_blocked(false);
smoke.deny = Vec::new(); // no built-in list: deny.txt is the whole list
```

## Key chords

`ctx.press(&app, "Ctrl+S")` sends Slint `KeyPressed` and `KeyReleased` events to the window:
modifiers first, then the key, released in reverse. The accepted names (`Escape`, `Return`,
`Tab`, `Backspace`, `Delete`, the arrows, `F1`..`F12`, single characters) are in
`crates/slint-harness/src/keys.rs`. The events reach the focused element only, so give the
window a `forward-focus` target or call `focus()` on an element first; otherwise a handler on a
`FocusScope` never fires. A chord that does not parse is a `bad-chord` finding.

```rust
.scenario("shortcuts", |ctx| {
    let app = ctx.launch(MainWindow::new)?;
    // The window forwards focus to its editor (`forward-focus: editor;` in the .slint file).
    ctx.press(&app, "Ctrl+S");
    ctx.press(&app, "Escape");
    ctx.shutdown(app);
    Ok(())
})
```

## Reading report.json in CI

`report.json` is written on every run that does not abort. `findings[]` holds `window`,
`element`, `check`, `detail`, `state` and `repeats`; `scenarios[]` holds `name`, `findings` and
`duration_ms`; `per_check` maps a check name to its count; `notes` and `sections` carry the
text the smoke pass and the coverage gate add; `harness` and `slint` name the versions. The exit
code is 0 clean, 1 findings, 2 aborted. While a baseline is being built, run with
`SLINT_HARNESS_ALLOW_FAIL=1` (exit 0 with findings); `SLINT_HARNESS_FORMATS=txt` writes a plain
text file that is friendly to a CI log.

```text
$ SLINT_HARNESS_FORMATS=txt cargo test --features ui-harness --test ui
$ jq '.per_check' target/slint-harness/my-app/report.json
{ "overlap": 2, "not-found": 1 }
$ jq -r '.findings[] | "\(.check) \(.window): \(.detail) x\(.repeats)"' target/slint-harness/my-app/report.json
```

## Baselines across machines

Geometry depends on text metrics, and the testing backend measures text with the system fonts,
so the same app can differ between your laptop and a CI box (README, Limits). Fixtures that need
exact geometry set `default-font-family: "FixedTestFont"`. For a real app, compare the
`per_check` totals on one machine and keep one baseline per CI box, or install a pinned font
package on the runners.

```toml
# baseline-ci-linux.toml, kept next to the CI job that compares it with report.json
overlap = 0
zero-size = 0
missing-label = 3   # known: icon buttons, tracked in the issue tracker
```

## A custom SmokeHost

`ctx.smoke` fits a state that is "relaunch a window". When the state is something else (a wizard
page, an open document) implement `slint_harness::smoke::SmokeHost` yourself. It has six
required methods (`restore`, `views`, `notice`, `clear_notice`, `main_alive`, `report`) and the
optional `sweep_options`; `slint_harness::smoke::run(&mut host, &smoke)` runs the pass. Inside a
scenario `host.report()` is `ctx.report()`.

```rust
use slint_harness::smoke::SmokeHost;

struct WizardHost<'a, 'b> {
    ctx: &'a mut Ctx<'b>,
    app: Option<Attached>,
}

impl SmokeHost for WizardHost<'_, '_> {
    fn restore(&mut self) -> Result<Vec<WindowView>, String> {
        if let Some(old) = self.app.take() {
            self.ctx.shutdown(old);
        }
        let app = self.ctx.launch(Wizard::new).map_err(|f| f.detail)?;
        self.ctx.view(&app).click("Next").map_err(|f| f.detail)?; // go to page two
        let view = app.view();
        self.app = Some(app);
        Ok(vec![view])
    }
    fn views(&self) -> Vec<WindowView> {
        self.app.iter().map(Attached::view).collect()
    }
    fn notice(&mut self) -> Option<(String, String)> {
        None
    }
    fn clear_notice(&mut self) {}
    fn main_alive(&self) -> bool {
        self.app.as_ref().is_some_and(Attached::is_visible)
    }
    fn report(&mut self) -> &mut Report {
        self.ctx.report()
    }
}

// In a scenario:
let mut host = WizardHost { ctx, app: None };
let stats = slint_harness::smoke::run(&mut host, &Smoke::new("wizard page 2"));
```

## Filtering scenarios while developing

`SLINT_HARNESS_FILTER=open,save` runs the scenarios whose names contain `open` or `save`;
positional arguments that do not start with `-` filter the same way. Under a filter the coverage
gate is skipped, because the scenarios that would reach the rest did not run; give the gate
`GateRule::Always` to run it anyway. Beware `cargo test <name>`: cargo hands `<name>` to the
harness binary as a scenario filter, and when nothing matches the run does nothing and writes
no report (README, Troubleshooting).

```text
$ SLINT_HARNESS_FILTER=open,save cargo test --features ui-harness --test ui
slint-harness 0.18.0 (Slint 1.18.1): 2 of 7 scenario(s) selected, report dir target/slint-harness/my-app
```

## Guarded paths for apps that use the dirs crate

The profile redirects `APPDATA` and the XDG variables, but on Windows the `dirs` crate asks the
Known Folders API, which ignores them. Under the feature, ask the harness first with
`slint_harness_hooks::paths::data_dir()` and fall back to the real lookup otherwise, in one
function that the app and the guard share. Then hand that function to the profile: a path that
still resolves outside the profile aborts the run with exit code 2 before a window exists.

```rust
pub fn settings_path() -> PathBuf {
    #[cfg(feature = "ui-harness")]
    if let Some(dir) = slint_harness_hooks::paths::data_dir() {
        return dir.join("settings.txt");
    }
    dirs::config_dir().unwrap_or_default().join("my-app").join("settings.txt")
}

// tests/ui.rs
Harness::new()
    .profile(Profile::isolated().guard_paths(|| vec![my_app::settings_path()]))
```
