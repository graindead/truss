//! Window transparency (Settings > Appearance, beta): frosted glass. The window is created
//! transparent with no GDI surface behind its content (vendor/iced_winit) and presented through
//! a DirectComposition visual (vendor/iced_wgpu); Windows then blurs and tints what's behind it
//! ("acrylic blur behind"), and the app's own background is drawn half see-through over that.
//! Takes effect when Truss starts.
//!
//! Tried and dropped: Windows 11's DWMWA_SYSTEMBACKDROP_TYPE (Mica/Acrylic) has no effect on a
//! window presented this way, extending the frame into the client area paints it black, and
//! without the blur the area behind the content is filled white, so there's no plain
//! see-through mode.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backdrop {
    #[default]
    Off,
    /// What's behind the window, blurred and tinted.
    Frosted,
}

impl Backdrop {
    pub const ALL: [Backdrop; 2] = [Backdrop::Off, Backdrop::Frosted];

    pub fn key(self) -> &'static str {
        match self {
            Backdrop::Off => "off",
            Backdrop::Frosted => "frosted",
        }
    }

    pub fn from_key(k: &str) -> Self {
        match k {
            // Earlier names of the experiment.
            "acrylic" | "mica" | "clear" => Backdrop::Frosted,
            k => Self::ALL.into_iter().find(|b| b.key() == k).unwrap_or_default(),
        }
    }

    /// How opaque the app's own background is drawn over what's behind.
    pub fn alpha(self) -> f32 {
        match self {
            Backdrop::Off => 1.0,
            Backdrop::Frosted => 0.5,
        }
    }
}

impl std::fmt::Display for Backdrop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Backdrop::Off => "Off",
            Backdrop::Frosted => "Frosted glass",
        })
    }
}

/// Turns on the blur behind the window `hwnd` (from `iced::window::raw_id`), tinted with `tint`
/// (0-255 RGB).
#[cfg(windows)]
pub fn apply(hwnd: u64, b: Backdrop, tint: [u8; 3]) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::core::{s, w};

    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        /// 0xAABBGGRR
        gradient: u32,
        animation: u32,
    }
    #[repr(C)]
    struct CompositionData {
        attribute: u32,
        data: *mut core::ffi::c_void,
        size: usize,
    }
    type SetWca = unsafe extern "system" fn(HWND, *mut CompositionData) -> i32;
    const WCA_ACCENT_POLICY: u32 = 19;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

    if b == Backdrop::Off {
        return;
    }
    let hwnd = HWND(hwnd as usize as *mut core::ffi::c_void);
    unsafe {
        // Not in the Windows SDK headers, so it's looked up by name.
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
        let Some(f) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
        let set: SetWca = std::mem::transmute(f);
        let mut accent = AccentPolicy {
            state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
            flags: 2,
            gradient: 0x50 << 24 | (tint[2] as u32) << 16 | (tint[1] as u32) << 8 | tint[0] as u32,
            animation: 0,
        };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut accent as *mut AccentPolicy as *mut _,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data);
    }
}

#[cfg(not(windows))]
pub fn apply(_hwnd: u64, _b: Backdrop, _tint: [u8; 3]) {}
