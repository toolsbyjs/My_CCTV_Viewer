# MyCCTV

A viewer for camera recorders (NVR/DVR) and IP cameras. One app for **Windows**, **macOS (Mac mini and other Macs)**, **Android phones and
tablets**, and **Android TV / Google TV**, written in Rust.

- Live view in 1 to 64 windows, with camera turning/zooming (PTZ) and digital zoom
- Playback with a 24-hour timeline, slow motion and fast forward
- Pictures and video clips saved to your computer or phone
- Motion and alarm messages from your devices
- The devices' own settings (video quality, picture, detection, recording, time, network and every other setting they offer)
- Works with a mouse, touch screen or TV remote; adapts to phone, tablet, desktop and TV screens

## Download and install (release 1.10.0)

The ready-made files are in the `release/` folder:

| File | For |
|---|---|
| `MyCCTV-1.10.0-windows-x64.exe` | Windows 10 / 11 (64-bit). One program, no installer: copy it anywhere and run it. |
| `MyCCTV-1.10.0-android-googletv.apk` | Android 7 or newer: phones, tablets, Android TV, Google TV (64-bit and 32-bit devices). |
| `MyCCTV-1.10.0-macos-universal.dmg` | macOS 11 or newer: Mac mini, MacBook, iMac (Apple silicon and Intel). Built on a Mac, see *Mac* below. |

**Android / Google TV:** copy the `.apk` to the device and open it (allow "install unknown apps" for the
file manager), or from a computer: `adb install -r MyCCTV-1.10.0-android-googletv.apk`.
On Google TV, MyCCTV appears in the apps list.

## Which TVs

| TV / stick | How | Notes |
|---|---|---|
| **Google TV** (Chromecast with Google TV, Google TV Streamer, Sony / TCL / Hisense Google TV models) | Install the `.apk` | Shows in the apps list. 64-bit and 32-bit models are covered. |
| **Android TV** boxes and TVs (Android 7 or newer) | Install the `.apk` | Same package. |
| **Amazon Fire TV Stick / Fire TV** with Fire OS 6, 7 or 8 (most models from 2018 on: Fire TV Stick 4K, 4K Max, Lite, 3rd gen, Fire TV Cube) | Install the `.apk` (sideload) | Fire OS is based on Android, so the same package works. Not tested on a real Fire TV yet. Older sticks on Fire OS 5 (Android 5) are too old. Newer Fire TV models running **Vega OS** can't install Android apps. |
| **Hisense with VIDAA OS** (and other TVs that only have a browser, such as LG webOS or Samsung Tizen) | TV web app through **TV sharing** | MyCCTV on a computer or Android box shares the cameras with the TV's browser. See [vidaa/README.md](vidaa/README.md). |

**Install on Google TV:** Settings → System → About → click *Android TV OS build* 7 times to become a
developer; Settings → Apps → Security & restrictions → allow your file manager to install unknown apps; copy the
`.apk` with a USB stick or a file-sharing app and open it. Or from a computer: `adb connect <TV-IP>` then
`adb install -r MyCCTV-1.10.0-android-googletv.apk`.

**Install on Fire TV Stick:** Settings → My Fire TV → About → click the device name 7 times; Settings → My Fire TV →
Developer options → *Install unknown apps* → allow **Downloader**; in **Downloader**, type
`http://<PC address>:8090/apk` (MyCCTV on the PC with TV sharing on, and the `.apk` next to the `.exe`, as in the
`release/` folder) and install. Or use `adb connect <FireTV-IP>` and `adb install -r MyCCTV-1.10.0-android-googletv.apk`.

On TVs, turn on Settings → *Arrow keys move between buttons* in MyCCTV (it is on by default on Android).

## Mac (Mac mini, MacBook, iMac)

The Mac app has to be built on a Mac (Apple's tools only run there). Either:

- **On a Mac:** install the Xcode command line tools (`xcode-select --install`) and Rust (https://rustup.rs),
  then in the app folder run `bash scripts/build-macos.sh`. It makes `release/MyCCTV.app` and
  `release/MyCCTV-<version>-macos-universal.dmg`, one app for both Apple-silicon (M1–M4) and Intel Mac minis.
- **Without a Mac:** run the **Release** action on GitHub (`.github/workflows/release.yml`, "Run workflow");
  it builds the `.dmg` on GitHub's Macs. Download it from the run's artifacts.

Open the `.dmg` and drag MyCCTV to Applications. The first time, right-click MyCCTV → **Open** (the app is not
signed with an Apple developer certificate yet). When macOS asks to let MyCCTV find devices on your local
network, choose **Allow**, otherwise it can't reach your cameras. Settings are kept in
`~/Library/Application Support/MyCCTV`, pictures and clips in `~/Pictures/MyCCTV`.

## Cameras on another router (two routers at home)

If your recorder and cameras are on a different router than the one your computer or phone uses for the
internet, MyCCTV can reach them through a **second connection** while everything else stays on your main one.

1. Give the device a second connection to the cameras' router:
   - **Computer:** a network cable from the cameras' router into a free cable port, or a small USB Wi-Fi
     adapter. One Wi-Fi adapter can only be on one router at a time. With a second Wi-Fi adapter, MyCCTV can
     join the cameras' router for you: **Settings > Networks > Join another router** (Windows; on a Mac
     type the Wi-Fi name and password).
   - **Phone / tablet:** join the cameras' router in Wi-Fi and keep mobile data on for the internet (if
     Android asks, choose to stay connected without internet).
