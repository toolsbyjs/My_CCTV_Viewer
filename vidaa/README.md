# MyCCTV for Hisense VIDAA OS (and other TVs with a browser)

Hisense TVs with **VIDAA OS** can't install Android apps or programs from outside developers: they only run
web apps. So MyCCTV works there as a **TV web app** fed by MyCCTV running on a computer or Android box in
the same home network:

    cameras / recorder  →  MyCCTV (Windows PC or Android box, "TV sharing" on)  →  Hisense TV browser

## Set up
1. On the computer (or Android box): MyCCTV → **Settings → TV sharing** → turn on
   *Share cameras with TVs on this network*. It shows an address such as `http://192.168.1.36:8090` and a
   6-digit access code.
   (Without opening the window: `start mycctv.exe --tv-server`.)
2. On the Hisense TV: open the **Browser** app, type the address, and enter the access code once
   (or type the address with the code: `http://192.168.1.36:8090/?key=123456`). Bookmark it / pin it to the
   home screen.
3. Allow MyCCTV through the Windows firewall when Windows asks (private networks).

## Remote control
| Key | Does |
|---|---|
| Arrows | Move between windows |
| OK | Choose the camera for the selected window |
| Green (or F) | Full screen on / off |
| Yellow (or P) | Camera control (PTZ): arrows move the camera, 1 / 3 zoom, Back to finish |
| 1, 4, 9 | Window layout |
| Back | Close a list / leave full screen |

## Good to know
- The computer or box must stay on with MyCCTV running while you watch.
- Video goes to the TV as a stream of JPEG pictures (about 10 per second, up to 1280 pixels wide), which every
  TV browser shows without plug-ins. Each window uses about 2–6 Mbit/s of your home network.
- Only devices in your home network can connect, and every request needs the access code. Change the code in
  Settings if needed.
- Publishing as an app in the VIDAA app store needs a VIDAA partner account and a web address reachable from
  the internet; this home-network setup doesn't need either.

## Files
`index.html`, `style.css`, `app.js` (plain JavaScript that older TV browsers can run) and `icon.png`. They are built
into MyCCTV and served by its TV server, so you don't copy them anywhere.
