use anyhow::{Context, Result};
use log::{LevelFilter, warn};
use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::core::PCWSTR;

/// Get the product version string from a Windows executable
///
/// Reads version information from a Windows PE file using the Windows API
/// (`GetFileVersionInfoW` and `VerQueryValueW`). This extracts the file version
/// from the `VS_FIXEDFILEINFO` structure embedded in the executable.
///
/// # Arguments
///
/// * `exe_path` - Path to the Windows executable (.exe or .dll) to query
///
/// # Returns
///
/// Returns a version string in the format `major.minor.build.revision` (e.g., "1.10.163.0")
///
/// # Errors
///
/// This function will return an error if:
/// - The path is not valid UTF-8
/// - The file has no version information resource
/// - Reading version info fails (file doesn't exist, access denied, corrupted PE file)
/// - Version value query fails (invalid or corrupted version resource)
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// let version = get_file_version(Path::new("C:\\Windows\\System32\\notepad.exe"))?;
/// println!("Notepad version: {}", version); // e.g., "10.0.19041.1"
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Platform Support
///
/// **Windows only.** This function uses Windows-specific APIs and will not compile
/// on other platforms.
#[allow(unsafe_code)]
pub fn get_file_version(exe_path: &Path) -> Result<String> {
    let path_wide: Vec<u16> = exe_path
        .to_str()
        .context("Invalid path")?
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: This unsafe block is required for Windows FFI to read version information
    // from PE executable resources. The following invariants are maintained:
    //
    // 1. `path_wide` is a properly null-terminated UTF-16 string created from a valid path.
    //    It remains valid for the entire duration of the unsafe block.
    //
    // 2. `GetFileVersionInfoSizeW` is called with a valid PCWSTR pointer. The Windows API
    //    handles invalid paths gracefully by returning 0 (checked before proceeding).
    //
    // 3. `buffer` is allocated with the exact size returned by GetFileVersionInfoSizeW,
    //    ensuring sufficient space for the version info data.
    //
    // 4. `GetFileVersionInfoW` writes into our owned buffer with correct size bounds.
    //    Errors are propagated via Result<()> and checked before proceeding.
    //
    // 5. `VerQueryValueW` returns a pointer into the buffer we own, which remains valid
    //    because we don't move or drop the buffer until after we're done reading.
    //    The returned pointer lifetime is tied to the buffer lifetime.
    //
    // 6. The pointer is cast to `VS_FIXEDFILEINFO` which has #[repr(C)] layout matching
    //    the Windows SDK structure exactly. We verify the pointer is non-null before
    //    dereferencing.
    //
    // 7. Dereferencing `file_info` is safe because: (a) pointer is non-null (checked),
    //    (b) points to valid memory in our buffer, (c) buffer is properly aligned,
    //    (d) VS_FIXEDFILEINFO has correct #[repr(C)] layout.
    //
    // 8. Bit operations on u32 values from the structure are safe arithmetic operations
    //    that cannot overflow (masked to 16 bits).
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
        )
        .ok()
        .context(format!(
            "Failed to read version info for {}",
            exe_path.display()
        ))?;

        // Query for the VS_FIXEDFILEINFO structure
        let mut len: u32 = 0;
        let mut info_ptr: *mut u8 = std::ptr::null_mut();
        let subblock: Vec<u16> = "\\".encode_utf16().chain(std::iter::once(0)).collect();

        VerQueryValueW(
            buffer.as_ptr() as *const _,
            PCWSTR(subblock.as_ptr()),
            &mut info_ptr as *mut _ as *mut *mut _,
            &mut len,
        )
        .ok()
        .context(format!(
            "Failed to query version value for {}",
            exe_path.display()
        ))?;

        // Cast to VS_FIXEDFILEINFO structure
        // We need the file version (dwFileVersionMS and dwFileVersionLS)
        let file_info = info_ptr as *const VS_FIXEDFILEINFO;
        if file_info.is_null() {
            anyhow::bail!("Version info pointer is null");
        }

        let version = &*file_info;

        // Extract version components from DWORD pairs
        // Each DWORD contains two 16-bit version numbers (HIWORD and LOWORD)
        let major = (version.dwFileVersionMS >> 16) & 0xFFFF;
        let minor = version.dwFileVersionMS & 0xFFFF;
        let build = (version.dwFileVersionLS >> 16) & 0xFFFF;
        let revision = version.dwFileVersionLS & 0xFFFF;

        // Sanity check: version components should be reasonable (0-65535 by design, but typically < 100 for major/minor)
        // This catches corrupted version resources early
        if major > 100 || minor > 100 {
            warn!(
                "Suspicious version numbers detected in {}: {}.{}.{}.{} (major or minor > 100)",
                exe_path.display(),
                major,
                minor,
                build,
                revision
            );
        }

        Ok(format!("{}.{}.{}.{}", major, minor, build, revision))
    }
}

