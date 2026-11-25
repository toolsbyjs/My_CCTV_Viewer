//! The few things that differ between Windows and Android / Google TV.

#[cfg(target_os = "android")]
static ANDROID_APP: std::sync::OnceLock<winit::platform::android::activity::AndroidApp> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
pub fn set_android_app(a: winit::platform::android::activity::AndroidApp) {
    let _ = ANDROID_APP.set(a);
}

#[cfg(target_os = "android")]
pub fn android_app() -> Option<&'static winit::platform::android::activity::AndroidApp> {
    ANDROID_APP.get()
}

pub const IS_ANDROID: bool = cfg!(target_os = "android");

/// Shows the on-screen keyboard while a text field is being edited (Android only;
/// desktop keyboards need nothing).
pub fn keyboard_wanted(_wanted: bool) {
    #[cfg(target_os = "android")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static SHOWN: AtomicBool = AtomicBool::new(false);
        if SHOWN.swap(_wanted, Ordering::Relaxed) != _wanted {
            if let Some(a) = ANDROID_APP.get() {
                if _wanted {
                    a.show_soft_input(true);
                } else {
                    a.hide_soft_input(true);
                }
            }
        }
    }
}
