# Repository Guidelines

## Project Overview
`kvm-switch` is a Rust desktop app for macOS, Linux and Windows. A hotkey, tray click or CLI request switches two things together: the monitor input (DDC/CI VCP `0x60`) and the Logitech Easy-Switch channel (HID++ 2.0 feature `0x1814` CHANGE_HOST over a Logi Bolt receiver `046D:C548`).
- The GPUI windows offer a settings view and a ⌘Tab-style "switcher" overlay.
- `linux/` and `macos/` hold the **legacy shell/Python/Hammerspoon version**. It still works on its own and installs separately from the app.
- All identifiers, comments, docs, user-facing strings and error messages are **Brazilian Portuguese**. Keep new code in Portuguese too; test function names are the only exception and stay in English.

## Architecture & Data Flow
The code follows hexagonal (ports and adapters) layering, described in the `src/main.rs` module doc:
- `domain.rs` holds the OS-free rules:
  - Types: `InputCode(u8)` (serde-transparent decimal VCP value), `Channel {One,Two,Three}` (stored as 1–3, `host_index()` is 0-based), `Target`, `Config`, `Cycle`, `Outcome`.
  - Use case: `Kvm { Arc<dyn Monitor>, Arc<dyn Peripherals> }` with `detect()` and `switch()`.
- `ports.rs` defines the traits each OS must provide: `Monitor`, `Peripherals` (both `Send + Sync`), `Hotkeys` and `StatusMenu`, plus `enum Action`.
- `adapters.rs` does the wiring:
  - `Platform { monitor, peripherals, hotkeys, status_menu }`.
  - Each OS has its own `#[cfg(target_os)]` versions of `first_action`, `devices`, `platform` and `hotkeys`. Any other OS hits `compile_error!`.
  - The `run(program, args)` helper runs external commands.
- `app.rs`: `AppState` is a GPUI entity. Its `handle(Action)` dispatches every action.
- `ui/`: `settings.rs` (settings view) and `overlay.rs` (switcher and HUD).
- `request.rs`: the text protocol shared by the CLI args, the Linux socket and the tray item ids (`arguments(Action)` ↔ `parse(&str)`).

Flow:
1. Every input becomes an `Action` on one `async_channel`. The sources are a hotkey, a tray click, a CLI argument or the Linux socket.
2. The GPUI loop calls `AppState::handle`.
3. `Kvm::switch` runs on `background_executor`.
4. The outcome comes back as a HUD message or a status line.

When `switch` is requested while the app is closed, it runs headless (`main.rs::switch_without_window`). GPUI starts only if a notice is left to show.

Platform specifics:
- **macOS**
  - `adapters/macos.rs`: monitor via `m1ddc`; inputs listed via DDPM (`/Applications/DDPM/DDPM`); hotkeys via `global-hotkey`; tray via `tray-icon`, created on the main thread.
  - `adapters/logi_bolt.rs`: peripherals via `hidapitester`.
  - `run()` prepends Homebrew to `PATH`, because Dock-launched apps don't inherit the shell `PATH`.
- **Linux**
  - `adapters/linux.rs`: `ddcutil` with the bus cached in `~/.cache/kvm-switch/ddc-bus`.
  - `linux/hidraw.rs`: raw HID++ to `/dev/hidrawN`, one thread per slot, `libc::poll`.
  - `linux/gnome.rs`: GNOME custom keybindings via `gsettings`, which run `kvm-switch <request>`.
  - `linux/instance.rs`: single instance via a Unix socket at `$XDG_RUNTIME_DIR/kvm-switch.sock`.
  - `linux/tray.rs`: `kvm-switch tray` runs as a separate process without GPUI.
- **Windows**
  - `adapters/windows.rs`: monitor via `ddc-hi` and the Windows Monitor Configuration API; peripherals via `hidapi`'s native Windows backend; hotkeys via `global-hotkey`; tray via `tray-icon`.

