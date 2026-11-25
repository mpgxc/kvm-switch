# Repository Guidelines

## Project Overview
`kvm-switch` is a Rust desktop app for macOS, Linux and Windows. It has two independent modes plus a clipboard mode that rides on Bordas, all toggled in the settings window:
- **Monitor compartilhado** (`Config.monitor_mode`, on by default): a hotkey, tray click or CLI request switches two things together: the monitor input (DDC/CI VCP `0x60`) and the Logitech Easy-Switch channel (HID++ 2.0 feature `0x1814` CHANGE_HOST over a Logi Bolt receiver `046D:C548`).
- **Bordas** (`Config.edges`, off by default): PCs side by side, each with its own screen and each running the app. When the cursor touches an outer edge of the desktop on a side that has a neighbor, only the keyboard and mouse go to that neighbor's channel. No network for the devices: the other PC's app sends them back the same way.
- **Área de transferência** (`Config.clipboard`, off by default, needs Bordas): on an edge crossing, the clipboard (text, else PNG) goes encrypted over TCP port 24811 to the neighbor's `address` (`.local` name or IPv4; the listener binds IPv4 only). This is the only network traffic.
- The GPUI windows offer a settings view and a ⌘Tab-style "switcher" overlay.
- `linux/` and `macos/` hold the **legacy shell/Python/Hammerspoon version**. It still works on its own and installs separately from the app.
- All identifiers, comments, docs, user-facing strings and error messages are **Brazilian Portuguese**. Keep new code in Portuguese too; test function names are the only exception and stay in English.

