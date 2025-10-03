use anyhow::{bail, Context, Result};
use log::{info, warn};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_RETURN,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SetForegroundWindow, SendMessageW, WM_CLOSE,
};

/// FO4Edit script names
pub const SCRIPT_MERGE_COMBINED: &str = "Batch_FO4MergeCombinedObjectsAndCheck.pas";
pub const SCRIPT_MERGE_PREVIS: &str = "Batch_FO4MergePrevisandCleanRefr.pas";

/// Success indicator in FO4Edit logs
const LOG_SUCCESS: &str = "Completed: No Errors.";

/// Error indicator in FO4Edit logs
const LOG_ERROR: &str = "Error:";

/// Runner for FO4Edit.exe operations
///
/// Handles the complex FO4Edit automation workflow:
/// 1. Create Plugins.txt file
/// 2. Launch with arguments
/// 3. **Send ENTER keystroke to Module Selection dialog** (REQUIRED WORKAROUND)
/// 4. Wait for completion
/// 5. Force close window (despite -autoexit flag)
/// 6. Parse log for errors
///
/// IMPORTANT: The keystroke automation is NOT code smell.
/// FO4Edit has no true headless mode - the Module Selection dialog
/// MUST be dismissed via keystroke, even with -autoexit flag.
pub struct FO4EditRunner {
    fo4edit_exe: PathBuf,
    fallout4_dir: PathBuf,
}

impl FO4EditRunner {
    /// Create a new FO4Edit runner
    pub fn new(fo4edit_exe: impl AsRef<Path>, fallout4_dir: impl AsRef<Path>) -> Self {
        Self {
            fo4edit_exe: fo4edit_exe.as_ref().to_path_buf(),
            fallout4_dir: fallout4_dir.as_ref().to_path_buf(),
        }
    }

    /// Run FO4Edit script to merge combined objects
    ///
    /// Script: Batch_FO4MergeCombinedObjectsAndCheck.pas
    /// Merges PrecombineObjects.esp into the main plugin
    pub fn merge_combined_objects(&self, plugin_name: &str) -> Result<()> {
        self.run_script(plugin_name, SCRIPT_MERGE_COMBINED, "Merge Combined Objects")
    }

    /// Run FO4Edit script to merge previs data
    ///
    /// Script: Batch_FO4MergePrevisandCleanRefr.pas
    /// Merges Previs.esp into the main plugin
    pub fn merge_previs(&self, plugin_name: &str) -> Result<()> {
        self.run_script(plugin_name, SCRIPT_MERGE_PREVIS, "Merge Previs")
    }

    /// Run an FO4Edit script with full automation
    fn run_script(&self, plugin_name: &str, script_name: &str, operation: &str) -> Result<()> {
        info!("Running FO4Edit: {}", operation);

        // Create temporary Plugins.txt
        let temp_dir = std::env::temp_dir();
        let plugins_file = temp_dir.join(format!("FO4Edit_Plugins_{}.txt", plugin_name));
        let log_file = temp_dir.join(format!("FO4Edit_Log_{}.txt", plugin_name));

        // Write plugin name to Plugins.txt
        fs::write(&plugins_file, plugin_name)
            .with_context(|| format!("Failed to create Plugins.txt: {}", plugins_file.display()))?;

        // Delete old log if it exists
        if log_file.exists() {
            fs::remove_file(&log_file)?;
        }

        // Build command arguments
        let args = vec![
            "-fo4".to_string(),
            "-autoexit".to_string(),
            format!("-P:{}", plugins_file.display()),
            format!("-Script:{}", script_name),
            format!("-Mod:{}", plugin_name),
            format!("-log:{}", log_file.display()),
        ];

        info!("Executing: {} {}", self.fo4edit_exe.display(), args.join(" "));

        // Launch FO4Edit
        let mut child = Command::new(&self.fo4edit_exe)
            .args(&args)
            .current_dir(&self.fallout4_dir)
            .spawn()
            .with_context(|| format!("Failed to launch FO4Edit: {}", self.fo4edit_exe.display()))?;

        // Wait for window to appear, then send ENTER keystroke
        // This dismisses the Module Selection dialog
        self.send_enter_keystroke()?;

        // Wait for log file to be created (indicates script is running)
        self.wait_for_log_file(&log_file)?;

        // Wait a bit more for script to complete
        thread::sleep(Duration::from_secs(5));

        // Force close the main window (autoexit doesn't always work)
        self.close_fo4edit_window();

        // Clean up child process
        let _ = child.wait(); // May already be closed

        // Parse log for success/errors
        self.check_log_for_errors(&log_file, operation)?;

        // Cleanup temp files
        let _ = fs::remove_file(&plugins_file);

        info!("FO4Edit {} completed successfully", operation);
        Ok(())
    }

