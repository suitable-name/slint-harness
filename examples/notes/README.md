# notes

A small Slint notes app that uses every `slint-harness` seam. Its test (`tests/ui.rs`) is the
reference for an app with file dialogs, a clipboard, an OS opener, an OS-resolved settings
directory and an error notice.

Run `cargo test -p notes --features ui-harness --test ui`, then open
`target/slint-harness/notes/report.html` (`report.md`, `report.txt` and `report.json` are next
to it).

What is where:

- `ui/notes.slint`: the window (`NotesWindow`) and the `Model` global whose callbacks are
  handled in Rust. `FixedTestFont` gives the same geometry on every OS.
- `src/lib.rs`: `build()` wires the callbacks; one function per seam with the feature-on arm
  (the harness's queue or capture) and the feature-off arm (the OS; printing stubs here, so the
  app never opens a real dialog or touches the network).
- `build.rs`: Slint debug info on under the `ui-harness` feature.
- `tests/ui.rs`: seven scenarios (see its module doc), the click-everything pass with a deny
  word and an expected notice, the coverage gate, a guarded settings path, all report formats.
- `tests/coverage_allow.toml`: the two items no scenario drives (Quit), each with a reason.

A plain `cargo build -p notes` compiles the feature-off arms; no harness crate is involved.