## Architecture & Data Flow
The code follows hexagonal (ports and adapters) layering, described in the `src/main.rs` module doc:
- `domain.rs` holds the OS-free rules:
  - Types: `InputCode(u8)` (serde-transparent decimal VCP value), `Channel {One,Two,Three}` (stored as 1–3, `host_index()` is 0-based), `Target`, `Config`, `Cycle`, `Outcome`, `Side` (`word()` = `esquerda|direita|cima|baixo`), `Neighbor { channel, name, address }`, `Edges`, `ClipboardMode { enabled, key }`. `Config::clip_destination(side)` and `clip_key()` return `Some` only when the modes, channel, address and key all allow it.
  - Edge geometry, OS-free and tested: `Rect`, `edge_at(x, y, screens)` (outer edges only, 8 px corners ignored) and `EdgeWatch` (an edge fires only after the cursor has left it, so devices coming back don't bounce away).
  - Use case: `Kvm { Arc<dyn Monitor>, Arc<dyn Peripherals> }` with `detect()`, `switch()` and `cross(&Edges, Side)`.
- `ports.rs` defines the traits each OS must provide: `Monitor`, `Peripherals`, `Clipboard` (all `Send + Sync`; `Clipboard::read` gives text, else PNG), `Hotkeys`, `StatusMenu` and `EdgeSensor`, plus `enum Action` (including `CrossEdge(Side)` and `Start`, the window-less start used by the macOS login item and the Windows startup shortcut).
- `clip.rs`: the OS-free clipboard core: `Clip { Text, Png }`, `Key` (32 random bytes in base64; `generate`/`parse`), the frame and `NoticeGate`. Frame: `"KVMC"` + version + 12-byte nonce + `u32` BE length + ChaCha20-Poly1305 body (`ring`). The receiver sends a fresh 16-byte challenge on each connection, and the AEAD associated data is `"KVMC"` + version + that challenge, so a captured frame is useless on any other connection. No clock window, no NTP.
- `adapters/net.rs`: `share` (connect, read the challenge, send one frame; 300 ms connect, 300 ms challenge read, 5 s write, `SEND_DEADLINE` 5.6 s) and `listen`/`relisten` (a thread on `0.0.0.0:24811`; `relisten` restarts only when the key changed and stops when `clip_key()` is `None`). A wrong key notifies at most once a minute.
- `adapters.rs` does the wiring:
  - `Platform { monitor, peripherals, hotkeys, status_menu, edge_sensor, clipboard }`.
  - Each OS has its own `#[cfg(target_os)]` versions of `devices`, `hotkeys`, `status_menu`, `edge_sensor` and `clipboard`; `first_action` and `listen` come from the shared `instance` module. Any other OS hits `compile_error!`.
  - The `run(program, args)` helper runs external commands.
- `app.rs`: `AppState` is a GPUI entity. Its `handle(Action)` dispatches every action. `send_clipboard` starts the send on its own thread when an edge is crossed; `AppState::refresh_clipboard` keeps the listener in step with the config (macOS/Windows; a no-op on Linux).
- `ui/`: `settings.rs` (settings view) and `overlay.rs` (switcher, HUD and the yes/no `Question` dialog: Enter = yes, Esc = no; the caller closes it).
- `request.rs`: the text protocol shared by the CLI args, the instance socket/pipe and the tray item ids (`arguments(Action)` ↔ `parse(&str)`).
- `adapters/instance.rs`: one app per session on every OS. `first_action()` returns a `Launch`: `Delivered` (sent to the open app), `Run(action)`, `AlreadyOpen` (a manual open with an app open: ask "encerrar e abrir de novo?") or `ConfirmQuit` (`kvm-switch quit`: ask here, then send `quit-now`; exit code 2 on "no"). Shortcuts, edges and CLI requests go straight to the open app. Transport: `instance/socket.rs` (Unix socket; `$XDG_RUNTIME_DIR` on Linux, `~/Library/Caches/kvm-switch` on macOS) or `instance/pipe.rs` (named pipe `\\.\pipe\kvm-switch-<USERNAME>` on Windows).

Flow:
1. Every input becomes an `Action` on one `async_channel`. The sources are a hotkey, a tray click, a CLI argument or another run of the program (through `instance`).
2. The GPUI loop calls `AppState::handle`.
3. `Kvm::switch` runs on `background_executor`.
4. The outcome comes back as a HUD message or a status line.

When `switch` or `edge` is requested while the app is closed, it runs headless (`main.rs::switch_without_window` / `cross_without_window`). GPUI starts only if a notice is left to show. `cross_without_window` starts the clipboard send before crossing and waits for it up to `SEND_DEADLINE` before exiting.

Platform specifics:
- **macOS**
  - `adapters/macos.rs`: monitor via `m1ddc`; inputs listed via DDPM (`/Applications/DDPM/DDPM`); hotkeys via `global-hotkey`; tray via `tray-icon`, created on the main thread; `cursor_and_screens` reads the cursor and display bounds through Quartz (no permission needed) for `adapters/cursor.rs`.
  - `adapters/macos/clipboard.rs`: `MacClipboard` over `NSPasteboard` (`objc2*`), fetched on each call so it is `Send + Sync` off the GPUI thread.
  - `adapters/logi_bolt.rs`: peripherals via `hidapitester`. `LogiBolt` caches slot → CHANGE_HOST index (the cache only grows); later switches send only `setCurrentHost` (~20 ms) and a background thread queries the uncached slots. The app warms the cache at start.
  - Login item: `register_login_item` writes a LaunchAgent (`io.github.mpgxc.kvm-switch`) running `open -g -a <app> --args start`, only when the app runs from `/Applications` or `~/Applications`.
  - `run()` prepends Homebrew to `PATH`, because Dock-launched apps don't inherit the shell `PATH`.
- **Linux**
  - `adapters/linux.rs`: `ddcutil` with the bus cached in `~/.cache/kvm-switch/ddc-bus`.
  - `linux/hidraw.rs`: raw HID++ to `/dev/hidrawN`, one thread per slot, `libc::poll`.
  - `linux/gnome.rs`: GNOME custom keybindings via `gsettings`, which run `kvm-switch <request>`.
  - Edges: a GNOME Shell extension (GNOME 45+, uuid `kvm-switch@mpgxc.github.io`, source in `linux/gnome-extension/`) puts pressure barriers on the outer edges and runs `kvm-switch edge <lado>`. `linux/edges.rs` (`GnomeEdges`) only enables it (`gnome-extensions enable`, falling back to `org.gnome.shell enabled-extensions` when the shell hasn't loaded it yet) and reports whether it is ACTIVE. Installed by `install.sh --app` (`~/.local/share/gnome-shell/extensions/`) and `deb.sh` (`/usr/share/gnome-shell/extensions/`); on Wayland it loads only after logging out and in.
  - Single instance is shared code now: see `adapters/instance.rs` above.
  - `linux/tray.rs`: `kvm-switch tray` runs as a separate process without GPUI. It also owns the clipboard listener on Linux (it outlives the app), re-checked with the config; wrong-key notices go out as GNOME notifications.
  - `linux/clipboard.rs`: `GnomeClipboard` calls the extension over D-Bus (`zbus` blocking): destination `org.gnome.Shell`, path `/io/github/mpgxc/KvmSwitch`, interface `io.github.mpgxc.KvmSwitch`, methods `Ler` (→ kind `texto`/`png`/`""` + bytes), `Gravar(kind, bytes)` and `Avisar(text)`; 2 s timeout per call. On Wayland only the shell may touch the clipboard of an unfocused app.
- **Windows**
  - `adapters/windows.rs`: monitor via `ddc-hi` and the Windows Monitor Configuration API; peripherals via `hidapi`'s native Windows backend; hotkeys via `global-hotkey`; tray via `tray-icon`; `cursor_and_screens` reads `GetCursorPos` and `EnumDisplayMonitors` (virtual-screen coordinates) for `adapters/cursor.rs`.
  - `adapters/windows/clipboard.rs`: Win32 clipboard: `CF_UNICODETEXT`, the registered `"PNG"` format and `CF_DIB` (written too, since many apps paste only DIB). `adapters/dib.rs` converts DIB ↔ PNG (compiled on Windows and in tests).
  - The installer's startup shortcut runs `kvm-switch.exe start` (window-less, like the macOS login item).
- **macOS and Windows edges:** `adapters/cursor.rs` (`PollingEdges`) polls the OS `cursor_and_screens` every 20 ms on its own thread and feeds `edge_at` + `EdgeWatch`; crossings arrive as `Action::CrossEdge`.
- **Clipboard listener:** macOS/Windows: the app (`AppState::refresh_clipboard` → `net::relisten`). Linux: the tray process. Receiving needs that process running.

**Invariants (covered by domain tests; don't break them):**
- The monitor switches first. Peripherals move only if the monitor succeeded.
- When the target is the local input, peripherals are untouched. The result is `AlreadyHere` if the monitor is already on that input.
- An edge crossing never touches the monitor (`Kvm::cross` calls only `Peripherals`).
- `Action::Quit` always asks first (in the app, or in the `kvm-switch quit` run); only `Action::QuitNow`, sent by a run that already asked, quits without asking.
- With `monitor_mode` off: no hotkeys are registered, the switcher doesn't open, `SwitchTo` only shows the `MONITOR_MODE_OFF` notice, and the tray has no "Ir para …" items.
- `Config::merge_inputs` never removes or edits existing targets.
- On Linux, GPUI quits with its last window:
  - Open the new HUD or switcher before closing the old one.
  - `app::finish_switches()` (the `IN_FLIGHT` guard) waits for pending switches.
- One frame per connection, bound to that connection's challenge; a replayed frame writes nothing.

**Clipboard invariants (app wiring, not covered by tests; don't break them):**
- An edge crossing never waits for the clipboard send (it runs on its own thread; the GPUI path doesn't await it). The send doesn't hold `InFlight`, so `finish_switches()` doesn't wait for it.
- The receiver writes only content different from what its clipboard already has (`should_write`), so a round trip keeps the original rich copy.
- The sender keeps no state; each crossing reads the clipboard fresh.

## Key Directories
| Path | Purpose |
|---|---|
| `src/` | Rust app (domain, ports, app, ui, request, tray, config) |
| `src/adapters/` | Per-OS implementations: `macos.rs` + `macos/` (clipboard), `logi_bolt.rs` (macOS), `linux.rs` + `linux/` (hidraw, gnome, edges, instance, tray, clipboard), `windows.rs` + `windows/` (clipboard), `mccs.rs` (shared capability parser), `cursor.rs` (edge polling shared by macOS and Windows), `net.rs` (clipboard network, shared), `dib.rs` (DIB ↔ PNG) |
| `src/adapters/windows.rs` | Windows DDC/CI, Logitech HID++, global hotkeys, status icon and cursor/monitor sampling for the edges |
| `src/ui/` | GPUI views: `settings.rs`, `overlay.rs` |
| `assets/icons/` | 128 px Noto Emoji PNGs named by codepoint, embedded with `include_bytes!` |
| `linux/`, `macos/` | Legacy scripts and installers (see their READMEs) |
| `linux/gnome-extension/` | GNOME Shell extension (ESM, 45+) for the edges; shipped by `install.sh --app` and `deb.sh` |
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
App CLI: `kvm-switch [open | switch <decimal VCP> | edge <esquerda|direita|cima|baixo> | switcher-next | switcher-previous | start | quit | tray]`. Exit code `2` (`EXIT_REFUSED`) means the request was refused or did not reach the running app.

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
- **Config compatibility:** new `Config`/`Target` fields need `#[serde(default)]`, plus a test that an old file still loads (see `file_from_before_the_active_flag_loads_every_target_active` and `file_from_before_the_modes_keeps_the_monitor_and_leaves_edges_off`). `Config` has a hand-written `Default` because `monitor_mode` defaults to `true`.
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
- Rust edition 2024, Cargo only. There is no `build.rs`, toolchain file or `.cargo/config`. The only CI is `.github/workflows/release.yml`: it builds the `.dmg`, `.deb` and Windows installer on every pull request and on manual runs (`workflow_dispatch`), and also publishes the GitHub release on a `v*` tag. It runs no clippy, and `cargo test` only in the Windows job.
- Each OS builds natively for releases. Code under `cfg(target_os)` compiles only on its own target, but other targets can be checked from a Mac:
  - Windows: `cargo install cargo-xwin`, `rustup component add llvm-tools`, put a `llvm-lib` symlink to the toolchain's `llvm-ar` on `PATH`, then `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` (check only; tests need Windows).
  - Linux: Docker `rust:1` with the GPUI dev packages from `linux/install.sh`, the repo mounted and a separate `CARGO_TARGET_DIR`; `cargo test` and clippy run there.
- `gpui 0.2.2`:
  - The `runtime_shaders` feature means the full Xcode isn't needed.
  - Tests need `features = ["test-support"]`, which is already in dev-deps.
- `tray-icon` uses `ksni` (D-Bus StatusNotifierItem) on Linux, native backends on macOS/Windows; there is no GTK dependency.
- Clipboard deps: `ring` (ChaCha20-Poly1305, random) and `base64` everywhere; `objc2`, `objc2-foundation`, `objc2-app-kit` on macOS; `zbus` (`blocking-api`) on Linux; extra `windows-sys` features (`Win32_System_DataExchange`, `_Memory`, `_Ole`) on Windows. All were already in `Cargo.lock` through GPUI or ksni.
- External tools needed at runtime:
  - macOS: `m1ddc`, `hidapitester`, DDPM.
  - Linux: `ddcutil`, `gsettings`, and access to `/dev/i2c-*` and `/dev/hidraw*`; the GNOME extension active for the edges and the clipboard.
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
  - Fakes: `NoHardware`, `RecordingHotkeys` and `RecordingEdges` (the sides the app last asked the edge sensor to watch).
  - Elements are found by `debug_selector` ids such as `"Name(0)"`, `"Hotkey(0)"`, `"Enabled(0)"`, `"SwitcherHotkey"`, `"MonitorMode"`, `"EdgesMode"`, `"EdgeChannel(Left)"`, `"EdgeName(Left)"`, `"EdgeAddress(Left)"` (the `Side` Debug name), `"ClipboardMode"`, `"ClipboardKey"` and `"CopyKey"`.
- **Clipboard network:** `adapters/net.rs` tests run a real listener on `127.0.0.1:0` with a `FakeClipboard`.
- **Filesystem tests:** temp dirs `std::env::temp_dir()/kvm-switch-test-{pid}-{name}`.
- **Untested:**
  - `app.rs`, `adapters.rs` wiring, `main.rs` launch flows (`AlreadyOpen`, `ConfirmQuit`), `linux/tray.rs`.
  - Real hardware I/O.
  - All shell scripts (there is no shellcheck setup).
- A full run needs the supported OS and hardware. Before finishing, run `cargo test` and the clippy gate on the OS you changed.