    /// Send ENTER keystroke to dismiss Module Selection dialog
    ///
    /// IMPORTANT: This is a REQUIRED workaround, not code smell.
    /// FO4Edit shows Module Selection even with -autoexit flag.
    #[cfg(windows)]
    fn send_enter_keystroke(&self) -> Result<()> {
        info!("Waiting for Module Selection window...");

        // Wait for window to appear
        thread::sleep(Duration::from_secs(3));

        // Find FO4Edit window
        let window_title = windows::core::w!("FO4Edit");
        let hwnd = unsafe { FindWindowW(None, window_title) };

        let hwnd = match hwnd {
            Ok(h) if !h.0.is_null() => h,
            _ => {
                warn!("FO4Edit window not found, keystroke automation may fail");
                return Ok(());
            }
        };

        // Bring window to foreground
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }

        thread::sleep(Duration::from_millis(500));

        // Send ENTER key press
        let mut inputs = [INPUT::default(); 2];

        // Key down
        inputs[0].r#type = INPUT_KEYBOARD;
        inputs[0].Anonymous.ki = KEYBDINPUT {
            wVk: VK_RETURN,
            wScan: 0,
            dwFlags: Default::default(),
            time: 0,
            dwExtraInfo: 0,
        };

        // Key up
        inputs[1].r#type = INPUT_KEYBOARD;
        inputs[1].Anonymous.ki = KEYBDINPUT {
            wVk: VK_RETURN,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };

        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }

        info!("Sent ENTER keystroke to Module Selection");
        Ok(())
    }

    #[cfg(not(windows))]
    fn send_enter_keystroke(&self) -> Result<()> {
        // Non-Windows platforms cannot automate FO4Edit
        bail!("FO4Edit automation requires Windows");
    }

    /// Wait for log file to be created
    fn wait_for_log_file(&self, log_file: &Path) -> Result<()> {
        info!("Waiting for log file creation...");

        for _ in 0..30 {
            // Wait up to 30 seconds
            if log_file.exists() {
                info!("Log file created");
                return Ok(());
            }
            thread::sleep(Duration::from_secs(1));
        }

        warn!("Log file not created within timeout");
        Ok(())
    }

    /// Close FO4Edit main window
    ///
    /// FO4Edit's -autoexit flag doesn't always work reliably.
    /// We force close the window to ensure cleanup.
    #[cfg(windows)]
    fn close_fo4edit_window(&self) {
        info!("Closing FO4Edit window...");

        let window_title = windows::core::w!("FO4Edit");
        let hwnd = unsafe { FindWindowW(None, window_title) };

        if let Ok(hwnd) = hwnd {
            if !hwnd.0.is_null() {
                unsafe {
                    SendMessageW(hwnd, WM_CLOSE, None, None);
                }
                info!("Sent close message to FO4Edit");
            }
        }

        // Fallback: taskkill if still running
        thread::sleep(Duration::from_secs(2));
        let _ = Command::new("taskkill")
            .args(&["/F", "/IM", "FO4Edit.exe"])
            .output();
    }

    #[cfg(not(windows))]
    fn close_fo4edit_window(&self) {
        // Non-Windows platforms - process should exit normally
    }

    /// Check log file for errors
    fn check_log_for_errors(&self, log_file: &Path, operation: &str) -> Result<()> {
        if !log_file.exists() {
            bail!("Log file not found: {}", log_file.display());
        }

        let log_content = fs::read_to_string(log_file)
            .context("Failed to read FO4Edit log")?;

        // Check for errors
        if log_content.contains(LOG_ERROR) {
            bail!(
                "FO4Edit {} failed: '{}' found in log.\n\
                Log file: {}",
                operation,
                LOG_ERROR,
                log_file.display()
            );
        }

        // For merge operations, check for success message
        if !log_content.contains(LOG_SUCCESS) {
            warn!(
                "FO4Edit log doesn't contain success message '{}', but no errors detected",
                LOG_SUCCESS
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_creation() {
        let runner = FO4EditRunner::new("FO4Edit.exe", "F:\\Games\\Fallout4");
        assert_eq!(runner.fo4edit_exe, PathBuf::from("FO4Edit.exe"));
    }
}
