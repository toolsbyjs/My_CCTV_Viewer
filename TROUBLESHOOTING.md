# Troubleshooting

## A window says…

| Message | What it means | What to do |
|---|---|---|
| *Device is not reachable* | Nothing answers at that address and video port. | Check the address and the **Video port** (usually 554) under Devices → Edit; check the device is on the same network. |
| *User name or password is wrong* | The device refused the login. | Fix it under Devices → Edit. Many failed tries can lock the device for a while. |
| *The device refused the video connection…* | The device's RTSP video service is off or limits connections. | Turn on RTSP in the device's web page (Network → Connection → RTSP). For recorders, MyCCTV tries the camera behind the channel and, for playback, the recorder's files. |
| *No camera is connected to this channel* | The recorder reports no camera on that channel. | Pick another channel. |
| *No recordings from this time on* | Nothing is recorded after the chosen time. | Choose an earlier time on the timeline. |
| *This camera sends … video* | A codec other than H.264/H.265 (for example MJPEG). | Set the stream to H.264 or H.265 in Devices → Settings → Video quality. |

## Recorder disks full
If a recorder's disks are full it keeps overwriting the oldest recordings, so playback starts at the earliest
recording still kept. Devices → Settings → Overview shows the storage.

## Check tools (Windows, from a command prompt in the program's folder; use the release file name or rename it to mycctv.exe)
They use the saved device list and never show passwords. Device and camera numbers start at 1, in the
order shown on the Devices page.

    start /wait mycctv.exe --probe-device 1 1 1 picture.png
        Live video of device 1, camera 1, light stream (0 = sharp). Saves one picture.

    start /wait mycctv.exe --playback-check 1 1 2026-11-11 17:30:00 picture.png
        Plays device 1, camera 1 from that time; says whether RTSP or the recorder files were used.

    start /wait mycctv.exe --config-check 1 "ChannelTitle[0].Name"
        Reads all settings of device 1 and writes one back unchanged (checks that saving settings works).

    start /wait mycctv.exe --probe "rtsp://192.168.1.108:554/cam/realmonitor?channel=1&subtype=1" admin PASSWORD picture.png
        Tests any RTSP address directly.

    mycctv.exe --size 412x860
        Opens the app in a phone-sized window, to see the phone layout on a computer.

## Android / Google TV
- Text input opens the on-screen keyboard. On a TV, turn on Settings → *Arrow keys move between buttons*.
- The Back button of a TV remote isn't used inside the app yet (it may close the app); use the on-screen buttons instead.
- If text looks too big or small: Settings → *Text and button size*.
