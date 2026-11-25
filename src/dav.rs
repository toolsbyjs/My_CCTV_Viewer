//! Reader for the recorder's own recording files (".dav"): a short index followed by
//! blocks that each start with "DHAV" and end with "dhav". Video blocks carry plain
//! H.264 / H.265 (Annex B), which our decoders already understand.
//!
//! Block layout (24-byte header, little-endian):
//! 0 "DHAV" | 4 type (0xFD key frame, 0xFC frame, 0xF0 audio, 0xF1 extra) | 5..8 sub/channel
//! 8 frame number u32 | 12 block length u32 (header + extras + data + 8-byte trailer)
//! 16 date u32 (sec 6 bits, min 6, hour 5, day 5, month 4, year-2000 6) | 20 milliseconds u16
//! 22 extra-header length u8 | 23 checksum | extras (tag 0x81: codec in byte 2) | data | "dhav" + length

use std::io::Read;

pub const KEY_FRAME: u8 = 0xFD;
pub const FRAME: u8 = 0xFC;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
}

pub struct DavFrame {
    pub kind: u8,
    /// Seconds since midnight, from the block's date.
    pub secs_of_day: u32,
    /// Free-running millisecond counter (wraps at 65536).
    pub ms: u16,
    /// Day of the month and month from the block's date (to skip left-over data from old recordings).
    pub day: u8,
    pub month: u8,
    pub codec: Option<Codec>,
    pub data: Vec<u8>,
}

impl DavFrame {
    pub fn is_key(&self) -> bool {
        self.kind == KEY_FRAME
    }
}

pub struct DavReader<R: Read> {
    r: R,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
}

const MAX_BLOCK: usize = 16 * 1024 * 1024;

impl<R: Read> DavReader<R> {
    pub fn new(r: R) -> Self {
        Self { r, buf: Vec::with_capacity(1 << 20), pos: 0, eof: false }
    }

    /// Makes sure `n` bytes from `pos` are in the buffer; false at the end of the file.
    fn fill(&mut self, n: usize) -> std::io::Result<bool> {
        while self.buf.len() - self.pos < n {
            if self.eof {
                return Ok(false);
            }
            if self.pos > (4 << 20) {
                self.buf.drain(..self.pos);
                self.pos = 0;
            }
            let mut chunk = [0u8; 64 * 1024];
            let got = self.r.read(&mut chunk)?;
            if got == 0 {
                self.eof = true;
            } else {
                self.buf.extend_from_slice(&chunk[..got]);
            }
        }
        Ok(true)
    }

    /// The next video frame, skipping the index, audio and anything damaged.
    pub fn next_frame(&mut self) -> std::io::Result<Option<DavFrame>> {
        loop {
            if !self.fill(24)? {
                return Ok(None);
            }
            if &self.buf[self.pos..self.pos + 4] != b"DHAV" {
                // Resynchronise on the next "DHAV".
                match self.buf[self.pos + 1..].windows(4).position(|w| w == b"DHAV") {
                    Some(off) => self.pos += 1 + off,
                    None => {
                        self.pos = self.buf.len().saturating_sub(3);
                        if !self.fill(24)? {
                            return Ok(None);
                        }
                    }
                }
                continue;
            }
            let h = &self.buf[self.pos..self.pos + 24];
            let kind = h[4];
            let len = u32::from_le_bytes([h[12], h[13], h[14], h[15]]) as usize;
            let ext = h[22] as usize;
            if !matches!(kind, KEY_FRAME | FRAME | 0xF0 | 0xF1) || len < 32 + ext || len > MAX_BLOCK {
                self.pos += 4;
                continue;
            }
            if !self.fill(len)? {
                return Ok(None);
            }
            let block = &self.buf[self.pos..self.pos + len];
            if &block[len - 8..len - 4] != b"dhav" {
                self.pos += 4;
                continue;
            }
            let date = u32::from_le_bytes([block[16], block[17], block[18], block[19]]);
            let ms = u16::from_le_bytes([block[20], block[21]]);
            let secs_of_day = ((date >> 12) & 0x1f) * 3600 + ((date >> 6) & 0x3f) * 60 + (date & 0x3f);
            let mut codec = None;
            let extras = &block[24..24 + ext];
            let mut i = 0;
            while i + 4 <= extras.len() {
                if extras[i] == 0x81 {
                    codec = match extras[i + 2] {
                        0x0C => Some(Codec::H265),
                        0x02 | 0x08 => Some(Codec::H264),
                        _ => None,
                    };
                }
                // Extras come in 4- or 8-byte records; 0x82 / 0x88 / 0x83 are 8 bytes long.
                i += if matches!(extras[i], 0x82 | 0x88 | 0x83 | 0x8C | 0x95) { 8 } else { 4 };
            }
            let data = block[24 + ext..len - 8].to_vec();
            self.pos += len;
            if kind == KEY_FRAME || kind == FRAME {
                let codec = codec.or_else(|| sniff(&data));
                let (day, month) = (((date >> 17) & 0x1f) as u8, ((date >> 22) & 0x0f) as u8);
                return Ok(Some(DavFrame { kind, secs_of_day, ms, day, month, codec, data }));
            }
        }
    }
}

