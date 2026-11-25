# Developer guide

MyCCTV is one Rust crate that builds a Windows program, a macOS app and an Android library (packaged as an APK for
phones, tablets and Google TV). The UI is [egui](https://github.com/emilk/egui) / eframe 0.29 (OpenGL
through glow). There is no C++ or Java code of our own; native pieces come from crates.

## Requirements

| For | Needs |
|---|---|
| Windows build | Rust (stable, 1.85+), MSVC build tools |
| Android build | `rustup target add aarch64-linux-android armv7-linux-androideabi`, Android SDK (build-tools, platform 33), NDK 27, `cargo install cargo-apk`, a JDK (for `keytool`/signing) |
| macOS build (on a Mac) | Xcode command line tools, Rust; `bash scripts/build-macos.sh` (universal app + .dmg), or the GitHub **Release** workflow (it builds on GitHub's Macs) |
| Tests with sample video | ffmpeg (optional, to make test clips) |

## Build, run, test

    cargo build                     # debug build: target\debug\mycctv.exe
    cargo run -- --size 412x860     # run with a phone-sized window
    cargo test --lib                # unit tests (see "Tests" below)

    cargo build --release           # Windows release: target\release\mycctv.exe

    set ANDROID_HOME=%LOCALAPPDATA%\Android\Sdk
    set ANDROID_NDK_ROOT=%ANDROID_HOME%\ndk\27.0.12077973
    cargo apk build --lib --release # target\release\apk\MyCCTV.apk (arm64 + armv7, signed)

### Making a release
1. Raise `version` in `Cargo.toml` (the APK takes its version name and code from it).
2. Add a `## <version>` section to `CHANGELOG.md`; it becomes the release notes.
3. Commit, then tag and push: `git tag v<version>` and `git push origin main v<version>`.

The **Release** workflow (`.github/workflows/release.yml`) then builds on GitHub's machines:
`MyCCTV-<version>-windows-x64.exe` (Windows, after the unit tests), `MyCCTV-<version>-android-googletv.apk`
(Linux, NDK 27, cargo-apk) and `MyCCTV-<version>-macos-universal.dmg` (macOS 14; allowed to fail while the Mac
build is untested), and publishes a GitHub Release with those files, `SHA256SUMS.txt` and the changelog section.
The tag must match the `Cargo.toml` version. "Run workflow" on the Actions tab builds everything without
publishing (files are kept as run artifacts).

Local builds still work as before (commands above, files copied into `release/`, which is not in git).

### Continuous integration and website
- **CI** (`.github/workflows/ci.yml`): every push to `main` and every pull request is built and tested on Windows.
- **Pages** (`.github/workflows/pages.yml`): the website in `site/` is published to GitHub Pages when it
  changes. Its download buttons ask the GitHub API for the newest release in the visitor's browser, so a new
  release shows up without publishing the site again. The pictures in `site/img/` were taken with
  `--screenshot` and a demo device list (see "Command-line options"), not real cameras.

### Signing
`keys/release.keystore` is a local key so the APK installs (`keys/` is not in git). Before publishing on Google
Play, create your own key (`keytool -genkeypair …`), keep it safe, and point
`[package.metadata.android.signing.release]` in `Cargo.toml` at it (and don't keep its password in the file).

GitHub releases are signed with the key in the repository secrets `ANDROID_KEYSTORE_BASE64` (the keystore file,
base64-encoded) and `ANDROID_KEYSTORE_PASSWORD` (cargo-apk reads them as `CARGO_APK_RELEASE_KEYSTORE` and
`CARGO_APK_RELEASE_KEYSTORE_PASSWORD`). Without them the workflow signs with a throwaway key, and that APK won't
install over one signed with your key.

## Command-line options (`src/main.rs`)

| Option | Purpose |
|---|---|
| `--screenshot <file.png>` | Start without taking focus, draw ~120 frames, save the window as PNG, exit (used by automated checks). |
| `--page <home\|live\|playback\|devices\|events\|settings\|add\|devcfg<N>>` | Page to open first. |
| `--size <W>x<H>` | Window size (to check the phone layout). |
| `--probe`, `--probe-device`, `--playback-check`, `--config-check` | Headless check tools, see TROUBLESHOOTING.md. |
| `--tv-server` | Run only the TV server (no window), with the port and code from Settings. |

Test-only environment variables: `MYCCTV_SHOT_SECS` (wait before the screenshot), `MYCCTV_SHOT_FRAMES`,
`MYCCTV_CFG_SECTION` (open a device-settings section), `MYCCTV_TEST_ZOOM` (start windows zoomed),
`MYCCTV_DEBUG` (trace recorder-file playback). Point `APPDATA` at a copy of the settings to test without
touching the real device list.

## Source map (`src/`)

| File | What it does |
|---|---|
| `main.rs` | Windows entry point and command-line options. |
| `lib.rs` | Starts the desktop window or the Android activity (`android_main`); check tools. |
| `app.rs` | The application: state, background work, and every screen (dashboard, live, playback, devices, events, settings, lock). Logic first, drawing after the "drawing" marker. |
| `config_ui.rs` | Device settings screen (child module of `app`). |
| `kit.rs` | Reusable UI pieces: cards, chips, buttons, page header, form fields. |
| `theme.rs` | Colours and widget style ("Aurora" Midnight / Daylight). |
| `icons.rs` | Icons drawn with shapes (no image files); the app logo (also used for window and launcher icons). |
| `layout.rs` | The 12 window layouts (1–64 windows, with big windows for 6, 8, 13, 20). |
| `model.rs` | Devices, settings, saving/loading (`mycctv.json`), add-device rules, RTSP addresses and routes. |
| `stream.rs` | Video engine: one thread per window; RTSP (retina) → H.264 (openh264) / H.265 (rusty_h265) → RGBA; routes and fall-back; recording; snapshots; recorder-file playback. |
| `dav.rs` | Reader for the recorders' `.dav` recording files (DHAV blocks). |
| `http.rs` | Small HTTP/1.1 client: Digest/Basic login, HTTPS (rustls, self-signed accepted), redirects to HTTPS, streaming downloads, long-lived alarm feed. |
| `services.rs` | Device web interface: PTZ, presets, device info, recorder channel→camera links, recording search, alarms, online check, network scan. |
| `devconfig.rs` | Reading and writing device settings (configManager), capabilities, facts, reboot, clock sync. |
| `clock.rs` | Local date/time helpers. |
| `platform.rs` | Android specifics (on-screen keyboard). |
| `../build.rs` | Embeds `assets/app.ico` and version details into the Windows program (winresource; needs the Windows SDK resource compiler, otherwise builds without the icon and warns). |
| `relay.rs` | TV server for TVs that only run web apps (Hisense VIDAA…): serves the `vidaa/` web app, the device list, live video as MJPEG, PTZ, and the APK; access code; home network only. |
| `netpath.rs` | Network connections (adapters) and reaching a device through a chosen one: the list (if-addrs, the Windows adapter list, macOS `networksetup`, Android ConnectivityManager over JNI), the address → connection table (`sync`), bound TCP connect (socket2; macOS IP_BOUND_IF; Android `android_setsocknetwork`), a local relay for RTSP (`rtsp_through`), joining a Wi-Fi adapter to a router. |
| `net_ui.rs` | Settings > Networks screen and the connection picker (child module of `app`). |

The `vidaa/` folder holds the TV web app (plain ES5 JavaScript for older TV browsers); its files are built into
the program with `include_str!`/`include_bytes!`, so a change there needs a rebuild.

## How the main pieces work

### Video windows
`app.rs::reconcile()` runs every frame: for each visible window it works out the routes it should play
(`Device::live_routes` / `Device::playback_routes`) and starts or restarts a `stream::StreamHandle` when they
change. `pump_windows()` moves new pictures from the worker threads into GPU textures.
Auto stream choice: sharp (main) stream for small layouts or a big window, light (sub) stream otherwise.

Pausing: only windows someone can see run. `reconcile()` pauses (`PlayWindow::pause`: drops the handle, keeps
the camera and last picture, and for playback / instant replay moves the start to the time on screen) every
window that is on another page, hidden behind a maximized window, behind the lock screen, or while the app is
minimized; it starts again when shown. A window that is recording keeps going. The app on another monitor or
behind another app's window keeps playing (Windows doesn't tell apps they are covered). As a safety net,
`update()` calls `stream::ui_alive()` every frame and the video threads stop by themselves (state `Paused`)
after 3 s without a drawn frame, which covers Android going to the background and the phone screen turning off.
TV sharing streams (`start_routes_always`) and the check tools are never paused.

### Routes and fall-backs
A window gets a list of `stream::Route`s and tries them in order, remembering the one that worked:
- **Live**: the device's RTSP → for a recorder, the camera behind the channel (from the recorder's
  `RemoteChannel` / `RemoteDevice` settings, read at login).
