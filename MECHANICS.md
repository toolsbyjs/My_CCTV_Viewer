# MyCCTV — rules sheet

Values read from the original program (KVMS Pro Lite V1.003.00AT005.0) by decompiling its
feature libraries, plus its own settings files. Our code was written from this sheet, not
copied. "estimated" means we looked in the code and did not find the value.

## Add / edit device (DeviceManagerUI.dll)

| Rule | Value | Source |
|---|---|---|
| Check order | name → address → port → user name | function 0x18005fdd0 |
| Empty name | "Device name is empty!" | 0x18005fdd0 |
| Empty address | "IP/Domain name can not be empty!" (serial mode: "SN can not be empty!") | 0x18005fdd0 |
| Empty port | "Device's port can not be empty!" | 0x18005fdd0 |
| Highest port | 65535 ("Port can not be more than 65535") | 0x18005fdd0 (`ebx <= 0xffff`) |
| Empty user | "User name can not be empty!" | 0x18005fdd0 |
| Address length / characters | 50, `[a-zA-Z0-9._:-]` | 0x180060b70 (setMaxLength 0x32, regex) |
| User name length | 50 | 0x180060b70 (setMaxLength 0x32) |
| Password length | 32, hidden while typing | 0x180060b70 (setMaxLength 0x20, echo mode 2) |
| Port digits | 5 | 0x180060b70 (validator `^[0-9]{5}`) |
| "Local" | becomes 127.0.0.1 | 0x18005f230 |
| Same name twice | refused: "Device Title has in list!" | 0x18005f230 |
| Same address + port twice | refused: "IP/Domain has in list!" | 0x18005f230 |
| Buttons | Add, Add and Continue, Cancel | 0x1800625b0 |
| Dialog width | 540 | 0x180060b70 (SetFixWidth 0x21c) |
| Register modes | IP/Domain, InstaOn serial number (cloud) | 0x180060b70 |
| Default user | admin | Default/conf.xml, string at 0x180097568 |
| Search limit | 16 network segments | 0x18001b840 |

Our version: the original talks to devices with the vendor's closed SDK (port 37777). We use
the standard video protocol (RTSP, port 554) and the device's web interface (port 80), so the
form asks for a video port and a web port. Cloud (serial number) sign-in is not available.

## Live view (RealMonitorUI.dll, Default/WndSplit.xml)

| Rule | Value | Source |
|---|---|---|
| Window layouts | 1, 4, 6, 8, 9, 13, 16, 20, 25, 36, 49, 64 | WndSplit.xml NormalSplit |
| 6 windows | 3×3 grid, one 2×2 big window top-left | WndSplit.xml |
| 8 windows | 4×4 grid, one 3×3 big window top-left | WndSplit.xml |
| 13 windows | 4×4 grid, one 2×2 big window in the centre | WndSplit.xml |
| 20 windows | 6×6 grid, two 3×3 big windows on top | WndSplit.xml |
| PTZ buttons | 8 directions, Zoom ±, Focus ±, Aperture ± | 0x180134500 |
| PTZ speed ("Step") | 1 to 8, steps of 1 | 0x180134500 (setMinimum 1, setMaximum 8, pageStep 1) |
| Preset numbers | 1 to 255 | string "Preset No. must be between 1 and 255!" |
| Window menu | Close Video, Close All Video, Start/Stop Record, Capture Picture, Continue Capture, Start/Stop Replay, Playback, Scale Type, Stream Type, Full Screen | 0x180164810 |
| Scale types | Original, Full, 16:9, 9:16 (also 64:9, 9:64, not kept) | 0x180164810 |
| Stream types | Main Stream, Assist Stream1, Assist Stream2 | 0x180164810 |
| Pick stream by window count | on | Default/Settings.ini AutoStreamType=1 |
| Main stream up to N windows | 4 | estimated |
| Tour | start / stop | 0x18011bc60 |
| Tour interval | 10 s | estimated |
| Instant replay length | 30 s | estimated |
| Continuous capture | 3 pictures, 1 s apart | estimated |
| A single PTZ press (remote control) | moves for 0.6 s | estimated |

## Playback (PlayBackUI.dll)

| Rule | Value | Source |
|---|---|---|
| Speeds | 1/8X, 1/4X, 1/2X, 1X, 2X, 4X, 8X | 0x180126e00 |
| Most cameras at once | 36 | 0x180134ac0 (`< 0x25`, message with 0x24) |
| Largest playback layout | 36 windows | 0x1800c38e0 |
| Recording types | All Records, General Record, Alarm Record, Motion Detect Record | 0x18014a840 |
| Other tools | frame by frame, time clip, download | strings 0x1801de5c8, 0x1801de588, 0x1801d6ac0 |

Our version: speeds below 1X are paced on our side. 2X/4X/8X need a fast-forward request that
the standard video protocol library we use does not send, so they are shown but disabled.

## Look

The person asked for a new, different design. Our "Aurora" design (navigation rail, cards,
floating video dock, round PTZ pad, Midnight/Daylight themes) is our own; the original's
"Dusty Blue" colours were only used as a starting reference.