/// Guesses the codec from the first NAL unit after a start code.
pub fn sniff(data: &[u8]) -> Option<Codec> {
    let start = data.windows(3).position(|w| w == [0, 0, 1])? + 3;
    let b = *data.get(start)?;
    let h265 = (b >> 1) & 0x3f;
    if matches!(h265, 32 | 33 | 34 | 19 | 20 | 1) && b & 0x81 == 0 {
        return Some(Codec::H265);
    }
    match b & 0x1f {
        1 | 5 | 6 | 7 | 8 | 9 => Some(Codec::H264),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    /// Reads a real recording downloaded from the recorder (target/davtest/a.dav), when present.
    #[test]
    fn reads_recorder_file() {
        let Ok(f) = std::fs::File::open("target/davtest/a.dav") else {
            eprintln!("no recording file; skipped");
            return;
        };
        let mut r = super::DavReader::new(std::io::BufReader::new(f));
        let mut frames = 0;
        let mut keys = 0;
        let mut first = None;
        let mut dec = crate::stream::VideoDecoder::H265(Box::new(rusty_h265::Decoder::new()));
        let mut pic = None;
        while let Some(fr) = r.next_frame().unwrap() {
            frames += 1;
            if fr.is_key() {
                keys += 1;
            }
            first.get_or_insert((fr.secs_of_day, fr.codec));
            if frames <= 30 {
                if let Some(p) = dec.decode(&fr.data) {
                    pic = Some(p);
                }
            }
        }
        let (secs, codec) = first.unwrap();
        assert_eq!(secs, 17 * 3600 + 24 * 60 + 54);
        assert_eq!(codec, Some(super::Codec::H265));
        assert!(frames > 1000 && keys > 10, "{frames} frames, {keys} key frames");
        let p = pic.expect("no picture");
        assert_eq!((p.w, p.h), (2560, 1440));
        image::save_buffer("shots/dav_frame.png", &p.rgba, p.w as u32, p.h as u32, image::ExtendedColorType::Rgba8).unwrap();
    }
}

#[cfg(test)]
mod timing {
    #[test]
    #[ignore]
    fn print_times() {
        let f = std::fs::File::open("target/davtest/a.dav").unwrap();
        let mut r = super::DavReader::new(std::io::BufReader::new(f));
        let mut n = 0;
        while let Some(fr) = r.next_frame().unwrap() {
            if n < 12 || n % 200 == 0 {
                eprintln!("{n}: secs {} ({:02}:{:02}:{:02}) ms {} key {}", fr.secs_of_day, fr.secs_of_day / 3600, fr.secs_of_day / 60 % 60, fr.secs_of_day % 60, fr.ms, fr.is_key());
            }
            n += 1;
        }
        eprintln!("total {n}");
    }
}

#[cfg(test)]
mod peek {
    /// Saves the first picture of the recording file named by MYCCTV_DAV_FILE to shots/dav_peek.png.
    #[test]
    #[ignore]
    fn first_picture() {
        let path = std::env::var("MYCCTV_DAV_FILE").unwrap();
        let mut r = super::DavReader::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut dec: Option<crate::stream::VideoDecoder> = None;
        let mut started = false;
        while let Some(fr) = r.next_frame().unwrap() {
            let want_day: Option<u8> = std::env::var("MYCCTV_DAV_DAY").ok().and_then(|v| v.parse().ok());
            if want_day.map(|d| d != fr.day).unwrap_or(false) {
                continue;
            }
            if !started && !fr.is_key() {
                continue;
            }
            started = true;
            let d = dec.get_or_insert_with(|| match fr.codec {
                Some(super::Codec::H264) => crate::stream::VideoDecoder::H264(openh264::decoder::Decoder::new().unwrap()),
                _ => crate::stream::VideoDecoder::H265(Box::new(rusty_h265::Decoder::new())),
            });
            if let Some(p) = d.decode(&fr.data) {
                let out = std::env::var("MYCCTV_DAV_OUT").unwrap_or_else(|_| "shots/dav_peek.png".into());
                image::save_buffer(out, &p.rgba, p.w as u32, p.h as u32, image::ExtendedColorType::Rgba8).unwrap();
                return;
            }
        }
        panic!("no picture");
    }
}
