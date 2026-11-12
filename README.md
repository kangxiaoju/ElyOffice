# Ely Office

Offline-first desktop apps in Rust and GPUI. Files stay in your folders; no account, no cloud service.

Download from [office.elygpui.com](https://office.elygpui.com) or the [latest release](https://github.com/ELY-Open-Source/ElyOffice/releases/latest): Windows, macOS (Apple silicon and Intel) and Linux (AppImage, deb, rpm, Arch, Nix; Debian 11 and Ubuntu 20.04 or newer).

| App | Does | Engine |
| --- | --- | --- |
| ElyDoc | Word processing (DOCX, ODT) | LibreOfficeKit |
| ElyTable | Spreadsheets (XLSX, ODS, CSV) | LibreOfficeKit |
| ElyShow | Presentations (PPTX, ODP) | LibreOfficeKit, system media |
| ElyPDF | PDF reading and editing, OCR, signatures | MuPDF, Tesseract |
| ElyMail | Mail over IMAP, POP3 and SMTP; follows the system proxy | own IMAP and POP3, mail-send, SQLite |
| ElyTree | Git client | Git |
| ElyMark | Markdown | markdown-rs, RaTeX, mermaid-rs-renderer |
| ElyAgent | Second release | Kigi CLI |

ElyMail signs in to Microsoft with OAuth, and to Gmail and iCloud with an app password.

Yu's `fix/yu-workspace-components` branch keeps its current Office client/worker protocol and aligns the component pin with the upstream-synchronized Yu components. Full Office upstream integration is published separately on `sync/ai-workspace-polish`; it requires matching LOK/PDF worker releases before Yu can adopt it.

`perf/yu-component-pacing` advances only that component pin to include optional `Spinner::max_fps`; Office implementations and worker protocols remain unchanged.

### GPUI Fast migration

The direct `gpui-fast` / `gpui-fast-platform` backend uses the shared Fast 0.1.14
runtime at `kangxiaoju/Ely-GPUI-Components@40427c396cc92b6946940e0e498612a8b680ce30`
on its independent `runtime/gpui-fast-0.1.14` branch. Core defaults are disabled;
`stacker`, `font-kit`, `wayland` and `x11` remain explicit. Windows text language
uses the same source's `gpui-fast-windows`. Ely components use revision
`8b9ab8b1a827bcb6102eaecabffd0b1f623d5401`, including multi-node graph search
highlights and the existing activation/read-only APIs; IPC, document formats and
worker dependencies are unchanged. GPUI Kit is not required.

The obsolete `vendor/gpui-pre*` copies and their four patches are removed after
porting the missing behavior below into that shared runtime. The accesskit, zbus
and mail-parser patches remain independent. Root patches in this repository are
not inherited by an application depending on Office.

Comparison against the published `0.1.14` sources found:

| Existing fix | Fast status | Minimal shared Fast change still needed |
| --- | --- | --- |
| Real platform quit observation (`TestAppContext::quit_requests`) | Missing API and test-platform instrumentation | Restore the context getter and `TestPlatform`'s `Cell<usize>`, initialized to zero and incremented only in `Platform::quit`; retain the tray assertions. |
| App-selected Han text language (`set_text_language`) | Missing API and behavior | Restore the Windows export, locked app-language override, Han fallback from user language preferences, trailing-NUL locale trimming, and original language/layout tests. Fast's `DirectWriteTextSystem::new()` takes no GPU argument; adapt the native test construction accordingly. Enable `Foundation_Collections` and `System_UserProfile` on its Windows dependency. |
| X11 XIM open-handshake guard | Included | `gpui-fast-linux/src/fast/xim.rs::is_handshake_complete` checks `connected` before `enable_ime` takes the connection. |
| Accessibility debug timestamp capture without per-frame local-time conversion | Included | Fast captures `SystemTime` in `src/fast/a11y.rs::CapturedAt` and formats it only on dump. |
| Invalid animated-image frame terminates decoding | Missing | In core `src/platform.rs::decode_animated_frames`, return `Err(err.into())` instead of skipping errors. Fast shares this decoder between GIF/WebP assets and image-data conversion. |
| AT-SPI window screen bounds on activation and move/resize | Missing | In core `src/window.rs`, call `platform_window.a11y_update_window_bounds()` before activation refresh and in `bounds_changed`. |
| Topmost Windows titlebar control hit-test | Missing | In core `src/window.rs`'s control hit-test callback, walk `mouse_hit_test.ids.iter().take(hover_hitbox_count)` in order, finding each ID in `window_control_hitboxes`, rather than walking registered controls first. |
| macOS accessibility first-responder view | Missing | In `gpui-fast-macos/src/window.rs`, use `SubclassingAdapter::new(native_view.as_ptr() as *mut c_void, ...)` instead of `for_window(native_window, ...)`. Keep adapter teardown to break the retain cycle. |
| Windows accessibility initial active state | Missing | In `gpui-fast-windows/src/window.rs::a11y_init`, use `self.is_active()` rather than `GetForegroundWindow()`. |
| Windows native window destruction and tasks after quit | Missing | Port the `destroyed` state, drag/drop revocation and active-callback removal in `events.rs`, the no-second-destroy guard in `window.rs`, and the invalid-window-handle shutdown handling in `dispatcher.rs`. |
| Windows Save As extension | Missing | In Windows `platform.rs::file_save_dialog`, derive the extension/filter from `suggested_name` before consuming it and call `SetDefaultExtension`, keeping the all-files option. |
| Windows pinned-window grouping | Missing Office-specific behavior | Preserve Office's absence of a process AUMID call in Windows `platform.rs::set_app_identity`, while still storing the app identity. |

No existing Fast public API replaces either missing quit observation or
app-selected text language. Their Office callers and tests intentionally remain
intact; the published release alone is not sufficient to complete this migration.

On Windows, Fast's platform dependency enables its core `windows-manifest`
feature even with defaults disabled. Keep it: Office's build script embeds only
an icon, not a host manifest. `windows-manifest-provided-by-host` skips Fast's
manifest entirely and must not be enabled here; Fast supplies PerMonitorV2 DPI
awareness and the common-controls declaration. Build LOK/PDF workers separately
from UI applications (as below), so UI features do not leak into workers.

The shared runtime's Linux platform builds and core tests have been exercised by
the integrating Yu migration. Office application/worker integration and native
macOS/Windows behavior require their own checks; this source comparison is not
evidence of platform parity.

## Build

Install Rust with [rustup](https://rustup.rs) (`rust-toolchain.toml` pins the version), then the system packages. Every system also needs Git, Python 3, curl, Node.js 22.18 or newer (the first to run the site's TypeScript tests unflagged) and pnpm (`check.sh` and the site use the last two).

- macOS: Xcode with its Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`), since gpui compiles its shaders with `xcrun metal`; then `brew install cmake ninja pkg-config make gnu-sed bison flex gettext autoconf automake node pnpm` for LibreOffice and the checks (`build-libreoffice.sh` puts Homebrew's GNU make, sed, bison, flex and gettext first).
- Windows: Visual Studio 2022 with the C++ workload (MSVC, a Windows SDK and CMake with Ninja); gpui compiles its shaders with the SDK's `fxc.exe`. Git for Windows supplies bash and curl. LibreOffice builds under Cygwin in `C:\cygwin64` (`ELY_CYGWIN` elsewhere) with autoconf, automake and LibreOffice's GNU make in `/opt/lo/bin`, as its [Windows build guide](https://wiki.documentfoundation.org/Development/BuildingOnWindows) sets up.
- Linux, one line per family, then pnpm. LibreOffice builds inside `packaging/linux/container.sh`, hence Docker. Ubuntu's own Node.js is too old, so Node.js 22 comes from NodeSource:

```sh
sudo apt install build-essential pkg-config git curl python3 docker.io libdbus-1-dev libfontconfig-dev libxcb1-dev libxkbcommon-x11-dev
curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash - && sudo apt install nodejs
sudo dnf install gcc make pkgconf-pkg-config git curl python3 nodejs npm moby-engine dbus-devel fontconfig-devel libxcb-devel libxkbcommon-x11-devel
sudo pacman -S --needed base-devel git curl python nodejs npm docker dbus fontconfig libxcb libxkbcommon-x11
sudo npm install -g pnpm
```

`scripts/doctor.sh` checks each of these for the system it runs on.

```sh
scripts/doctor.sh                 # toolchain versions
scripts/fetch-references.sh       # upstream sources at pinned commits
scripts/build-libreoffice.sh      # patched LibreOffice 26.2.6.3 (about an hour)
scripts/build-mupdf.sh            # patched MuPDF 1.28.5
scripts/build-git.sh && scripts/fetch-git-lfs.sh
scripts/fetch-tessdata.sh
cargo build -p ely-lok --bins -p ely-pdf-worker
cargo run -p ely-doc              # or ely-table, ely-show, ely-pdf, ely-mail, ely-tree, ely-mark
```

ElyMail needs a Microsoft client ID at build time: put `ELY_MICROSOFT_CLIENT_ID=<id>` in `cred.txt` at the repository root (git ignores it).

The scripts run on macOS, on Windows (LibreOffice under Cygwin, MuPDF with MSBuild) and on Linux inside `packaging/linux/container.sh`, an AlmaLinux 8 image that keeps the binaries on glibc 2.28.

## Experimental R macros

ElyTable can run local R macros on a selected range, including values, header
formatting and column widths. See [docs/r-macros.md](docs/r-macros.md) for
setup, the small API and its limits. Requires R (Rscript) and a LOK engine
built with patch 0081.

## Release

Pushing a `v*` tag runs `.github/workflows/release.yml`. Self-hosted runners build macOS (arm64, and Intel under Rosetta), Windows and Linux with `scripts/ci.sh` and upload each app's packages, update manifests and the source archives to the GitHub release. macOS builds are signed and notarized.

## Tests

`cargo test --workspace`; the script tests in `tests/scripts` (the window tests need macOS Accessibility permission; Linux tests run in containers); the benchmarks `tests/perf/bench.py` and `tests/perf/folder-search.rs`.

## Documents

| | |
| --- | --- |
| `PRD.md` | Requirements |
| `agents.md` | Stack, rules and commands; the single source of truth |
| `tasks/TASKS.md` | Progress |
| `docs/decisions.md` | Decision log, by date |
| `docs/help.md` | Help: opening and saving, panels, recovery, offline limits, repositories, mail |
| `docs/compatibility.md` | Compatibility list for the current release |
| `docs/accessibility.md` | Accessibility and language review |
| `docs/perf/` | Benchmark records |
| `docs/licenses.md` | Licences and sources |
| `CONTRIBUTING.md` | Opening a pull request |

## Licence

MIT (`LICENSE`). Third-party components keep their own licences; see `docs/licenses.md`.
