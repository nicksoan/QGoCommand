# QGo

The standalone Rust implementation of the QGo keyboard-driven shortcut
launcher.

It can reuse profiles created by earlier QGo versions: the application reads
and writes `%LOCALAPPDATA%\QGo` using the existing JSON format. This is a data
compatibility contract only; building and running this project does not require
the earlier .NET codebase or runtime.

## Layout

```
QGo.Rust/
├── qgo-core/     models, storage, expansion, launching, stats — no GUI
└── qgo-app/      the launcher window, hotkey, tray, secondary windows
```

Splitting the crate in two is the one structural change from the C#, where
`MainViewModel.cs` owns the collection view, the brushes, the dialogs *and* the
launch logic. Here `qgo-core` is pure behaviour, so it can be tested — the WPF
project has no automated tests at all. The workspace includes unit tests plus
four read-only checks against a real profile.

## Build and run

```powershell
cd QGo.Rust
cargo run                       # debug, with a console for logs
cargo build --release           # target\release\qgo.exe
cargo test --workspace
cargo clippy --workspace
```

The release profile sets `windows_subsystem = "windows"`, so there is no console
window — matching `<OutputType>WinExe</OutputType>`.

To check the port against a real profile written by the WPF app:

```powershell
cargo test -p qgo-core --test live_profile -- --ignored --nocapture
```

Those tests are read-only and are ignored by default because they need a machine
that has actually used QGo.

To exercise it without touching your real data, point `LOCALAPPDATA` somewhere
scratch:

```powershell
$env:LOCALAPPDATA = "$env:TEMP\qgo-scratch"; cargo run
```

### Diagnosing a rendering problem

`--diagnose` logs the resolved geometry, DPI scale and theme colours, then dumps
the actual framebuffer as coarse ASCII luminance and exits:

```powershell
$env:RUST_LOG = "info"; cargo run -- --diagnose
```

A correct launcher looks like a rounded dark slab with the logo bottom-left of
centre and the hint text beside it. An all-`.` dump means nothing was painted;
an all-`:` dump means the shell painted but its contents did not.

## Stack

| Concern | C# | Rust |
|---|---|---|
| Window | WPF | `eframe` / `egui` 0.35 |
| Global hotkey | `RegisterHotKey` + `WndProc` hook | `global-hotkey` |
| Tray | `System.Windows.Forms.NotifyIcon` | `tray-icon` |
| Launching | `Process.Start(UseShellExecute = true)` | `ShellExecuteW` via `windows` |
| Start with Windows | `Microsoft.Win32.Registry` | `winreg` |
| JSON | `System.Text.Json` | `serde_json` + adapters in `serde_cs` |

`egui` was chosen over Tauri to avoid carrying a whole webview for an app whose
primary UI is one text box.

### The hidden-window problem

QGo spends most of its life invisible, waiting on a hotkey. eframe 0.35 splits a
frame into `logic()` and `ui()`, and `logic()` runs **even while the window is
hidden** whenever `Context::request_repaint` is called from any thread. That is
what makes the design work: the hotkey thread calls `request_repaint`, `logic()`
wakes, sees the press, and issues `ViewportCommand::Visible(true)`. Verified
empirically before any of this was written.

## File-format compatibility

The awkward parts of matching `System.Text.Json` live in `qgo-core/src/serde_cs.rs`:

* **PascalCase members.** No `PropertyNamingPolicy` is configured in
  `Storage.Opt`, so the on-disk names are the C# property names.
* **`"NaN"` as a JSON string.** `Storage.Opt` sets
  `NumberHandling = AllowNamedFloatingPointLiterals`, and `WindowLeft` /
  `WindowTop` / `WindowHeight` default to `double.NaN`. A plain `f64` field
  would fail to parse a fresh `settings.json`.
* **Integer enums.** No `JsonStringEnumConverter` is registered, so
  `ExecutionMode` is `0` / `1` on disk, not `"Sequential"`.
* **C# `DateTime` formatting.** Up to 7 fractional digits with trailing zeros
  trimmed; `Z` for `UtcNow` values, a numeric offset for `Now` values, and
  `DateTime.MinValue` written bare as `0001-01-01T00:00:00`.

One cosmetic difference: `serde_json` writes integral doubles as `18.0` where
C# writes `18`. Both parse either.

## Quirks preserved on purpose