- **Playback / instant replay**: RTSP playback → `dav://…` route, which searches the recordings
  (`mediaFileFind`), downloads each file (`/cgi-bin/RPC_Loadfile/<path>`) as a stream, reads its DHAV blocks
  and plays them at the chosen speed.

### Device web interface
Dahua-compatible CGI used by CP Plus and similar devices: `ptz.cgi`, `magicBox.cgi`, `configManager.cgi`,
`encode.cgi`, `mediaFileFind.cgi`, `eventManager.cgi`, `storageDevice.cgi`, `global.cgi`. `http.rs`
remembers which hosts redirected to HTTPS.

### Layouts
`app.rs` sets `compact` when the screen is narrower than 720 points: bottom tabs (`bottom_nav`), stacked
dashboard, Live View with video on top and the camera list below in portrait, PTZ as a bottom panel,
full-screen playback search, drop-downs in device settings. `kit::page_header` wraps on narrow screens.

### Saving
Everything is in `model::Store` → `mycctv.json` (written atomically). Live View's open cameras and the
folded panels are remembered in `Settings`.

### Two routers (chosen network connection)
`Device::network` names a connection ("Wi-Fi 2", "en1", "wlan0"; empty = automatic). `netpath::sync` maps the
device's address and its linked cameras to it (on load and every frame). `http.rs` and the online check use
`netpath::connect`, which binds the socket to that connection's address (Windows sends from the adapter that owns
the source address, so two routers that both use 192.168.1.x still work). retina opens its own sockets, so
`stream::run` sends RTSP through `netpath::rtsp_through`, a 127.0.0.1 relay that connects out through the chosen
connection. "Keep internet on main" (Windows) raises that adapter's interface metric to 9000 with Windows'
permission prompt. Wi-Fi joins add an adapter-only, manual-connect profile for the current user, so the main
adapter never joins the cameras' router by itself. Windows 11 hides Wi-Fi network names unless Location is allowed
for desktop apps; adapters come from `GetAdaptersAddresses`, so they are listed anyway. A Wi-Fi card taken over by
a Hyper-V virtual switch shows up as "vEthernet (…)".