//noinspection RsStructNaming
/// VS_FIXEDFILEINFO structure (from Windows SDK)
///
/// **IMPORTANT**: Field names must match Windows SDK exactly for FFI safety.
/// The struct uses `#[repr(C)]` for correct memory layout when casting from
/// Windows API pointers. Renaming fields to snake_case would break the layout.
///
/// # Safety
///
/// This struct is cast from a pointer returned by `VerQueryValueW`. The field
/// order and sizes must match the Windows SDK definition exactly. All fields
/// are `u32` (DWORD) types matching the Windows SDK structure.
///
/// # Naming Convention
///
/// The `#[allow(non_snake_case)]` and `#[allow(non_camel_case_types)]` attributes
/// are required because this is a foreign function interface (FFI) struct that
/// must match the Windows SDK naming exactly. Do not "fix" this naming.
#[allow(non_camel_case_types)]
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
///
/// Sets up the `env_logger` to write all log output at INFO level and above
/// to a file named `GeneratePrevisibines.log` in the system's temporary directory.
/// The log file is created or truncated if it already exists.
///
/// # Returns
///
/// Returns the full path to the log file (e.g., `C:\Users\username\AppData\Local\Temp\GeneratePrevisibines.log`)
///
/// # Errors
///
/// This function will return an error if:
/// - The log file cannot be created in the temp directory (insufficient permissions, disk full)
/// - The env_logger initialization fails
///
/// # Examples
///
/// ```no_run
/// let log_path = init_logging()?;
/// println!("Logging to: {}", log_path.display());
/// log::info!("Application started");
/// # Ok::<(), anyhow::Error>(())
/// ```
///
/// # Notes
///
/// - All subsequent `log::info!`, `log::warn!`, and `log::error!` calls will write to this file
/// - The log file persists after the application exits for debugging purposes
/// - Log level is fixed at INFO; use `RUST_LOG` environment variable for more control
pub fn init_logging() -> Result<PathBuf> {
    let temp_dir = env::temp_dir();
    let log_file_path = temp_dir.join("GeneratePrevisibines.log");

    // Create or truncate the log file
    let log_file = File::create(&log_file_path).context("Failed to create log file in %TEMP%")?;

    // Set up env_logger to write to the file
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .target(env_logger::Target::Pipe(Box::new(log_file)))
        .init();

    Ok(log_file_path)
}

/// Get a simpler version string (just major.minor if available)
///
/// Calls `get_file_version` and extracts only the major and minor version numbers,
/// discarding the build and revision numbers for cleaner display.
///
/// # Arguments
///
/// * `exe_path` - Path to the Windows executable (.exe or .dll) to query
///
/// # Returns
///
/// Returns a simplified version string in the format `major.minor` (e.g., "1.10"),
/// or "Unknown" if version information cannot be read.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// let version = get_simple_version(Path::new("C:\\Games\\Fallout4\\FO4Edit.exe"));
/// println!("FO4Edit version: {}", version); // e.g., "4.0" or "Unknown"
/// ```
///
/// # Notes
///
/// - This function never returns an error; it returns "Unknown" on failure
/// - Useful for display purposes where full version is too verbose
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
    #[test]
    #[ignore] // Requires actual executable file
    fn test_get_file_version() {
        // This test would need a real Windows executable to work
        // We can't test it without one
    }
}
