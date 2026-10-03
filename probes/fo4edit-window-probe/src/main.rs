//! Throwaway probe for the map ticket "Probe FO4Edit's windows and dismissal on Windows" (#40).
//!
//! Each scenario launches FO4Edit the way the batch's `:RunScript` does (V2.99 lines 530–564),
//! with a probe script against a stub plugin it creates in `Data`. It waits for Module
//! Selection, tries **one** dismissal rung, waits for the unattended log, then runs the close
//! sequence from the FO4Edit design and checks whether the plugin was really saved. Everything
//! observed goes into a JSON report under `reports/`.
//!
//! The probe never kills FO4Edit, never sends input unless Module Selection is the foreground
//! window, and deletes its stub plugin (plus xEdit's `.save.*` files and backups of it) at the
//! end unless `--keep` is given.

mod win;

use std::fs;
use std::io::{self, BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

use win::{Hwnd, WindowInfo};

const BM_CLICK: u32 = 0x00F5;
const WM_CLOSE: u32 = 0x0010;
const WM_COMMAND: u32 = 0x0111;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const VK_RETURN: usize = 0x0D;
const BN_CLICKED: usize = 0;
/// `WM_KEYDOWN` lParam for ENTER: repeat count 1, scan code 0x1C.
const ENTER_DOWN_LPARAM: isize = 0x001C_0001;
/// `WM_KEYUP` lParam for ENTER: as above plus previous-state (bit 30) and transition (bit 31).
const ENTER_UP_LPARAM: isize = 0xC01C_0001;

const PLUGIN_NAME: &str = "GPProbe.esp";
const MARKER: &[u8] = b"GPProbeMarker";
const MODULE_SELECTION: &str = "Module Selection";
const PROBE_SCRIPT: &str = include_str!("../GPProbe_WindowProbe.pas");

#[derive(Parser)]
#[command(about = "Probe FO4Edit 4.1.5q Module Selection dismissal and close on Windows")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run one scenario.
    Run(RunArgs),
    /// Run the standard scenario matrix back to back, prompting before each.
    Matrix(CommonArgs),
    /// Dump the windows of a running process (no input, no messages).
    Dump {
        /// Process ID to dump.
        pid: u32,
    },
}

#[derive(Args, Clone)]
struct CommonArgs {
    /// FO4Edit executable.
    #[arg(long, default_value = r"D:\programs\xEdit 4.1\FO4Edit64.exe")]
    xedit: PathBuf,
    /// Fallout 4 `Data` folder; read from the registry when omitted.
    #[arg(long)]
    data: Option<PathBuf>,
    /// Seconds to wait after Module Selection appears, so you can set the focus.
    #[arg(long, default_value_t = 8)]
    countdown: u64,
    /// Seconds between the log appearing and the first close request (batch line 552: 10).
    #[arg(long, default_value_t = 10)]
    close_delay: u64,
    /// Leave the stub plugin and xEdit's save/backup files in place for inspection.
    #[arg(long)]
    keep: bool,
}