## Tests
`cargo test --lib`:
- `layout`: every layout has the right number of windows.
- `model`: add-device checks in the original's order and messages.
- `stream`: decodes H.264 and H.265 sample clips (`shots/test.h264`, `shots/test.h265`; skipped if missing).
- `devconfig`: settings grouping and channel count.
- `netpath`: address segments, this computer's connections, and a relay to a pretend device through a chosen connection.
- `dav`: reads a real recorder file (`target/davtest/a.dav`; skipped if missing).

Regenerate the icons (Android launcher icons, `assets/app.ico`, `assets/app.icns`, `vidaa/icon.png`) after changing the logo in `icons.rs`:

    cargo test --lib write_launcher_icons -- --ignored

Make the sample clips with:

    ffmpeg -f lavfi -i testsrc2=size=640x360:rate=25 -t 2 -c:v libx264 -bf 0 -tune zerolatency -g 25 -pix_fmt yuv420p -bsf:v h264_mp4toannexb -f h264 shots/test.h264
    ffmpeg -f lavfi -i testsrc2=size=640x360:rate=25 -t 2 -c:v libx265 -x265-params bframes=0 -g 25 -pix_fmt yuv420p -f hevc shots/test.h265

## Known limits
- No camera sound or two-way talk yet; no vendor cloud (serial-number) sign-in.
- Fast forward above 1× only when playing from recorder files; above 2× only key frames are shown.
- Android Back key is not handled by the UI (egui-winit 0.29 does not map it); no Google TV banner image yet.
- Timings marked "estimated" in MECHANICS.md were not found in the original program.

## macOS
- Built only on a Mac (Apple's linker and system files aren't available on Windows): `bash scripts/build-macos.sh`
  builds `aarch64-apple-darwin` and `x86_64-apple-darwin`, joins them with `lipo`, makes `MyCCTV.app` from
  `macos/Info.plist` (version filled in from Cargo.toml) and `assets/app.icns`, signs it ad hoc and packs a `.dmg`.
- `macos/Info.plist` includes `NSLocalNetworkUsageDescription`: macOS 15+ asks the user before the app may
  reach devices on the home network; without permission no camera can be reached.
- Code differences: settings in `~/Library/Application Support/MyCCTV` (`model::data_dir`); during
  `--screenshot` checks the app doesn't take focus (`with_activate_ignoring_other_apps(false)` in `lib.rs`) and is
  not lowered behind other windows.
- To ship to other Macs without warnings: sign with a "Developer ID Application" certificate
  (`codesign --force --deep --options runtime --sign "Developer ID Application: …" MyCCTV.app`), then notarize
  (`xcrun notarytool submit MyCCTV-….dmg --keychain-profile … --wait`) and staple (`xcrun stapler staple`).
- Not built or tested on a real Mac yet.
