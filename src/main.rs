#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut opts = mycctv_core::Options::default();
    let all: Vec<String> = std::env::args().skip(1).collect();
    // The release program is a windowed app; the check tools print to the command prompt they
    // were started from, so attach to it.
    #[cfg(windows)]
    if all.first().map(|a| a.starts_with("--probe") || a.ends_with("-check") || a == "--tv-server").unwrap_or(false) {
        extern "system" {
            fn AttachConsole(process: u32) -> i32;
        }
        // ATTACH_PARENT_PROCESS
        unsafe {
            AttachConsole(u32::MAX);
        }
    }
    if all.first().map(|a| a == "--tv-server").unwrap_or(false) {
        if let Err(e) = mycctv_core::tv_server_forever() {
            println!("FAILED: {e}");
        }
        return;
    }
    if all.first().map(|a| a == "--probe-device").unwrap_or(false) {
        let n = |i: usize, d: u32| all.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
        let r = mycctv_core::probe_saved(n(1, 1) as usize, n(2, 1), n(3, 1) as u8, all.get(4).map(std::path::PathBuf::from));
        match r {
            Ok(m) => println!("OK: {m}"),
            Err(e) => println!("FAILED: {e}"),
        }
        return;
    }
    if all.first().map(|a| a == "--config-check").unwrap_or(false) {
        let r = mycctv_core::config_check(all.get(1).and_then(|v| v.parse().ok()).unwrap_or(1), all.get(2).map(|s| s.as_str()).unwrap_or("ChannelTitle[0].Name"));
        println!("{}", match r { Ok(m) => format!("OK: {m}"), Err(e) => format!("FAILED: {e}") });
        return;
    }
    if all.first().map(|a| a == "--playback-check").unwrap_or(false) {
        let r = mycctv_core::playback_check(all.get(1).and_then(|v| v.parse().ok()).unwrap_or(1), all.get(2).and_then(|v| v.parse().ok()).unwrap_or(1), all.get(3).map(|s| s.as_str()).unwrap_or(""), all.get(4).map(|s| s.as_str()).unwrap_or("00:00:00"), all.get(5).map(std::path::PathBuf::from));
        println!("{}", match r { Ok(m) => format!("OK: {m}"), Err(e) => format!("FAILED: {e}") });
        return;
    }
    if all.first().map(|a| a == "--probe").unwrap_or(false) {
        let get = |i: usize| all.get(i).cloned().unwrap_or_default();
        let pic = all.get(4).map(std::path::PathBuf::from);
        match mycctv_core::probe(&get(1), &get(2), &get(3), pic) {
            Ok(m) => println!("OK: {m}"),
            Err(e) => {
                println!("FAILED: {e}");
                std::process::exit(2);
            }
        }
        return;
    }
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            opts.screenshot = args.next().map(std::path::PathBuf::from);
        } else if a == "--size" {
            opts.size = args.next().and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))));
        } else if a == "--page" {
            opts.page = args.next();
        } else if let Some(v) = a.strip_prefix("--screenshot=") {
            opts.screenshot = Some(v.into());
        }
    }
    #[cfg(not(target_os = "android"))]
    if let Err(e) = mycctv_core::run_desktop(opts) {
        eprintln!("MyCCTV could not start: {e}");
        std::process::exit(1);
    }
}