#[derive(Args, Clone)]
struct RunArgs {
    /// Dismissal rung to try.
    #[arg(long, value_enum)]
    method: Method,
    /// Where you put the focus during the countdown.
    #[arg(long, value_enum)]
    focus: Focus,
    /// Which of FO4Edit's top-level windows get `WM_CLOSE`.
    #[arg(long, value_enum, default_value = "all")]
    close: CloseMode,
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Method {
    /// Rung A: post `BM_CLICK` to the OK button.
    BmClick,
    /// Rung A variant: post `WM_COMMAND(BN_CLICKED)` for OK to the dialog.
    WmCommand,
    /// Rung B: post `WM_KEYDOWN`/`WM_KEYUP` ENTER to the dialog's focused control.
    PostEnter,
    /// Rung C: `SetForegroundWindow` on the dialog, then `SendInput` ENTER (only if it became
    /// the foreground window).
    SendInput,
    /// Rung D baseline: do nothing; you press OK.
    Manual,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Focus {
    /// Click back into this console window during the countdown.
    Console,
    /// Click some other window (not FO4Edit, not this console) during the countdown.
    Away,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CloseMode {
    /// `WM_CLOSE` to every top-level window of FO4Edit's PID (the FO4Edit design's close).
    All,
    /// `WM_CLOSE` to every visible top-level window of the PID.
    Visible,
    /// .NET `CloseMainWindow()` emulation: first visible unowned window, skipped if disabled.
    Main,
}

/// The fixed matrix: every rung with the console both focused and not, plus each close mode.
const MATRIX: &[(Method, Focus, CloseMode)] = &[
    (Method::PostEnter, Focus::Away, CloseMode::All),
    (Method::BmClick, Focus::Away, CloseMode::All),
    (Method::WmCommand, Focus::Away, CloseMode::Visible),
    (Method::SendInput, Focus::Console, CloseMode::Main),
    (Method::SendInput, Focus::Away, CloseMode::All),
    (Method::PostEnter, Focus::Console, CloseMode::All),
    (Method::BmClick, Focus::Console, CloseMode::All),
];

fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd {
        Cmd::Dump { pid } => {
            println!("{}", serde_json::to_string_pretty(&dump(pid)).unwrap_or_default());
            0
        }
        Cmd::Run(args) => match run_and_save(&args) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("probe failed: {e}");
                1
            }
        },
        Cmd::Matrix(common) => run_matrix(&common),
    };
    std::process::exit(code);
}

/// Runs every matrix scenario, asking for ENTER before each so the human is ready.
fn run_matrix(common: &CommonArgs) -> i32 {
    let mut failures = 0;
    for (i, &(method, focus, close)) in MATRIX.iter().enumerate() {
        println!();
        println!("=== Scenario {}/{}: method={method:?} focus={focus:?} close={close:?} ===", i + 1, MATRIX.len());
        println!("{}", focus_instruction(focus, common.countdown));
        print!("Press ENTER to start (s = skip, q = quit): ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        let _ = io::stdin().lock().read_line(&mut line);
        match line.trim() {
            "q" => break,
            "s" => continue,
            _ => {}
        }
        let args = RunArgs { method, focus, close, common: common.clone() };
        if let Err(e) = run_and_save(&args) {
            eprintln!("scenario failed: {e}");
            failures += 1;
        }
    }
    i32::from(failures > 0)
}

/// Runs one scenario and writes its report, even when the scenario itself failed midway.
fn run_and_save(args: &RunArgs) -> Result<(), String> {
    let mut probe = Probe::new();
    let result = probe.run(args);
    if let Err(e) = &result {
        probe.note(format!("SCENARIO ERROR: {e}"));
        probe.set("error", json!(e));
    }
    let path = probe.save(args)?;
    println!("\nReport written to {}", path.display());
    result
}

fn focus_instruction(focus: Focus, countdown: u64) -> String {
    match focus {
        Focus::Console => format!(
            "FOCUS: when Module Selection appears you have {countdown}s to click back into THIS \
             console window. Leave it focused and don't type."
        ),
        Focus::Away => format!(
            "FOCUS: when Module Selection appears you have {countdown}s to click some OTHER \
             window (File Explorer, the desktop) — NOT FO4Edit and NOT this console."
        ),
    }
}

/// Resolved file locations for one scenario.
struct Paths {
    xedit: PathBuf,
    data: PathBuf,
    plugin: PathBuf,
    plugins_txt: PathBuf,
    log: PathBuf,
    script: PathBuf,
}

impl Paths {
    fn resolve(common: &CommonArgs) -> Result<Self, String> {
        let data = match &common.data {
            Some(d) => d.clone(),
            None => registry_data_dir()?,
        };
        let temp = std::env::temp_dir();
        Ok(Self {
            xedit: common.xedit.clone(),
            plugin: data.join(PLUGIN_NAME),
            data,
            plugins_txt: temp.join("GPProbePlugins.txt"),
            // Kept short on purpose: the PJM scripts copy at most 60 characters of the -log: path.
            log: temp.join("GPProbe.log"),
            script: temp.join("GPProbe_WindowProbe.pas"),
        })
    }
}

/// `<Installed Path>\Data` from the Fallout 4 registry key.
fn registry_data_dir() -> Result<PathBuf, String> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\WOW6432Node\Bethesda Softworks\Fallout4")
        .map_err(|e| format!("Fallout 4 registry key: {e}; pass --data"))?;
    let installed: String =
        key.get_value("Installed Path").map_err(|e| format!("Installed Path: {e}; pass --data"))?;
    Ok(PathBuf::from(installed).join("Data"))
}