2. On Windows, press **Keep internet on main** next to the second connection (Windows asks for permission
   once) so the computer keeps using the main router for the internet.
3. In **Devices > Edit**, set **Network connection** to the second connection. Live view, playback, settings and
   the cameras behind the recorder all go through it, even when both routers use the same addresses
   (192.168.1.x). **Scan network** can also search on the second connection.

## First steps

1. Open **Devices** → **Add device** (or **Scan network** to find recorders and cameras).
2. Type the device's address (for example `192.168.1.108`), user name and password, and press **Add device**.
   MyCCTV reads the model, serial number and number of cameras by itself.
3. Open **Live**, then double-click (or tap) a camera. **Watch** on a device card opens all its cameras.

Your device must offer standard video streaming (**RTSP**, usually on by default). Video in **H.264** and
**H.265** is supported.

## Using the app

### Live
- **Window layout**: the layout button in the floating bar (1, 4, 6, 8, 9, 13, 16, 20, 25, 36, 49, 64 windows).
- **Double-click** a window to make it big; again (or Esc) to go back. **F11** = full screen.
- **Hover a window** for: instant replay, take a picture, record, close.
- **Right-click / long-press** a window for: picture shape, video quality (sharp/light stream), take 3 pictures, open in Playback, zoom.
- **Digital zoom**: mouse wheel or pinch over the video zooms at that spot (up to 8×); drag to move around.
  The + / − / 1× buttons on the right of a window, the menu, or the + / − / 0 keys do the same.
- **Tour** (loop button) shows every camera in turn.
- **Camera control (PTZ)**: the joystick button opens the round pad, zoom/focus/iris, speed and presets.
  Hold a button to move; a single press (TV remote) moves briefly.
- **Recorders**: if a recorder refuses video, MyCCTV shows the channel straight from the camera behind it
  (the window title says "via camera …").

### Playback
Pick a day, a recording type and up to 36 cameras, press **Search recordings**, then click the timeline to play
from that moment. **Zoom the timeline** with − / +, the Day / 6 h / 1 h / 10 min / 2 min buttons, the mouse wheel or
pinch, and drag it (or use ◀ ▶) to move along the day, so you can pick the exact second.
Speeds 1/8× to 8×, save a clip, take a picture. If a recorder refuses standard playback,
MyCCTV plays its recording files directly over its web page.

### Devices
Each card shows status, model and serial. **Watch** opens all cameras, **Edit** changes how MyCCTV connects,
**Settings** opens the device's own settings, **Refresh details** reads them again.
**Import / Export** moves the device list (with passwords) between computers, phones and TVs.

### Device settings
Overview (model, firmware, storage, restart), video quality, picture, on-screen text, detection & alarms,
recording, date & time (and "set to this computer's time"), network, ports & services, and **All settings**
(everything the device offers, searchable). For a recorder you can switch to the camera behind each channel.
Network and port changes are confirmed first.

### Events
Motion, alarm inputs, video loss and smart detections from your devices, with filters and a **Watch** button.

### Settings
Theme (Midnight / Daylight), text size, automatic sharp/light stream, tour interval, instant replay length,
alarms on/off, TV remote mode, where pictures and recordings are saved, and an optional app password.

## Screens and controls

- **Phones** (narrow screens): tabs at the bottom, video on top with the camera list below, camera control
  slides up from the bottom.
- **Bigger screens**: the left menu, the camera list and the playback search panel can each be folded away for
  more video; a small button brings them back.
- **TV remote**: turn on Settings → *Arrow keys move between buttons*. Arrows move, OK presses, + / − zoom.

## Where things are kept

| What | Windows | Android |
|---|---|---|
| Device list and settings | `%APPDATA%\MyCCTV\mycctv.json` | app's private storage |
| Pictures (`Snapshots`) and clips (`Records`) | `Pictures\MyCCTV` | app's private storage |

The folder for pictures and clips can be changed in Settings. Clips are raw `.h264` / `.h265` video
(they open in VLC).

## More documentation

- [TROUBLESHOOTING.md](TROUBLESHOOTING.md): no video, wrong password, recorder problems, check tools
- [DEV_GUIDE.md](DEV_GUIDE.md): how the code is organised, how to build, test and release
- [CHANGELOG.md](CHANGELOG.md): what changed in each version
- [MECHANICS.md](MECHANICS.md): rules and numbers taken from the original vendor program
