//! OS "reduce motion" preference (best effort, read once at start; §2.6).
//! `AUTORENAME_REDUCE_MOTION` and Settings › Appearance override it.

#[cfg(target_os = "linux")]
pub fn os_reduce_motion() -> bool {
    // GNOME / GTK: org.gnome.desktop.interface enable-animations.
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
}

#[cfg(target_os = "macos")]
pub fn os_reduce_motion() -> bool {
    // accessibilityDisplayShouldReduceMotion ↔ com.apple.universalaccess reduceMotion.
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}

#[cfg(windows)]
pub fn os_reduce_motion() -> bool {
    // SPI_GETCLIENTAREAANIMATION (0x1042): FALSE = animations off.
    // SAFETY: the declaration matches the documented Win32 signature
    // `BOOL SystemParametersInfoW(UINT, UINT, PVOID, UINT)` in user32.
    #[allow(unsafe_code)]
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            param: u32,
            pv: *mut core::ffi::c_void,
            win_ini: u32,
        ) -> i32;
    }
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut on: i32 = 1;
    // SAFETY: pvParam points to a BOOL as the API requires.
    #[allow(unsafe_code)]
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut on).cast(), 0) };
    ok != 0 && on == 0
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn os_reduce_motion() -> bool {
    false
}
