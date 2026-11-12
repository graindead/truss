//! Recolours Truss's taskbar pin (and its desktop / Start menu shortcuts) to match the theme.
//! truss.exe's own file icon can't follow: Windows always shows an exe's first embedded icon.
//!
//! A pinned taskbar button is drawn from its shortcut (in "User Pinned\TaskBar"), not from the
//! running window, and by default that shortcut shows truss.exe's main icon. build.rs embeds a
//! recoloured icon for every theme in truss.exe, so picking a theme points the shortcut(s) that
//! launch this exe at that theme's icon inside the exe (the Rust theme puts them back on the
//! main icon). Nothing is written anywhere else, and only the shortcuts' icon is changed.

/// Points this exe's taskbar pins at the embedded icon with resource ID `icon_id`, or back at the
/// exe's main icon for `None`. Returns how many pins changed. Blocking; run it off the UI thread.
#[cfg(windows)]
pub fn apply(icon_id: Option<u16>) -> Result<usize, String> {
    use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
        STGM_READWRITE,
    };
    use windows::Win32::UI::Shell::{
        IShellLinkW, SHCNE_ASSOCCHANGED, SHCNE_UPDATEITEM, SHCNF_IDLIST, SHCNF_PATHW, SHChangeNotify, ShellLink,
    };
    use windows::core::{HSTRING, Interface, PCWSTR};

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let exe_key = canon(&exe).to_string_lossy().to_lowercase();
    // Taskbar pins, plus shortcuts on the desktop and in the Start menu (one level of folders).
    let mut dirs = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA").map(std::path::PathBuf::from) {
        dirs.push(appdata.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"));
        dirs.push(appdata.join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from) {
        dirs.push(home.join("Desktop"));
        dirs.push(home.join(r"OneDrive\Desktop"));
    }
    let is_lnk = |p: &std::path::Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk"));
    let mut shortcuts: Vec<std::path::PathBuf> = Vec::new();
    for dir in &dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if is_lnk(&p) {
                shortcuts.push(p);
            } else if p.is_dir()
                && let Ok(sub) = std::fs::read_dir(&p)
            {
                shortcuts.extend(sub.flatten().map(|e| e.path()).filter(|p| is_lnk(p)));
            }
        }
    }

    // The icon to show: (file, index), where a negative index is a resource ID. An empty file
    // means "the target's own icon".
    let (want_file, want_index) = match icon_id {
        Some(id) => (exe.to_string_lossy().to_string(), -(id as i32)),
        None => (String::new(), 0),
    };

    let mut changed = 0;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        for lnk in &shortcuts {
            let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { continue };
            let Ok(file) = link.cast::<IPersistFile>() else { continue };
            let lnk_w = HSTRING::from(lnk.as_os_str());
            if file.Load(&lnk_w, STGM_READWRITE).is_err() {
                continue;
            }
            let mut target = [0u16; 1024];
            let mut fd = WIN32_FIND_DATAW::default();
            if link.GetPath(&mut target, &mut fd, 0).is_err() {
                continue;
            }
            let len = target.iter().position(|c| *c == 0).unwrap_or(target.len());
            let target = std::path::PathBuf::from(String::from_utf16_lossy(&target[..len]));
            if canon(&target).to_string_lossy().to_lowercase() != exe_key {
                continue;
            }
            let mut current = [0u16; 1024];
            let mut index = 0i32;
            let _ = link.GetIconLocation(&mut current, &mut index);
            let len = current.iter().position(|c| *c == 0).unwrap_or(current.len());
            let current = String::from_utf16_lossy(&current[..len]);
            if current.eq_ignore_ascii_case(&want_file) && (want_file.is_empty() || index == want_index) {
                continue;
            }
            if link.SetIconLocation(&HSTRING::from(want_file.as_str()), want_index).is_err()
                || file.Save(PCWSTR::null(), true).is_err()
            {
                continue;
            }
            SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW, Some(lnk_w.as_ptr() as *const _), None);
            changed += 1;
        }
        // The taskbar only redraws a pin's icon after a general icon refresh.
        if changed > 0 {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
        CoUninitialize();
    }
    Ok(changed)
}

#[cfg(not(windows))]
pub fn apply(_icon_id: Option<u16>) -> Result<usize, String> {
    Ok(0)
}
