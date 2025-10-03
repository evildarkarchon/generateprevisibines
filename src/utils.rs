use anyhow::{Context, Result};
use log::LevelFilter;
use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::core::PCWSTR;

/// Get the product version string from a Windows executable
/// This uses the Windows API to read version info from the PE file
pub fn get_file_version(exe_path: &Path) -> Result<String> {
    let path_wide: Vec<u16> = exe_path
        .to_str()
        .context("Invalid path")?
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        // Get the size of the version info
        let size = GetFileVersionInfoSizeW(PCWSTR(path_wide.as_ptr()), None);
        if size == 0 {
            anyhow::bail!("No version info found for {}", exe_path.display());
        }

        // Allocate buffer and read version info
        let mut buffer = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(path_wide.as_ptr()),
            Some(0),
            size,
            buffer.as_mut_ptr() as *mut _,
        ).ok()
        .context(format!("Failed to read version info for {}", exe_path.display()))?;

        // Query for the VS_FIXEDFILEINFO structure
        let mut len: u32 = 0;
        let mut info_ptr: *mut u8 = std::ptr::null_mut();
        let subblock: Vec<u16> = "\\".encode_utf16().chain(std::iter::once(0)).collect();

        VerQueryValueW(
            buffer.as_ptr() as *const _,
            PCWSTR(subblock.as_ptr()),
            &mut info_ptr as *mut _ as *mut *mut _,
            &mut len,
        ).ok()
        .context(format!("Failed to query version value for {}", exe_path.display()))?;

        // Cast to VS_FIXEDFILEINFO structure
        // We need the file version (dwFileVersionMS and dwFileVersionLS)
        let file_info = info_ptr as *const VS_FIXEDFILEINFO;
        if file_info.is_null() {
            anyhow::bail!("Version info pointer is null");
        }

        let version = &*file_info;
        let major = (version.dwFileVersionMS >> 16) & 0xFFFF;
        let minor = version.dwFileVersionMS & 0xFFFF;
        let build = (version.dwFileVersionLS >> 16) & 0xFFFF;
        let revision = version.dwFileVersionLS & 0xFFFF;

        Ok(format!("{}.{}.{}.{}", major, minor, build, revision))
    }
}

/// VS_FIXEDFILEINFO structure (from Windows SDK)
/// Field names match Windows SDK exactly - do not rename to snake_case
#[repr(C)]
#[allow(non_snake_case)]
struct VS_FIXEDFILEINFO {
    dwSignature: u32,
    dwStrucVersion: u32,
    dwFileVersionMS: u32,
    dwFileVersionLS: u32,
    dwProductVersionMS: u32,
    dwProductVersionLS: u32,
    dwFileFlagsMask: u32,
    dwFileFlags: u32,
    dwFileOS: u32,
    dwFileType: u32,
    dwFileSubtype: u32,
    dwFileDateMS: u32,
    dwFileDateLS: u32,
}

/// Initialize logging to a file in %TEMP%
/// Returns the path to the log file
pub fn init_logging() -> Result<PathBuf> {
    let temp_dir = env::temp_dir();
    let log_file_path = temp_dir.join("GeneratePrevisibines.log");

    // Create or truncate the log file
    let log_file = File::create(&log_file_path)
        .context("Failed to create log file in %TEMP%")?;

    // Set up env_logger to write to the file
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .target(env_logger::Target::Pipe(Box::new(log_file)))
        .init();

    Ok(log_file_path)
}

/// Get a simpler version string (just major.minor if available)
pub fn get_simple_version(exe_path: &Path) -> String {
    match get_file_version(exe_path) {
        Ok(version) => {
            // Try to extract just major.minor
            let parts: Vec<&str> = version.split('.').collect();
            if parts.len() >= 2 {
                format!("{}.{}", parts[0], parts[1])
            } else {
                version
            }
        }
        Err(_) => "Unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires actual executable file
    fn test_get_file_version() {
        // This test would need a real Windows executable to work
        // We can't test it without one
    }
}