/// A minimal Fallout 4 plugin: a `TES4` header only, mastered on `Fallout4.esm`. It stands in
/// for the real `-Mod:` plugin, which already exists on disk in Steps 2 and 7. That matters:
/// xEdit saves an existing plugin as `<name>.save.<timestamp>` and renames it at shutdown.
fn stub_plugin() -> Vec<u8> {
    fn sub(out: &mut Vec<u8>, sig: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(sig);
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
    }
    let mut hedr = Vec::new();
    hedr.extend_from_slice(&1.0f32.to_le_bytes()); // version
    hedr.extend_from_slice(&0i32.to_le_bytes()); // record count
    hedr.extend_from_slice(&0x800u32.to_le_bytes()); // next object ID
    let mut data = Vec::new();
    sub(&mut data, b"HEDR", &hedr);
    sub(&mut data, b"CNAM", b"GPProbe\0");
    sub(&mut data, b"MAST", b"Fallout4.esm\0");
    sub(&mut data, b"DATA", &0u64.to_le_bytes());
    let mut rec = b"TES4".to_vec();
    rec.extend_from_slice(&(data.len() as u32).to_le_bytes());
    rec.extend_from_slice(&0u32.to_le_bytes()); // flags
    rec.extend_from_slice(&0u32.to_le_bytes()); // form ID
    rec.extend_from_slice(&0u32.to_le_bytes()); // version control info
    rec.extend_from_slice(&131u16.to_le_bytes()); // form version
    rec.extend_from_slice(&0u16.to_le_bytes()); // unknown
    rec.extend_from_slice(&data);
    rec
}

