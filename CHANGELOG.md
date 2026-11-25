# Changelog

## Unreleased
- Fixed: watching live video for a few minutes made the whole computer slow. Every picture (about 15 MB for a
  2560x1440 camera, 25 times a second per window) was put in brand-new memory twice, so Windows had to clear
  hundreds of thousands of memory pages a second. Pictures now reuse their memory and go to the screen
  without being copied, and the picture on the graphics card is updated in place
  (page faults with two 1440p cameras: about 362,000/s before, about 250/s after).

## 1.10.0
- Smoother computer: live view and playback only use the network and processor while you can see them. They
  pause on other pages, behind an enlarged window, behind the lock screen, and while the app is minimized or in
  the background on Android, then carry on when shown again (playback from the same moment). The app visible on
  another monitor keeps playing; a window that is recording keeps recording; TV sharing is not affected.

## 1.9.0
- Two routers at home: each device can be reached through a chosen network connection (a second Wi-Fi adapter,
  a network cable, or Wi-Fi next to mobile data on phones) while the computer keeps its main connection for
  everything else; works even when both routers use the same addresses. Live view, playback, settings, the
  online check and the cameras behind a recorder all follow it.
- Settings > Networks: the connections now (main one marked), free cable ports, joining a second Wi-Fi adapter
  to a router (Windows: pick from the list; Mac: type the name), "Keep internet on main" (Windows) and a
  shortcut to the system Wi-Fi settings.
- Scan network can search on a chosen connection; devices found there are added with it.
- The same address behind two different routers can be added as two devices.

## 1.8.0
- macOS support (Mac mini, MacBook, iMac; Apple silicon and Intel): Mac app bundle with icon, local-network
  permission text, settings in Application Support, a build script for a universal app and .dmg
  (`scripts/build-macos.sh`) and a GitHub action to build it without a Mac. Not yet built on a real Mac.

## 1.7.0
- Renamed to **MyCCTV** (window, screens, Windows program `mycctv.exe`, Android / TV package `com.mycctv.viewer`,
  TV web app). The device list and settings of "My KVMS" are moved over automatically on first start.
  On Android / TV the new package installs next to the old one (they are separate apps).

(Entries below were released under the name "My KVMS".)

## 1.6.0
- Playback from the recorder starts within about a second at the exact moment chosen: the recorder is asked
  for that time directly instead of downloading whole recording files from their start.
- Fixed: playback could show the wrong camera or an older day, because recording files begin with left-over
  data from overwritten recordings; only footage from the chosen day is played now.
- Zoomable playback timeline: − / + buttons, Day / 6 h / 1 h / 10 min / 2 min views, ◀ ▶ to move,
  mouse wheel or pinch to zoom at a point, drag to move; the time marks adapt (down to 1-minute steps) and the
  view follows the playing picture.
- Fixed: a recording search could fail when several started at the same moment on a recorder with a secure web page.

## 1.5.1
- Proper Windows program icon (16–256 px, from the app logo) and file details (name, description, version),
  embedded in both debug and release builds.

## 1.5.0
- TV sharing for Hisense VIDAA OS and other TVs with only a browser: a built-in TV server (Settings → TV sharing,
  or `--tv-server` without a window) and a remote-friendly TV web app (`vidaa/`): 1/4/9 layouts, camera list,
  full screen, camera control, access code, home network only.
- The TV server also hands out the Android package at `/apk` (for Fire TV Downloader).
- README: which TVs are supported (Google TV, Android TV, Fire TV, VIDAA) and how to install.

## 1.4.0
- Digital zoom in every live and playback window: mouse wheel / pinch at a point (up to 8×), drag to move,
  + / − / 1× buttons, menu items and + / − / 0 keys; zoom level and a small map of the zoomed area.
- Clean rebuild of all release files.
- Check tools print to the command prompt in the release program too.
- Documentation: README, TROUBLESHOOTING, DEV_GUIDE, CHANGELOG.

## 1.3.0
- Playback from recorders that refuse RTSP: plays the recorder's own recording files (`.dav`) over its
  (secure) web page. Enables 2×, 4× and 8× fast forward; the timeline cursor follows the picture.
- Instant replay uses the same fallback.

## 1.2.0
- Phone layout for narrow screens: bottom tabs, video on top with camera list below, PTZ sliding up,
  full-screen playback search, full-width forms and settings.
- Left menu, camera list and playback search panel can be folded away (remembered).
- Default text size 1.0 on Android (density scaling already applies).

## 1.1.1
- Devices page: cards in a grid that fits the window; clear Watch / Edit / Settings / Refresh / Delete buttons.

## 1.1.0
- Device settings screen: friendly sections plus "All settings" (every setting the device offers), for
  recorders and the cameras behind their channels; restart and clock sync.

## 1.0.2
- Recorders that refuse RTSP: live channels play straight from the camera behind them.
- HTTPS support for device web pages (self-signed certificates accepted).
- Device details refreshed at start-up.

## 1.0.1
- H.265 video decoding (pure Rust), so CP Plus cameras sending H.265 now show video.
- Clear message when a device refuses the video connection.
- Live View reopens the cameras that were open last time.

## 1.0.0
- First release for Windows and Android / Google TV (64- and 32-bit), with app icon.
- New "Aurora" design: navigation rail, dashboard, device cards, floating video controls, round PTZ pad.
- Devices, live view (1–64 windows), PTZ and presets, playback with timeline, pictures and clips,
  events, settings, app lock, TV remote navigation.
