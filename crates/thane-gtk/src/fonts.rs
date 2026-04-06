//! Bundled font loading via fontconfig.
//!
//! The JetBrains Mono NL font files are embedded directly into the binary at
//! compile time. On startup, they are extracted to `~/.local/share/thane/fonts/`
//! (if not already present) and registered with fontconfig so Pango/VTE can
//! render text without requiring system-wide font installation.

use std::ffi::CString;
use std::path::{Path, PathBuf};

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigAppFontAddDir(config: *mut libc::c_void, dir: *const libc::c_uchar) -> libc::c_int;
    fn FcConfigGetCurrent() -> *mut libc::c_void;
}

/// The default font family name for bundled JetBrains Mono NL Light.
pub const BUNDLED_FONT_FAMILY: &str = "JetBrains Mono NL Light";

/// Embedded font files (baked into the binary at compile time).
const EMBEDDED_FONTS: &[(&str, &[u8])] = &[
    ("JetBrainsMonoNL-Light.ttf", include_bytes!("../fonts/JetBrainsMonoNL-Light.ttf")),
    ("JetBrainsMonoNL-Regular.ttf", include_bytes!("../fonts/JetBrainsMonoNL-Regular.ttf")),
    ("JetBrainsMonoNL-Medium.ttf", include_bytes!("../fonts/JetBrainsMonoNL-Medium.ttf")),
    ("JetBrainsMonoNL-Bold.ttf", include_bytes!("../fonts/JetBrainsMonoNL-Bold.ttf")),
];

/// Register the bundled fonts with fontconfig.
/// Must be called before any Pango font enumeration or terminal creation.
///
/// Fonts are extracted from the binary to `~/.local/share/thane/fonts/` on
/// first run (or if any font file is missing), then registered via fontconfig.
pub fn load_bundled_fonts() {
    let font_dir = match find_font_dir() {
        Some(dir) => dir,
        None => {
            tracing::warn!("Could not locate or create fonts directory; using system fonts");
            return;
        }
    };

    let dir_str = font_dir.to_string_lossy();
    let c_dir = match CString::new(dir_str.as_bytes()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Invalid font directory path: {e}");
            return;
        }
    };

    let result = unsafe {
        let config = FcConfigGetCurrent();
        FcConfigAppFontAddDir(config, c_dir.as_ptr() as *const libc::c_uchar)
    };

    if result != 0 {
        tracing::info!("Loaded bundled fonts from {}", dir_str);
    } else {
        tracing::warn!("Failed to register bundled fonts from {}", dir_str);
    }
}

/// Locate the bundled fonts directory, extracting embedded fonts if needed.
fn find_font_dir() -> Option<PathBuf> {
    // 1. Next to the running binary (installed/packaged layout).
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        let candidate = exe_dir.join("fonts");
        if candidate.is_dir() && has_all_fonts(&candidate) {
            return Some(candidate);
        }
    }

    // 2. System-wide install location.
    let system_path = Path::new("/usr/share/thane/fonts");
    if system_path.is_dir() && has_all_fonts(system_path) {
        return Some(system_path.to_path_buf());
    }

    // 3. User-local directory — extract embedded fonts here if missing.
    if let Some(data_dir) = dirs::data_dir() {
        let user_font_dir = data_dir.join("thane").join("fonts");
        if ensure_embedded_fonts(&user_font_dir) {
            return Some(user_font_dir);
        }
    }

    None
}

/// Check whether all expected font files exist in a directory.
fn has_all_fonts(dir: &Path) -> bool {
    EMBEDDED_FONTS.iter().all(|(name, _)| dir.join(name).is_file())
}

/// Ensure all embedded font files are written to `dir`, creating the
/// directory if needed. Returns `true` if the directory is ready.
fn ensure_embedded_fonts(dir: &Path) -> bool {
    if has_all_fonts(dir) {
        return true;
    }

    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("Failed to create fonts directory {}: {e}", dir.display());
        return false;
    }

    for (name, data) in EMBEDDED_FONTS {
        let path = dir.join(name);
        if !path.is_file() {
            if let Err(e) = std::fs::write(&path, data) {
                tracing::warn!("Failed to write font {}: {e}", path.display());
                return false;
            }
            tracing::info!("Extracted bundled font: {}", path.display());
        }
    }

    true
}