**Invariants (covered by domain tests; don't break them):**
- The monitor switches first. Peripherals move only if the monitor succeeded.
- When the target is the local input, peripherals are untouched. The result is `AlreadyHere` if the monitor is already on that input.
- `Config::merge_inputs` never removes or edits existing targets.
- On Linux, GPUI quits with its last window:
  - Open the new HUD or switcher before closing the old one.
  - `app::finish_switches()` (the `IN_FLIGHT` guard) waits for pending switches.

## Key Directories
| Path | Purpose |
|---|---|
| `src/` | Rust app (domain, ports, app, ui, request, tray, config) |
| `src/adapters/` | Per-OS implementations: `macos.rs`, `logi_bolt.rs` (macOS), `linux.rs` + `linux/` (hidraw, gnome, instance, tray), `mccs.rs` (shared capability parser) |
| `src/adapters/windows.rs` | Windows DDC/CI, Logitech HID++, global hotkeys and status icon |
| `src/ui/` | GPUI views: `settings.rs`, `overlay.rs` |
| `assets/icons/` | 128 px Noto Emoji PNGs named by codepoint, embedded with `include_bytes!` |
| `linux/`, `macos/` | Legacy scripts and installers (see their READMEs) |
| `vendor/xattr/` | Patched xattr 0.2.3; don't edit (see below) |

## Development Commands
```sh
cargo test                                   # unit + GPUI tests (adapter tests only on their own OS)
cargo clippy --all-targets -- -D warnings    # the lint gate; warnings are errors only here
cargo run --release                          # macOS dev; needs: brew install m1ddc hidapitester
macos/bundle.sh                              # macOS installer: target/release/KVM Switch.app + KVM-Switch-<version>.dmg (ad-hoc signed)
linux/install.sh --app                       # Linux: apt deps + cargo install --root ~/.local + .desktop/autostart
linux/deb.sh                                 # Linux package: target/release/kvm-switch_<version>_<arch>.deb (dpkg-deb, deps via dpkg-shlibdeps)
windows/build-installer.ps1                  # Windows: target/release/KVM-Switch-Setup-<version>.exe (requires Inno Setup)
./release.sh X.Y.Z                           # bump Cargo.toml/.lock, commit, tag vX.Y.Z, push → CI builds .dmg + .deb + Windows installer and publishes a GitHub release
```
App CLI: `kvm-switch [open | switch <decimal VCP> | switcher-next | switcher-previous | quit | tray]`. Exit code `2` (`EXIT_REFUSED`) means the request was refused or did not reach the running app.

## Code Conventions & Common Patterns
- **Lints** (`Cargo.toml` `[lints]`):
  - No `.unwrap()`; use `.expect("…")` with a Portuguese message, also in tests.
  - No `todo!` and no `dbg!`.
  - Every `#[allow]`/`#[expect]` needs `reason = "…"`.
  - Every `unsafe` block needs a `// SAFETY:` comment.
  - `unsafe_op_in_unsafe_fn` is on.
- **Errors:**
  - Use `anyhow` with `.context()` / `.with_context()`. There is no `thiserror`.
  - Show errors to the user with `format!("{error:#}")`.
  - Best-effort calls end in `.ok()` with a comment explaining why.
  - Lock poisoning is handled with `.expect("lock … envenenado")`.
- **DI:**
  - Trait objects only: `Arc<dyn …>` when shared across threads, `Box<dyn …>` when owned by the UI thread.
  - Build them in `adapters::platform` / `adapters::devices`. No generic DI.
- **Concurrency:**
  - The GPUI main thread owns all state (`Entity<AppState>`, `cx.spawn`, `cx.background_executor()`).
  - OS callbacks send `Action`s with `sender.try_send(..).ok()`.
  - Everything else uses plain `std::thread` / `thread::scope`.
- **Comments:**
  - Every module and item gets a doc comment explaining *why*.
  - Mark a known limit as `// shortcut: <limit>, <when to upgrade>`.
- **Config compatibility:** new `Config`/`Target` fields need `#[serde(default)]`, plus a test that an old file still loads (see `file_from_before_the_active_flag_loads_every_target_active`).
- **Adding an icon:** three steps (see `assets/icons/README.md`):
  1. Add the PNG to `assets/icons/`.
  2. Add the emoji to `ICONS` in `src/ui/settings.rs`.
  3. Add its row to the Linux `mod icons` map in `src/ui.rs`.
- **Adding an OS:** create a new adapter module implementing the ports. Add cfg-gated `devices`/`hotkeys`/`first_action`/`platform` in `adapters.rs` and a target-specific dependency section in `Cargo.toml` (`README.md` covers this).
- **File table:** `README.md` maps each `src/` file to its role. Update it when files are added or renamed.

## Important Files
- `src/main.rs`: entry point. Handles the Linux `tray` branch, headless switching, `start()` (channel, `Platform`, `AppState`).
- `src/domain.rs` and `src/ports.rs`: the core rules and contracts. Read these first.
- `src/config.rs`: `<config_dir>/kvm-switch/config.json`, pretty JSON.
  - Path: `~/Library/Application Support/…` on macOS, `~/.config/…` on Linux, `%APPDATA%\…` on Windows.
  - Atomic save: write `config.json.parcial`, then rename.
  - An invalid file is renamed to `config.json.invalido`; a missing file loads as the default config.
- `src/request.rs`: CLI and IPC protocol. Keep `arguments` and `parse` symmetric.
- `Cargo.toml`: deps, lints, the `[patch.crates-io]` entry for xattr.
- `linux/70-logi-bolt.rules`: an optional udev `uaccess` rule for the receiver's hidraw device.

## Runtime/Tooling Preferences
- Rust edition 2024, Cargo only. There is no `build.rs`, toolchain file or `.cargo/config`. The only CI is the tag-triggered release workflow; it runs no clippy, and `cargo test` only in the Windows job.
- Each OS builds natively; there is no cross-compiling. Code under `cfg(target_os)` compiles only on its own target, so a change to one adapter can't be checked from the other OS.
- `gpui 0.2.2`:
  - The `runtime_shaders` feature means the full Xcode isn't needed.
  - Tests need `features = ["test-support"]`, which is already in dev-deps.
- `tray-icon` uses `ksni` (D-Bus StatusNotifierItem) on Linux, native backends on macOS/Windows; there is no GTK dependency.
- External tools needed at runtime:
  - macOS: `m1ddc`, `hidapitester`, DDPM.
  - Linux: `ddcutil`, `gsettings`, and access to `/dev/i2c-*` and `/dev/hidraw*`.
  - Windows: none; enable DDC/CI in the monitor menu and connect the Bolt receiver.
- `vendor/xattr` works around gpui → zed-async-tar → xattr 0.2.3, which fails on Linux with libc ≥ 0.2.190.
  - The patch is a single `ENOATTR` constant change, marked `// kvm-switch:`.
  - Remove the patch once gpui drops the dependency.
- The hardware the code assumes:
  - Monitor: Dell P3225QE, with inputs USB-C `27`, DP `15`, HDMI `17`.
  - Peripherals: MX Mechanical Mini and MX Master 3S on Bolt receivers.

## Testing & QA
- Tests use the built-in harness only, in inline `#[cfg(test)] mod tests`. There is no `tests/` directory and no mock crate.
- **Domain:** a hand-written `Fake` in `src/domain.rs` implements `Monitor` + `Peripherals` and records calls in order. Use it to check switch ordering and failure paths.
- **Parsers:** fixtures are `const` strings or byte arrays copied from real tool or hardware output (ddcutil 2.2.5, m1ddc, DDPM, hidapitester, gsettings, hidraw). Add new fixtures from real captures and note their source.
- **UI:** `src/ui/settings.rs` uses `#[gpui::test]` with `TestAppContext`.
  - Fakes: `NoHardware` and `RecordingHotkeys`.
  - Elements are found by `debug_selector` ids such as `"Name(0)"`, `"Hotkey(0)"`, `"Enabled(0)"` and `"SwitcherHotkey"`.
- **Filesystem tests:** temp dirs `std::env::temp_dir()/kvm-switch-test-{pid}-{name}`.
- **Untested:**
  - `app.rs`, `ui/overlay.rs`, `adapters.rs` wiring, `linux/instance.rs`, `linux/tray.rs`.
  - Real hardware I/O.
  - All shell scripts (there is no shellcheck setup).
- A full run needs the supported OS and hardware. Before finishing, run `cargo test` and the clippy gate on the OS you changed.
