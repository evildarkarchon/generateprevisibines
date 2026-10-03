use std::ffi::OsString;
use std::process::{Command, Stdio};

use tempfile::tempdir;

#[test]
fn legacy_and_modern_arguments_share_the_production_dry_run_path() {
    let isolated_working_directory = tempdir().unwrap();
    let missing_fallout4_directory = isolated_working_directory.path().join("Missing Fallout 4");
    let mut attached_legacy_fo4 = OsString::from("-fO4:");
    attached_legacy_fo4.push(&missing_fallout4_directory);

    let output = Command::new(env!("CARGO_BIN_EXE_generateprevisibines"))
        .args(["-FiLtErEd", "-BsArCh"])
        .arg(attached_legacy_fo4)
        .args(["MyMod.esp", "--dry-run"])
        .current_dir(isolated_working_directory.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    // A deliberate, completed exit is `0`, not merely "success".
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert!(stdout.contains("Build mode: filtered"), "stdout: {stdout}");
    assert!(stdout.contains("Archiver: BSArch"), "stdout: {stdout}");
    assert!(
        stdout.contains(
            "Planned steps:\n  1 - Generate Precombines Via CK\n  2 - Merge PrecombineObjects.esp Via xEdit\n  3 - Create BA2 Archive from Precombines\n  6 - Generate Previs Via CK\n  7 - Merge Previs.esp Via xEdit\n  8 - Add Previs files to BA2 Archive"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(
            "Note: current production capability (Steps 1 and 2) would execute 2 of 6 planned steps."
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("Dry run — external tools are not invoked."),
        "stdout: {stdout}"
    );
    assert_eq!(
        std::fs::read_dir(isolated_working_directory.path())
            .unwrap()
            .count(),
        0,
        "dry-run must not create logs or workflow artifacts"
    );
}

/// V2.99 Xbox is a clean build that skips only `CompressPSG`, so its dry run plans all eight
/// steps, Steps 4 and 5 included (batch 219, 296, 305 test only for `filtered`).
#[test]
fn an_xbox_dry_run_lists_all_eight_planned_steps() {
    let isolated_working_directory = tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_generateprevisibines"))
        .args(["-xbox", "MyMod.esp", "--dry-run"])
        .current_dir(isolated_working_directory.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    assert!(stdout.contains("Build mode: xbox"), "stdout: {stdout}");
    assert!(
        stdout.contains(
            "Planned steps:\n  1 - Generate Precombines Via CK\n  2 - Merge PrecombineObjects.esp Via xEdit\n  3 - Create BA2 Archive from Precombines\n  4 - Compress PSG Via CK\n  5 - Build CDX Via CK\n  6 - Generate Previs Via CK\n  7 - Merge Previs.esp Via xEdit\n  8 - Add Previs files to BA2 Archive\n"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(
            "Note: current production capability (Steps 1 and 2) would execute 2 of 8 planned steps."
        ),
        "stdout: {stdout}"
    );
}

#[test]
fn a_usage_error_exits_with_code_two() {
    let isolated_working_directory = tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_generateprevisibines"))
        .arg("--definitely-not-a-flag")
        .current_dir(isolated_working_directory.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A stop before any Workflow Run exists keeps today's bare `ERROR - …` and exits `1`.
///
/// There is no session log yet, so there is no `See Log at` to print and no failure line to
/// attach it to. Naming the plugin on the command line is what makes the run non-interactive,
/// so nothing here can block on a prompt.
#[test]
fn a_stop_before_the_run_exists_exits_with_code_one_and_no_log_line() {
    let isolated_working_directory = tempdir().unwrap();
    let missing_fallout4_directory = isolated_working_directory.path().join("Missing Fallout 4");

    let output = Command::new(env!("CARGO_BIN_EXE_generateprevisibines"))
        .arg("--FO4")
        .arg(&missing_fallout4_directory)
        .arg("MyMod.esp")
        .current_dir(isolated_working_directory.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    // Printed once: `main` reports it, and no Workflow Run existed to report it first.
    assert_eq!(stderr.matches("ERROR - ").count(), 1, "stderr: {stderr}");
    assert!(!stdout.contains("See Log at"), "stdout: {stdout}");
    assert!(!stdout.contains("Build of Patch"), "stdout: {stdout}");
}