/// Files in `dir` whose names start with `prefix` (case-insensitive).
fn files_with_prefix(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let prefix = prefix.to_ascii_lowercase();
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.to_ascii_lowercase().starts_with(&prefix))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every probe-owned file: the stub plugin, xEdit's `.save.*` files and backups of it.
fn probe_files(paths: &Paths) -> Vec<PathBuf> {
    let mut files = files_with_prefix(&paths.data, PLUGIN_NAME);
    files.extend(files_with_prefix(&paths.data.join("FO4Edit Backups"), PLUGIN_NAME));
    files
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn names(windows: &[WindowInfo]) -> Vec<String> {
    windows.iter().map(|w| format!("{} [{}] {:?}", w.hwnd, w.class, w.title)).collect()
}

/// Window snapshot of a PID plus the foreground window — the `dump` subcommand.
fn dump(pid: u32) -> Value {
    let windows = win::top_level_of(pid);
    let children: Vec<Value> = windows
        .iter()
        .filter(|w| w.visible)
        .map(|w| json!({ "parent": w.hwnd, "children": win::children(w.raw) }))
        .collect();
    let threads: Vec<u32> = {
        let mut t: Vec<u32> = windows.iter().map(|w| w.thread_id).collect();
        t.sort_unstable();
        t.dedup();
        t
    };
    json!({
        "top_level": windows,
        "visible_children": children,
        "gui_threads": threads.iter().map(|&t| json!({ "thread": t, "info": win::gui_thread_info(t) })).collect::<Vec<_>>(),
        "dotnet_main_window": win::dotnet_main_window(pid),
        "foreground": win::foreground(),
    })
}

/// One scenario's state: its clock, timeline and report fields.
struct Probe {
    start: Instant,
    timeline: Vec<String>,
    report: serde_json::Map<String, Value>,
}

impl Probe {
    fn new() -> Self {
        Self { start: Instant::now(), timeline: Vec::new(), report: serde_json::Map::new() }
    }

    fn secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Prints a line and records it in the timeline with its offset from scenario start.
    fn note(&mut self, msg: impl AsRef<str>) {
        let line = format!("[+{:6.1}s] {}", self.secs(), msg.as_ref());
        println!("{line}");
        self.timeline.push(line);
    }

    fn set(&mut self, key: &str, value: Value) {
        self.report.insert(key.to_owned(), value);
    }

    /// Writes the report as `reports/<unix>-<method>-<focus>-<close>.json` in the probe crate.
    fn save(&mut self, args: &RunArgs) -> Result<PathBuf, String> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("reports");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let name = format!(
            "{stamp}-{:?}-{:?}-{:?}.json",
            args.method, args.focus, args.close
        )
        .to_ascii_lowercase();
        let path = dir.join(name);
        self.set("timeline", json!(self.timeline));
        let text = serde_json::to_string_pretty(&self.report).map_err(|e| e.to_string())?;
        fs::write(&path, text).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Whether the child has exited; records the exit the first time it is seen.
    fn exited(&mut self, child: &mut Child) -> bool {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !self.report.contains_key("exit") {
                    let at = self.secs();
                    self.set("exit", json!({ "at_s": at, "code": status.code() }));
                    self.note(format!("FO4Edit exited with {:?}", status.code()));
                }
                true
            }
            _ => false,
        }
    }

    fn run(&mut self, args: &RunArgs) -> Result<(), String> {
        let common = &args.common;
        let paths = Paths::resolve(common)?;
        self.set(
            "scenario",
            json!({ "method": format!("{:?}", args.method), "focus": format!("{:?}", args.focus),
                    "close": format!("{:?}", args.close), "countdown_s": common.countdown,
                    "close_delay_s": common.close_delay }),
        );
        if !paths.xedit.is_file() {
            return Err(format!("no FO4Edit at {}", paths.xedit.display()));
        }
        if !paths.data.join("Fallout4.esm").is_file() {
            return Err(format!("no Fallout4.esm in {}", paths.data.display()));
        }

        self.prepare(&paths)?;
        self.record_environment();

        let cmdline = [
            "-fo4".to_owned(),
            "-autoexit".to_owned(),
            format!("-P:{}", paths.plugins_txt.display()),
            format!("-Script:{}", paths.script.display()),
            format!("-Mod:{PLUGIN_NAME}"),
            format!("-log:{}", paths.log.display()),
        ];
        self.set("xedit", json!({ "exe": paths.xedit, "args": cmdline }));
        let mut child =
            Command::new(&paths.xedit).args(&cmdline).spawn().map_err(|e| format!("spawn: {e}"))?;
        let pid = child.id();
        self.set("xedit_pid", json!(pid));
        self.note(format!("launched FO4Edit, PID {pid}"));

        let result = self.drive(args, &paths, &mut child, pid);
        self.verify_and_clean(&paths, common.keep);
        result
    }

    /// Clears leftovers from earlier runs, then writes the stub plugin, plugins file and script.
    fn prepare(&mut self, paths: &Paths) -> Result<(), String> {
        for leftover in probe_files(paths) {
            fs::remove_file(&leftover).map_err(|e| format!("remove {}: {e}", leftover.display()))?;
            self.note(format!("removed leftover {}", leftover.display()));
        }
        let _ = fs::remove_file(&paths.log);
        let stub = stub_plugin();
        fs::write(&paths.plugin, &stub).map_err(|e| format!("write stub plugin: {e}"))?;
        fs::write(&paths.plugins_txt, format!("*{PLUGIN_NAME}\r\n")).map_err(|e| e.to_string())?;
        fs::write(&paths.script, PROBE_SCRIPT).map_err(|e| e.to_string())?;
        self.set("stub_plugin", json!({ "path": paths.plugin, "bytes": stub.len() }));
        Ok(())
    }

    /// Records who this probe is and who holds the foreground — rung C's preconditions.
    fn record_environment(&mut self) {
        let procs = win::processes();
        let mut ancestors = Vec::new();
        let mut pid = std::process::id();
        for _ in 0..12 {
            let Some(p) = procs.get(&pid) else { break };
            ancestors.push(p.clone());
            if p.parent_pid == 0 || p.parent_pid == pid {
                break;
            }
            pid = p.parent_pid;
        }
        let fg = win::foreground();
        let fg_exe = fg.as_ref().and_then(|w| procs.get(&w.pid)).map(|p| p.exe.clone());
        let fg_is_ancestor = fg.as_ref().is_some_and(|w| ancestors.iter().any(|a| a.pid == w.pid));
        let console = win::console_window();
        let console_exe = console.as_ref().and_then(|w| procs.get(&w.pid)).map(|p| p.exe.clone());
        let other_xedits: Vec<&win::ProcInfo> =
            procs.values().filter(|p| p.exe.to_ascii_lowercase().contains("edit")).collect();
        self.set(
            "environment",
            json!({
                "probe_pid": std::process::id(),
                "ancestors": ancestors,
                "console_window": console,
                "console_window_exe": console_exe,
                "foreground_at_start": fg,
                "foreground_exe": fg_exe,
                "foreground_is_ancestor": fg_is_ancestor,
                "foreground_lock_timeout_ms": win::foreground_lock_timeout_ms(),
                "other_edit_processes": other_xedits,
            }),
        );
    }

    /// Everything between launch and FO4Edit's exit.
    fn drive(&mut self, args: &RunArgs, paths: &Paths, child: &mut Child, pid: u32) -> Result<(), String> {
        let dialog = self.wait_for_module_selection(child, pid)?;
        self.set("at_appearance", self.snapshot(pid, Some(&dialog)));

        println!("\n{}", focus_instruction(args.focus, args.common.countdown));
        for left in (1..=args.common.countdown).rev() {
            print!("\r  {left:2}s… ");
            let _ = io::stdout().flush();
            sleep(Duration::from_secs(1));
        }
        println!();

        let at_attempt = self.snapshot(pid, Some(&dialog));
        let dialog_is_fg = win::foreground().is_some_and(|w| w.raw == dialog.raw);
        self.note(format!("attempting {:?}; Module Selection is foreground: {dialog_is_fg}", args.method));
        self.set("at_attempt", at_attempt);
        self.set("dialog_was_foreground_at_attempt", json!(dialog_is_fg));
        let attempt = self.attempt(args.method, &dialog);
        self.set("attempt", attempt);

        self.observe_dismissal(args.method, &dialog, child)?;
        self.wait_for_log(paths, child)?;

        self.note(format!("waiting {}s before closing (batch line 552)", args.common.close_delay));
        sleep(Duration::from_secs(args.common.close_delay));
        if self.exited(child) {
            self.note("FO4Edit already exited on its own; no close needed");
            return Ok(());
        }
        let at_close = self.snapshot(pid, None);
        self.set("at_close", at_close);
        self.close_sequence(args.close, child, pid);
        Ok(())
    }

    /// Polls for a visible top-level window of `pid` captioned exactly "Module Selection",
    /// noting every new window of the PID on the way.
    fn wait_for_module_selection(&mut self, child: &mut Child, pid: u32) -> Result<WindowInfo, String> {
        let mut seen: Vec<Hwnd> = Vec::new();
        let mut hinted = false;
        let mut first_windows = Vec::new();
        loop {
            if self.exited(child) {
                return Err("FO4Edit exited before Module Selection appeared".into());
            }
            for w in win::top_level_of(pid) {
                if !seen.contains(&w.raw) {
                    seen.push(w.raw);
                    self.note(format!(
                        "new window {} [{}] {:?} visible={} owner={:?}",
                        w.hwnd, w.class, w.title, w.visible, w.owner
                    ));
                    first_windows.push(w.clone());
                }
                if w.visible && w.title == MODULE_SELECTION {
                    self.note("Module Selection is up");
                    self.set("windows_seen_before_dialog", json!(first_windows));
                    // Give FormShow (SimulateLoad, tree focus) a moment, as the batch's 5s+1s did.
                    sleep(Duration::from_millis(1500));
                    return Ok(win::info(w.raw));
                }
            }
            if !hinted && self.secs() > 30.0 {
                hinted = true;
                self.note("HINT: no Module Selection after 30s. If FO4Edit shows another dialog \
                           (developer message / What's New), dismiss it by hand.");
            }
            if self.secs() > 600.0 {
                return Err("no Module Selection within 10 min".into());
            }
            sleep(Duration::from_millis(250));
        }
    }

    /// Windows of the PID, the dialog's children and GUI-thread state, foreground, and the
    /// window .NET's `CloseMainWindow()` would pick.
    fn snapshot(&self, pid: u32, dialog: Option<&WindowInfo>) -> Value {
        let windows = win::top_level_of(pid);
        let (children, gti) = match dialog {
            Some(d) => (json!(win::children(d.raw)), json!(win::gui_thread_info(d.thread_id))),
            None => {
                let main_thread = windows.iter().find(|w| w.visible).map(|w| w.thread_id);
                (Value::Null, json!(main_thread.map(win::gui_thread_info)))
            }
        };
        let main = win::dotnet_main_window(pid);
        let close_main_window_would = match &main {
            None => "return false (no main window)",
            Some(m) if m.disabled => "return false without posting (main window disabled)",
            Some(_) => "post WM_CLOSE and return true",
        };
        json!({
            "at_s": self.secs(),
            "top_level": windows,
            "dialog_children": children,
            "gui_thread": gti,
            "foreground": win::foreground(),
            "dotnet_main_window": main,
            "close_main_window_would": close_main_window_would,
        })
    }

    /// Tries one dismissal rung and returns the raw API results.
    fn attempt(&mut self, method: Method, dialog: &WindowInfo) -> Value {
        let children = win::children(dialog.raw);
        let ok_button = children
            .iter()
            .find(|c| c.title.replace('&', "") == "OK" && c.class.to_ascii_lowercase().contains("button"))
            .cloned();
        match method {
            Method::BmClick => match &ok_button {
                Some(b) => {
                    let r = win::post(b.raw, BM_CLICK, 0, 0);
                    self.note(format!("posted BM_CLICK to {} -> {r:?}", b.hwnd));
                    json!({ "target": b, "post": format!("{r:?}") })
                }
                None => json!({ "error": "no OK button child found", "children": children }),
            },
            Method::WmCommand => match &ok_button {
                Some(b) => {
                    let wparam = (b.ctrl_id as u16 as usize) | (BN_CLICKED << 16);
                    let r = win::post(dialog.raw, WM_COMMAND, wparam, b.raw);
                    self.note(format!("posted WM_COMMAND(BN_CLICKED, id {}) to dialog -> {r:?}", b.ctrl_id));
                    json!({ "target": dialog, "button": b, "wparam": format!("{wparam:#x}"), "post": format!("{r:?}") })
                }
                None => json!({ "error": "no OK button child found", "children": children }),
            },
            Method::PostEnter => {
                // The focused control per the dialog thread; FormShow focuses the module tree.
                let gti = win::gui_thread_info(dialog.thread_id);
                let target = gti
                    .focus
                    .clone()
                    .filter(|f| f.pid == dialog.pid)
                    .or_else(|| children.iter().find(|c| c.class.contains("VirtualStringTree")).cloned())
                    .unwrap_or_else(|| dialog.clone());
                let down = win::post(target.raw, WM_KEYDOWN, VK_RETURN, ENTER_DOWN_LPARAM);
                let up = win::post(target.raw, WM_KEYUP, VK_RETURN, ENTER_UP_LPARAM);
                self.note(format!("posted ENTER down/up to {} [{}] -> {down:?}/{up:?}", target.hwnd, target.class));
                json!({ "target": target, "gui_thread": gti, "keydown": format!("{down:?}"), "keyup": format!("{up:?}") })
            }
            Method::SendInput => {
                let before = win::foreground();
                let already = before.as_ref().is_some_and(|w| w.raw == dialog.raw);
                let set_fg = (!already).then(|| win::set_foreground(dialog.raw));
                // Activation of another thread group's window is asynchronous; give it a moment.
                sleep(Duration::from_millis(400));
                let after = win::foreground();
                let is_fg = after.as_ref().is_some_and(|w| w.raw == dialog.raw);
                // The design forbids sending input to any window but Module Selection.
                let sent = is_fg.then(win::send_input_enter);
                self.note(format!(
                    "SetForegroundWindow -> {set_fg:?}; foreground is dialog: {is_fg}; SendInput -> {sent:?}"
                ));
                json!({
                    "foreground_before": before, "already_foreground": already,
                    "set_foreground_window": set_fg, "foreground_after": after,
                    "dialog_is_foreground": is_fg, "send_input_events": sent,
                    "skipped_input": !is_fg,
                })
            }
            Method::Manual => {
                self.note("manual: press OK in Module Selection now");
                json!({ "manual": true })
            }
        }
    }

    /// Waits up to 15s for the dialog to go away by itself; then asks the human (rung D).
    fn observe_dismissal(&mut self, method: Method, dialog: &WindowInfo, child: &mut Child) -> Result<(), String> {
        let started = Instant::now();
        let gone = |h: Hwnd| !win::exists(h) || !win::info(h).visible;
        let limit = if matches!(method, Method::Manual) { 600 } else { 15 };
        while started.elapsed() < Duration::from_secs(limit) {
            if gone(dialog.raw) {
                let ms = started.elapsed().as_millis();
                self.note(format!("Module Selection dismissed {ms} ms after the attempt"));
                self.set("dismissed_by_method", json!({ "dismissed": true, "latency_ms": ms }));
                return Ok(());
            }
            if self.exited(child) {
                return Err("FO4Edit exited while Module Selection was up".into());
            }
            sleep(Duration::from_millis(100));
        }
        self.set("dismissed_by_method", json!({ "dismissed": false }));
        self.note(">>> NOT dismissed by the method. Press OK in FO4Edit's Module Selection by hand now.");
        let manual = Instant::now();
        while !gone(dialog.raw) {
            if self.exited(child) {
                return Err("FO4Edit exited while waiting for a manual OK".into());
            }
            if manual.elapsed() > Duration::from_secs(600) {
                return Err("Module Selection still up after 10 min".into());
            }
            sleep(Duration::from_millis(250));
        }
        self.note("Module Selection dismissed by hand");
        self.set("dismissed_by_hand", json!(true));
        Ok(())
    }

    /// Polls for the unattended log; records when it appears and its contents.
    fn wait_for_log(&mut self, paths: &Paths, child: &mut Child) -> Result<(), String> {
        loop {
            if paths.log.is_file() {
                // The PJM-style script writes the log in one SaveToFile; let it finish.
                sleep(Duration::from_millis(300));
                let text = fs::read_to_string(&paths.log).unwrap_or_default();
                let at = self.secs();
                self.note(format!("log appeared: {:?}", text.lines().last().unwrap_or("")));
                let plugin_now = fs::read(&paths.plugin).unwrap_or_default();
                self.set(
                    "log",
                    json!({ "at_s": at, "text": text,
                            "plugin_has_marker_when_log_appeared": contains(&plugin_now, MARKER),
                            "save_files_when_log_appeared": files_with_prefix(&paths.data, &format!("{PLUGIN_NAME}.save")) }),
                );
                return Ok(());
            }
            if self.exited(child) {
                return Err("FO4Edit exited before writing the log".into());
            }
            if self.secs() > 1800.0 {
                return Err("no log within 30 min".into());
            }
            sleep(Duration::from_millis(500));
        }
    }

    /// The FO4Edit design's close: request, wait 15s, request again, wait 10s; never kill.
    /// If FO4Edit is still alive after that, the human closes it so the save can be checked.
    fn close_sequence(&mut self, mode: CloseMode, child: &mut Child, pid: u32) {
        let mut rounds = Vec::new();
        for (round, wait) in [(1, 15u64), (2, 10u64)] {
            let targets: Vec<WindowInfo> = match mode {
                CloseMode::All => win::top_level_of(pid),
                CloseMode::Visible => win::top_level_of(pid).into_iter().filter(|w| w.visible).collect(),
                CloseMode::Main => win::dotnet_main_window(pid).into_iter().filter(|w| !w.disabled).collect(),
            };
            let results: Vec<Value> = targets
                .iter()
                .map(|w| json!({ "window": format!("{} [{}] {:?}", w.hwnd, w.class, w.title),
                                 "post": format!("{:?}", win::post(w.raw, WM_CLOSE, 0, 0)) }))
                .collect();
            self.note(format!("close round {round}: WM_CLOSE to {} window(s): {:?}", targets.len(), names(&targets)));
            let posted_at = Instant::now();
            let mut exited_after = None;
            while posted_at.elapsed() < Duration::from_secs(wait) {
                if self.exited(child) {
                    exited_after = Some(posted_at.elapsed().as_millis());
                    break;
                }
                sleep(Duration::from_millis(100));
            }
            let after = (exited_after.is_none()).then(|| self.snapshot(pid, None));
            rounds.push(json!({ "round": round, "targets": results, "exited_after_ms": exited_after,
                                "windows_after_wait": after }));
            if exited_after.is_some() {
                break;
            }
        }
        self.set("close_rounds", json!(rounds));
        if self.exited(child) {
            return;
        }
        self.note(">>> FO4Edit is still running after the close sequence. Close it by hand now \
                   (the probe never kills it).");
        self.set("closed_by_hand", json!(true));
        let started = Instant::now();
        while !self.exited(child) && started.elapsed() < Duration::from_secs(600) {
            sleep(Duration::from_millis(250));
        }
    }

    /// Checks whether the marker reached the real plugin, lists xEdit's save/backup files,
    /// then removes every probe-owned file unless `keep`.
    fn verify_and_clean(&mut self, paths: &Paths, keep: bool) {
        let plugin = fs::read(&paths.plugin).unwrap_or_default();
        let saves = files_with_prefix(&paths.data, &format!("{PLUGIN_NAME}.save"));
        let backups = files_with_prefix(&paths.data.join("FO4Edit Backups"), PLUGIN_NAME);
        let saved = contains(&plugin, MARKER);
        self.note(format!(
            "plugin saved with marker: {saved} ({} bytes); leftover .save files: {}; backups: {}",
            plugin.len(),
            saves.len(),
            backups.len()
        ));
        self.set(
            "save",
            json!({ "plugin_bytes_after": plugin.len(), "plugin_has_marker": saved,
                    "leftover_save_files": saves, "backups": backups }),
        );
        let mut removed = Vec::new();
        if !keep {
            let mut extra = vec![paths.plugins_txt.clone(), paths.log.clone(), paths.script.clone()];
            extra.extend(probe_files(paths));
            for f in extra {
                if f.exists() && fs::remove_file(&f).is_ok() {
                    removed.push(f);
                }
            }
        }
        self.set("cleanup", json!({ "kept": keep, "removed": removed }));
    }
}