These look like bugs. They are reproduced so both builds resolve a template the
same way; changing them is a product decision, not a porting one.

* `Expand()` URL-encodes the parameter **always**, even when the target is a
  file path or a shell command.
* Only the **first** `{...}` pair in a template is substituted; a second
  placeholder survives verbatim into the launched target.
* A template with no braces drops the parameter entirely.
* The seeded default `notepad` shortcut is the bare string `notepad.exe`, which
  is neither an absolute URI nor an existing file, so it ends up at the
  `explorer /select` fallback and does nothing useful. Same in the WPF app.
* `LocalUsageStatsService` carries its own copy of `DetermineShortcutType` that
  — unlike `MainViewModel`'s — does not recognise `cmd:` / `ps:` targets, so the
  two files bucket shell commands differently. Both copies are ported as-is
  (`telemetry::determine_shortcut_type_basic` vs `command::determine_shortcut_type`).
* `RecordChainItemExecution` reconstructs its success count from the stored
  percentage instead of keeping a counter, so the chain success rate drifts.
## Deliberate differences

* **Single instance.** The C# takes a named mutex, then walks the process list
  for another QGo with a `MainWindowHandle` to bring forward. That cannot work
  when the window is hidden, which is most of the time. This port has the running
  instance park a thread on a named event; a second launch signals it and exits.
* **Saving.** `Storage` writes in place; this port writes to a temporary file and
  renames, so an interrupted save cannot truncate `links.json`.
* **Launching is split in two.** `LauncherSession::prepare_launch` resolves the
  target and updates the counters; `PendingLaunch::run` does the blocking part.
  The split exists because a sequential chain sleeps for the sum of its delays,
  and the render loop takes the same lock every frame — holding it across those
  sleeps would freeze the window for the length of the chain.
* **Unknown members are preserved.** `AppSettings`, `Shortcut`,
  `ChainedShortcut` and `ShortcutChainItem` each carry a `#[serde(flatten)]`
  catch-all, so running this build against a profile written by a newer
  `QGo.App` does not delete settings it has never heard of. Serde would
  otherwise drop them silently, and the shared-profile promise above would only
  hold in one direction.
* **The Settings window only writes what it shows.** It re-reads
  `settings.json` on Save and folds its own fields onto that, preserving fields
  owned by other parts of the application.
* **Chain proxies.** Chained shortcuts are not editable in Manage Shortcuts —
  they are owned by `chained-shortcuts.json` and rebuilt on load — so that window
  filters them out and hands them to the chains editor.

## Not ported

Stated explicitly rather than quietly dropped.

* **Remote telemetry** (`Services/TelemetryService.cs`). Local usage statistics
  are fully ported and `usage-stats.json` stays compatible; nothing is sent to
  the Azure Functions backend. The consent setting is still honoured and stored.
* **Automatic update downloads.** The first-run-after-version-change
  notification and changelog window are ported, but remote update discovery,
  download, and installer integration are not implemented.
* **Welcome window** (`Views/WelcomeWindow.xaml`, 1255 lines of onboarding) and
  the **telemetry consent window**.
* **`AccessibilityService`** sound feedback and screen-reader announcements. The
  settings are read, stored and round-tripped, and font scaling and high
  contrast are applied; the announcement plumbing has no egui equivalent.
## Verified

* The workspace test suite passes, and clippy is clean.
* The app starts, registers `Alt+Q`, and creates a byte-compatible profile from
  scratch.
* The logo acts as the window drag handle, borderless edge resizing is enabled,
  and the resulting position and width are saved when the launcher hides.
* Right-clicking the launcher, or pressing F10, opens the original launcher
  menu. Resizing is horizontal while height continues to follow the results.
* A second launch exits and signals the first instance.
* The four `live_profile` tests parse this machine's real WPF-written profile
  and round-trip its shortcuts, chains, settings, and usage statistics.

Beyond that, the UI has not been driven by hand. Specifically unverified, and
worth checking first:

* the popup renders with the rounded, transparent shell;
* Tab autocompletes rather than moving focus between widgets;
* Up/Down reveals the most-used list and scrolls the selection into view within
  the 360px cap;
* each tray item opens its window **while the launcher is hidden** — that path
  goes through `ui()`'s early-return branch;
* an immediate viewport keeps repainting on its own input while the root
  viewport is invisible;
* `WindowLeft` / `WindowTop` are reapplied when the window is shown by hotkey.
