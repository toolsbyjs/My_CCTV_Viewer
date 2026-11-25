//! Local date and time helpers.

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, Timelike};

pub fn now_text() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn file_stamp() -> String {
    Local::now().format("%Y%m%d_%H%M%S").to_string()
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub fn greeting() -> &'static str {
    match Local::now().hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

pub fn long_date() -> String {
    Local::now().format("%A, %-d %B %Y").to_string()
}

pub fn short_time() -> String {
    Local::now().format("%H:%M").to_string()
}

pub fn nice_day(d: NaiveDate) -> String {
    d.format("%a %-d %b %Y").to_string()
}

pub fn is_today(text: &str) -> bool {
    text.starts_with(&Local::now().format("%Y-%m-%d").to_string())
}

pub fn day_text(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn shift_day(d: NaiveDate, days: i64) -> NaiveDate {
    d + Duration::days(days)
}

pub fn seconds_now_of_day() -> u32 {
    let n = Local::now();
    n.hour() * 3600 + n.minute() * 60 + n.second()
}

/// Device playback time format: YYYY_MM_DD_hh_mm_ss
pub fn playback_stamp(d: NaiveDate, secs: u32) -> String {
    let secs = secs.min(86_399);
    format!("{:04}_{:02}_{:02}_{:02}_{:02}_{:02}", d.year(), d.month(), d.day(), secs / 3600, secs / 60 % 60, secs % 60)
}

pub fn hms(secs: u32) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Start and end stamps covering the last `back` seconds (for instant replay).
pub fn last_seconds(back: u32) -> (String, String) {
    let now: NaiveDateTime = Local::now().naive_local();
    let start = now - Duration::seconds(back as i64);
    let f = |t: NaiveDateTime| t.format("%Y_%m_%d_%H_%M_%S").to_string();
    (f(start), f(now))
}

/// "YYYY_MM_DD_hh_mm_ss" back into a day and seconds since midnight.
pub fn parse_stamp(s: &str) -> Option<(NaiveDate, u32)> {
    let p: Vec<u32> = s.split('_').filter_map(|x| x.parse().ok()).collect();
    if p.len() != 6 {
        return None;
    }
    let d = NaiveDate::from_ymd_opt(p[0] as i32, p[1], p[2])?;
    Some((d, p[3] * 3600 + p[4] * 60 + p[5]))
}
