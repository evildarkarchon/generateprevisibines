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

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Build mode: filtered"), "stdout: {stdout}");
    assert!(stdout.contains("Archiver: BSArch"), "stdout: {stdout}");
    assert!(
        stdout.contains("Dry run — external tools are not invoked."),
        "stdout: {stdout}"
    );
}
