//! Throwaway probe for the map ticket "Probe archive rebuild round trips and the work-folder
//! swap on Windows" (#44).
//!
//! It plays the Archive design (#29, ADR-0004) against the real tools, using synthetic
//! precombines and previs, for every pairing of a Step 3 tool with a Step 8 tool:
//!
//! - **Step 3** packs `Data\meshes\precombined` into `<fo4>\ArchiveWork\<archive>`. Archive2
//!   runs with cwd `Data` and a relative source. BSArch packs a staging tree that was moved into
//!   the work folder. The step checks the archive, removes the loose files and swaps the archive
//!   into `Data`.
//! - **Step 8** rebuilds the archive as its own contents plus `vis`. Archive2 does extract → 5s →
//!   repack; BSArch does `unpack` into the work folder, moves `vis` in, waits 5s and packs. The
//!   step checks the new archive, then deletes the archive in `Data` and moves the new one in.
//!
//! The probe reads every archive back with its own BA2 reader, so the path and byte checks
//! don't trust either tool. Moves use `std::fs::rename`, the same call as the port's
//! `FileSpace::rename`. A JSON report goes under `reports/`.
//!
//! Safety: the probe refuses to start unless `Data\meshes\precombined` and `Data\vis` hold no
//! files and neither `<fo4>\ArchiveWork` nor `Data\GPProbe - Main.ba2` exists. It deletes only
//! what it created.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read as _;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Parser, ValueEnum};
use flate2::read::ZlibDecoder;
use serde_json::{Value, json};

const ARCHIVE_NAME: &str = "GPProbe - Main.ba2";
const WORK_FOLDER: &str = "ArchiveWork";
const PRECOMBINED: &str = r"meshes\precombined";
const VIS: &str = "vis";
const MESH_COUNT: u32 = 40;
const VIS_COUNT: u32 = 20;
/// The design's MO2 settle wait: after the Archive2 extract, and before every BSArch pack.
const SETTLE: Duration = Duration::from_secs(5);

/// Internal archive path (`meshes\precombined\….nif`, `vis\….uvd`) → file bytes.
type Fixture = BTreeMap<String, Vec<u8>>;

#[derive(Parser)]
#[command(about = "Probe archive rebuild round trips and the ArchiveWork → Data swap (map ticket #44)")]
struct Cli {
    /// Fallout 4 install folder (the one holding `Data`); read from the registry when omitted.
    #[arg(long)]
    fo4: Option<PathBuf>,
    /// Archive2.exe; defaults to `<fo4>\Tools\Archive2\Archive2.exe`.
    #[arg(long)]
    archive2: Option<PathBuf>,
    /// BSArch 1.0 (xEdit 4.1.5q).
    #[arg(long, default_value = r"D:\programs\xEdit 4.1\BSArch.exe")]
    bsarch10: PathBuf,
    /// BSArch 0.9x (xEdit 4.1.5p or earlier).
    #[arg(long, default_value = r"D:\programs\xEdit 4.1.5f\BSArch.exe")]
    bsarch09: PathBuf,
    /// MO2 instance folder. When the probe runs inside MO2, its `overwrite` and `mods` folders
    /// are searched to see where files physically landed.
    #[arg(long, default_value = r"E:\Mod Organizer\Fallout 4")]
    mo2_instance: PathBuf,
    /// Step 3 tools to try (comma-separated). Defaults to every tool found.
    #[arg(long, value_enum, value_delimiter = ',')]
    step3: Vec<Tool>,
    /// Step 8 tools to try (comma-separated). Defaults to every tool found.
    #[arg(long, value_enum, value_delimiter = ',')]
    step8: Vec<Tool>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Archive2,
    Bsarch10,
    Bsarch09,
}

impl Tool {
    const ALL: [Tool; 3] = [Tool::Archive2, Tool::Bsarch10, Tool::Bsarch09];

    fn label(self) -> &'static str {
        match self {
            Tool::Archive2 => "archive2",
            Tool::Bsarch10 => "bsarch-1.0",
            Tool::Bsarch09 => "bsarch-0.9x",
        }
    }
}

/// Resolved paths and the fixtures every pair packs.
struct Ctx {
    data: PathBuf,
    work: PathBuf,
    archive2: PathBuf,
    bsarch10: PathBuf,
    bsarch09: PathBuf,
    /// `Some` only when usvfs is loaded into this process, i.e. the probe runs inside MO2.
    mo2: Option<PathBuf>,
    meshes: Fixture,
    vis: Fixture,
    /// Folders the fixtures write into, with whether each existed before the probe ran, so
    /// cleanup removes only the ones the probe created (and recreates ones the steps removed).
    data_dirs: Vec<(PathBuf, bool)>,
    /// Normalized internal path → (lookup key, which archive first had it), across every pair.
    /// `RefCell` because the checks run behind `&Ctx`.
    lookup_keys: RefCell<BTreeMap<String, (String, String)>>,
}

impl Ctx {
    fn exe(&self, tool: Tool) -> &Path {
        match tool {
            Tool::Archive2 => &self.archive2,
            Tool::Bsarch10 => &self.bsarch10,
            Tool::Bsarch09 => &self.bsarch09,
        }
    }

    fn work_archive(&self) -> PathBuf {
        self.work.join(ARCHIVE_NAME)
    }

    fn data_archive(&self) -> PathBuf {
        self.data.join(ARCHIVE_NAME)
    }

    /// Precombines plus previs: what a finished Step 8 archive must hold.
    fn both(&self) -> Fixture {
        self.meshes.iter().chain(&self.vis).map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

/// One Step 3 tool × Step 8 tool run: printed as it goes, and kept for the JSON report.
struct Pair {
    step3: Tool,
    step8: Tool,
    events: Vec<Value>,
    checks: Vec<Value>,
    fatal: Option<String>,
}

impl Pair {
    fn new(step3: Tool, step8: Tool) -> Self {
        Self { step3, step8, events: Vec::new(), checks: Vec::new(), fatal: None }
    }

    fn event(&mut self, what: &str, detail: Value) {
        println!("  · {what}");
        self.events.push(json!({ "what": what, "detail": detail }));
    }

    /// Records a check against one of the ticket's items (`item1`/`item2`/`item3`) or `extract`.
    fn check(&mut self, item: &str, what: &str, ok: bool, detail: Value) -> bool {
        println!("  [{}] {item}: {what}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            println!("         {detail}");
        }
        self.checks.push(json!({ "item": item, "what": what, "ok": ok, "detail": detail }));
        ok
    }

    /// `PASS`/`FAIL` for an item, or `n/a` when this pair never exercised it.
    fn verdict(&self, item: &str) -> &'static str {
        let mine: Vec<_> = self.checks.iter().filter(|c| c["item"] == item).collect();
        if mine.is_empty() {
            "n/a"
        } else if mine.iter().all(|c| c["ok"] == true) {
            "PASS"
        } else {
            "FAIL"
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "step3": self.step3.label(),
            "step8": self.step8.label(),
            "verdicts": {
                "item1_bsarch_round_trip": self.verdict("item1"),
                "item2_archive2_outside_data": self.verdict("item2"),
                "item3_swap_into_data": self.verdict("item3"),
                "archive2_extract": self.verdict("extract"),
            },
            "fatal": self.fatal,
            "checks": self.checks,
            "events": self.events,
        })
    }
}

/// A command-line argument, either quoted by Rust's usual rules or passed through verbatim.
enum Arg {
    Plain(String),
    /// Passed with `raw_arg`, for reproducing the batch's `-c="…"` quoting exactly.
    Raw(String),
}

struct RunOut {
    exit: Option<i32>,
}

fn main() {
    let code = match real_main(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("probe failed: {e}");
            2
        }
    };
    // MO2 starts the probe in a console of its own, which would vanish with the summary.
    if usvfs_loaded() {
        print!("\nPress ENTER to close.");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(code);
}

fn real_main(cli: Cli) -> Result<i32, String> {
    let fo4 = match cli.fo4 {
        Some(p) => p,
        None => registry_fo4()?,
    };
    let data = fo4.join("Data");
    let archive2 = cli.archive2.unwrap_or_else(|| fo4.join(r"Tools\Archive2\Archive2.exe"));
    let in_mo2 = usvfs_loaded();
    let (meshes, vis) = fixtures();
    let data_dirs = [data.join("meshes"), data.join(PRECOMBINED), data.join(VIS)]
        .into_iter()
        .map(|d| {
            let existed = d.is_dir();
            (d, existed)
        })
        .collect();
    let ctx = Ctx {
        work: fo4.join(WORK_FOLDER),
        data,
        archive2,
        bsarch10: cli.bsarch10,
        bsarch09: cli.bsarch09,
        mo2: in_mo2.then(|| cli.mo2_instance.clone()),
        meshes,
        vis,
        data_dirs,
        lookup_keys: RefCell::new(BTreeMap::new()),
    };

    println!("Context: {}", if in_mo2 { "INSIDE MO2 (usvfs loaded)" } else { "outside MO2" });
    println!("Data:    {}", ctx.data.display());
    println!("Work:    {}", ctx.work.display());
    if let Some(mo2) = &ctx.mo2 {
        if !mo2.is_dir() {
            return Err(format!(
                "running inside MO2, but the instance folder {} doesn't exist; pass --mo2-instance",
                mo2.display()
            ));
        }
    }

    let mut tools = Vec::new();
    for tool in Tool::ALL {
        let exe = ctx.exe(tool);
        let banner = if exe.is_file() { Some(tool_banner(tool, exe)) } else { None };
        println!("{:<12} {} {}", tool.label(), exe.display(), banner.as_deref().unwrap_or("(MISSING — pairs using it are skipped)"));
        tools.push(json!({ "tool": tool.label(), "path": exe, "found": banner.is_some(), "banner": banner }));
    }
    let found = |t: Tool| ctx.exe(t).is_file();
    let step3: Vec<Tool> = if cli.step3.is_empty() { Tool::ALL.to_vec() } else { cli.step3 };
    let step8: Vec<Tool> = if cli.step8.is_empty() { Tool::ALL.to_vec() } else { cli.step8 };

    preflight(&ctx)?;

    let mut pairs = Vec::new();
    let mut skipped = Vec::new();
    for &s3 in &step3 {
        for &s8 in &step8 {
            if !found(s3) || !found(s8) {
                skipped.push(format!("{} → {}", s3.label(), s8.label()));
                continue;
            }
            println!("\n=== Step 3: {}  →  Step 8: {} ===", s3.label(), s8.label());
            let mut pair = Pair::new(s3, s8);
            if let Err(e) = run_pair(&mut pair, &ctx) {
                println!("  !! pair stopped: {e}");
                pair.fatal = Some(e);
            }
            cleanup(&mut pair, &ctx);
            pairs.push(pair);
            // A pair that couldn't clean up would poison the next one, so stop instead.
            if let Err(e) = preflight(&ctx) {
                println!("\nStopping: cleanup left the probe's state behind: {e}");
                break;
            }
        }
    }

    println!("\n=== Summary ({}) ===", if in_mo2 { "inside MO2" } else { "outside MO2" });
    println!("{:<26} {:<6} {:<6} {:<6} {:<8}", "step3 → step8", "item1", "item2", "item3", "extract");
    for p in &pairs {
        println!(
            "{:<26} {:<6} {:<6} {:<6} {:<8}{}",
            format!("{} → {}", p.step3.label(), p.step8.label()),
            p.verdict("item1"),
            p.verdict("item2"),
            p.verdict("item3"),
            p.verdict("extract"),
            if p.fatal.is_some() { "  (stopped)" } else { "" }
        );
    }
    for s in &skipped {
        println!("{s:<26} skipped (tool missing)");
    }

    let report = json!({
        "ticket": "https://github.com/evildarkarchon/generateprevisibines/issues/44",
        "context": if in_mo2 { "inside-mo2" } else { "outside-mo2" },
        "usvfs_loaded": in_mo2,
        "fo4": fo4,
        "data": ctx.data,
        "work": ctx.work,
        "mo2_instance": ctx.mo2,
        "tools": tools,
        "fixture": { "meshes": ctx.meshes.len(), "vis": ctx.vis.len() },
        "skipped": skipped,
        "pairs": pairs.iter().map(Pair::to_json).collect::<Vec<_>>(),
    });
    let path = save_report(&report, in_mo2)?;
    println!("\nReport written to {}", path.display());

    let all_ok = pairs.iter().all(|p| {
        p.fatal.is_none() && ["item1", "item2", "item3", "extract"].iter().all(|i| p.verdict(i) != "FAIL")
    });
    Ok(i32::from(!all_ok))
}

/// Refuses to touch an install where any of the probe's paths already hold something.
fn preflight(ctx: &Ctx) -> Result<(), String> {
    if !ctx.data.is_dir() {
        return Err(format!("{} is not a folder; pass --fo4", ctx.data.display()));
    }
    if ctx.work.exists() {
        return Err(format!(
            "{} already exists. Check it holds nothing you need, then remove it by hand",
            ctx.work.display()
        ));
    }
    if ctx.data_archive().exists() {
        return Err(format!("{} already exists; remove it by hand", ctx.data_archive().display()));
    }
    for rel in [PRECOMBINED, VIS] {
        let dir = ctx.data.join(rel);
        let n = count_files(&dir);
        if n > 0 {
            return Err(format!(
                "{} holds {n} file(s). The probe moves and deletes this folder, so it must be empty",
                dir.display()
            ));
        }
    }
    Ok(())
}

/// Runs both steps for one pair, then checks the archive left in `Data`.
fn run_pair(pair: &mut Pair, ctx: &Ctx) -> Result<(), String> {
    write_fixture(pair, &ctx.data, &ctx.meshes, "seed loose precombines in Data")?;
    pair.event("physical location after seeding precombines", physical(ctx));

    match pair.step3 {
        Tool::Archive2 => step3_archive2(pair, ctx)?,
        bsarch => step3_bsarch(pair, ctx, bsarch)?,
    }

    write_fixture(pair, &ctx.data, &ctx.vis, "seed loose previs in Data")?;
    pair.event("physical location after seeding previs", physical(ctx));

    match pair.step8 {
        Tool::Archive2 => step8_archive2(pair, ctx)?,
        bsarch => step8_bsarch(pair, ctx, bsarch)?,
    }

    verify_archive(pair, ctx, "item3", "final Data archive holds precombines + previs", &ctx.data_archive(), &ctx.both());
    let leftovers: Vec<_> = [PRECOMBINED, VIS].iter().filter(|r| count_files(&ctx.data.join(r)) > 0).collect();
    pair.check("item3", "no loose precombines or previs left in Data", leftovers.is_empty(), json!({ "leftover": leftovers }));
    Ok(())
}

/// Step 3, Archive2: pack the relative `meshes\precombined` into the work folder, check, RD,
/// swap.
fn step3_archive2(pair: &mut Pair, ctx: &Ctx) -> Result<(), String> {
    make_dir(pair, &ctx.work)?;
    archive2_create(pair, ctx, PRECOMBINED, "step 3 pack")?;
    verify_archive(pair, ctx, "item2", "step 3: Archive2 archive in ArchiveWork holds meshes\\precombined\\…", &ctx.work_archive(), &ctx.meshes);
    remove_tree(pair, &ctx.data.join(PRECOMBINED), "step 3: RD loose Data\\meshes\\precombined")?;
    swap(pair, ctx, "step 3")?;
    remove_empty_work(pair, ctx)
}

/// Step 3, BSArch: move the precombines into the work folder, wait, pack the work folder into
/// itself, check, drop the staged loose files, swap.
fn step3_bsarch(pair: &mut Pair, ctx: &Ctx, tool: Tool) -> Result<(), String> {
    make_dir(pair, &ctx.work.join("meshes"))?;
    move_dir(pair, ctx, &ctx.data.join(PRECOMBINED), &ctx.work.join(PRECOMBINED), "step 3: move Data\\meshes\\precombined into ArchiveWork")?;
    pair.event("wait 5s before BSArch pack", json!(null));
    sleep(SETTLE);
    bsarch_pack(pair, ctx, tool, "step 3 pack")?;
    verify_archive(pair, ctx, "item1", "step 3: BSArch archive packed from ArchiveWork holds meshes\\precombined\\…", &ctx.work_archive(), &ctx.meshes);
    remove_tree(pair, &ctx.work.join("meshes"), "step 3: remove staged loose precombines")?;
    swap(pair, ctx, "step 3")?;
    remove_empty_work(pair, ctx)
}

/// Step 8, Archive2: extract into `Data`, wait, repack `meshes\precombined,vis` into the work
/// folder, check, swap, RD the loose folders.
fn step8_archive2(pair: &mut Pair, ctx: &Ctx) -> Result<(), String> {
    require_data_archive(pair, ctx)?;
    let out = run_tool(
        pair,
        ctx,
        Tool::Archive2,
        "step 8: Archive2 extract into Data",
        vec![Arg::Plain(ARCHIVE_NAME.into()), Arg::Plain("-e=.".into()), Arg::Plain("-q".into())],
    );
    if out.exit != Some(0) {
        return Err(format!("Archive2 extract exited {:?}", out.exit));
    }
    pair.event("wait 5s after extract", json!(null));
    sleep(SETTLE);
    verify_folder(pair, "extract", "step 8: Archive2 extract put the precombines back in Data", &ctx.data, &ctx.meshes, &[PRECOMBINED]);
    make_dir(pair, &ctx.work)?;
    archive2_create(pair, ctx, &format!("{PRECOMBINED},{VIS}"), "step 8 repack")?;
    verify_archive(pair, ctx, "item2", "step 8: Archive2 archive in ArchiveWork holds meshes\\precombined\\… + vis\\…", &ctx.work_archive(), &ctx.both());
    swap(pair, ctx, "step 8")?;
    remove_tree(pair, &ctx.data.join(PRECOMBINED), "step 8: RD loose Data\\meshes\\precombined")?;
    remove_tree(pair, &ctx.data.join(VIS), "step 8: RD loose Data\\vis")?;
    remove_empty_work(pair, ctx)
}

/// Step 8, BSArch: `unpack` the archive into the work folder, move `vis` in, wait, pack, check,
/// swap, drop the staging tree.
fn step8_bsarch(pair: &mut Pair, ctx: &Ctx, tool: Tool) -> Result<(), String> {
    require_data_archive(pair, ctx)?;
    make_dir(pair, &ctx.work)?;
    let out = run_tool(
        pair,
        ctx,
        tool,
        "step 8: BSArch unpack into ArchiveWork",
        vec![
            Arg::Plain("unpack".into()),
            Arg::Plain(ctx.data_archive().display().to_string()),
            Arg::Plain(ctx.work.display().to_string()),
        ],
    );
    if out.exit != Some(0) {
        return Err(format!("BSArch unpack exited {:?}", out.exit));
    }
    verify_folder(pair, "item1", "step 8: BSArch unpack gives the precombines back byte-identical", &ctx.work, &ctx.meshes, &[]);
    move_dir(pair, ctx, &ctx.data.join(VIS), &ctx.work.join(VIS), "step 8: move Data\\vis into ArchiveWork")?;
    pair.event("wait 5s before BSArch pack", json!(null));
    sleep(SETTLE);
    bsarch_pack(pair, ctx, tool, "step 8 repack")?;
    verify_archive(pair, ctx, "item1", "step 8: BSArch repack holds meshes\\precombined\\… + vis\\… byte-identical", &ctx.work_archive(), &ctx.both());
    swap(pair, ctx, "step 8")?;
    remove_tree(pair, &ctx.work.join("meshes"), "step 8: remove staged precombines")?;
    remove_tree(pair, &ctx.work.join(VIS), "step 8: remove staged previs")?;
    remove_empty_work(pair, ctx)
}

/// Archive2 `-c=` into the work folder, with cwd `Data` and relative sources. If Rust's own
/// quoting of the `-c=` argument fails, it retries once with the batch's `-c="…"` form, so a
/// quoting problem isn't mistaken for "Archive2 can't write outside Data".
fn archive2_create(pair: &mut Pair, ctx: &Ctx, sources: &str, what: &str) -> Result<(), String> {
    let out_path = ctx.work_archive();
    let target = out_path.display().to_string();
    let attempts = [
        ("Rust-quoted \"-c=<path>\"", Arg::Plain(format!("-c={target}"))),
        ("batch-quoted -c=\"<path>\"", Arg::Raw(format!("-c=\"{target}\""))),
    ];
    for (i, (quoting, c_arg)) in attempts.into_iter().enumerate() {
        let out = run_tool(
            pair,
            ctx,
            Tool::Archive2,
            &format!("{what}: Archive2 -c= into ArchiveWork ({quoting})"),
            vec![Arg::Plain(sources.into()), c_arg, Arg::Plain("-f=General".into()), Arg::Plain("-q".into())],
        );
        let exists = out_path.is_file();
        let ok = out.exit == Some(0) && exists;
        pair.check(
            "item2",
            &format!("{what}: Archive2 wrote the archive into ArchiveWork ({quoting})"),
            ok,
            json!({ "exit": out.exit, "archive_exists": exists }),
        );
        if ok {
            return Ok(());
        }
        if i == 0 {
            let _ = fs::remove_file(&out_path); // A partial archive would fool the retry's check.
        }
    }
    Err(format!("{what}: Archive2 did not produce {}", out_path.display()))
}

/// BSArch `pack <ArchiveWork> <ArchiveWork>\<archive> -mt -fo4 -z`, cwd `Data`: the work
/// folder is both the staging tree and the output folder, as the design has it.
fn bsarch_pack(pair: &mut Pair, ctx: &Ctx, tool: Tool, what: &str) -> Result<(), String> {
    let out_path = ctx.work_archive();
    let out = run_tool(
        pair,
        ctx,
        tool,
        &format!("{what}: BSArch pack ArchiveWork into itself"),
        vec![
            Arg::Plain("pack".into()),
            Arg::Plain(ctx.work.display().to_string()),
            Arg::Plain(out_path.display().to_string()),
            Arg::Plain("-mt".into()),
            Arg::Plain("-fo4".into()),
            Arg::Plain("-z".into()),
        ],
    );
    let exists = out_path.is_file();
    if out.exit != Some(0) || !exists {
        return Err(format!("{what}: BSArch exited {:?}, archive exists: {exists}", out.exit));
    }
    Ok(())
}

/// The design's swap: delete the archive in `Data` (if any), then move the checked one in from
/// the work folder. Records where it physically landed when inside MO2.
fn swap(pair: &mut Pair, ctx: &Ctx, stage: &str) -> Result<(), String> {
    let built = ctx.work_archive();
    let target = ctx.data_archive();
    let built_bytes = fs::read(&built).map_err(|e| format!("read {}: {e}", built.display()))?;
    let had_old = target.exists();
    if had_old {
        let r = fs::remove_file(&target);
        let gone = !target.exists();
        pair.check(
            "item3",
            &format!("{stage}: delete the old archive in Data"),
            r.is_ok() && gone,
            json!({ "error": r.err().map(|e| io_err(&e)), "still_visible": !gone, "physical": physical(ctx) }),
        );
    }
    let r = fs::rename(&built, &target);
    let landed = fs::read(&target).ok();
    let bytes_match = landed.as_deref() == Some(&built_bytes[..]);
    let ok = r.is_ok() && bytes_match && !built.exists();
    pair.check(
        "item3",
        &format!("{stage}: std::fs::rename ArchiveWork archive into Data{}", if had_old { " (replacing)" } else { "" }),
        ok,
        json!({
            "rename_error": r.err().map(|e| io_err(&e)),
            "visible_in_data": landed.is_some(),
            "bytes_match_built": bytes_match,
            "still_in_work_folder": built.exists(),
            "physical": physical(ctx),
        }),
    );
    if ok { Ok(()) } else { Err(format!("{stage}: swap into Data failed")) }
}

fn require_data_archive(pair: &mut Pair, ctx: &Ctx) -> Result<(), String> {
    let present = ctx.data_archive().is_file();
    pair.event("step 8: archive present in Data before rebuild", json!({ "present": present, "physical": physical(ctx) }));
    if present { Ok(()) } else { Err("step 8: no archive in Data".into()) }
}

/// Moves a folder with `std::fs::rename`, as the port's `FileSpace::rename` would, and checks
/// both ends from this process's view of the file system (the VFS view inside MO2).
fn move_dir(pair: &mut Pair, ctx: &Ctx, from: &Path, to: &Path, what: &str) -> Result<(), String> {
    let r = fs::rename(from, to);
    let detail = json!({
        "from": from,
        "to": to,
        "error": r.as_ref().err().map(io_err),
        "source_still_visible": from.exists(),
        "files_at_destination": count_files(to),
        "physical": physical(ctx),
    });
    pair.event(what, detail.clone());
    // Which item a move belongs to: the BSArch staging is item 1's setup.
    if r.is_err() || from.exists() {
        pair.check("item1", what, false, detail);
        return Err(format!("{what}: failed"));
    }
    Ok(())
}

/// Removes a folder the probe filled. Only ever called on probe-created trees (see `preflight`).
fn remove_tree(pair: &mut Pair, dir: &Path, what: &str) -> Result<(), String> {
    let r = fs::remove_dir_all(dir);
    pair.event(what, json!({ "error": r.as_ref().err().map(io_err), "still_visible": dir.exists() }));
    r.map_err(|e| format!("{what}: {e}"))
}

/// The work folder must be empty once the archive has moved out. `remove_dir` proves that.
fn remove_empty_work(pair: &mut Pair, ctx: &Ctx) -> Result<(), String> {
    let r = fs::remove_dir(&ctx.work);
    pair.event("remove the now-empty ArchiveWork", json!({ "error": r.as_ref().err().map(io_err) }));
    r.map_err(|e| format!("ArchiveWork not empty after the swap: {e}"))
}

fn make_dir(pair: &mut Pair, dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| {
        let msg = format!("create {}: {e}", dir.display());
        pair.event(&msg, json!(null));
        msg
    })
}

/// Best-effort cleanup after a pair, finished or not. It deletes only probe-created files:
/// `preflight` guaranteed the work folder and the archive didn't exist, and that the loose
/// folders held no files.
fn cleanup(pair: &mut Pair, ctx: &Ctx) {
    let mut errors = Vec::new();
    if ctx.work.exists() {
        if let Err(e) = fs::remove_dir_all(&ctx.work) {
            errors.push(format!("{}: {e}", ctx.work.display()));
        }
    }
    let archive = ctx.data_archive();
    if archive.exists() {
        if let Err(e) = fs::remove_file(&archive) {
            errors.push(format!("{}: {e}", archive.display()));
        }
    }
    for rel in ctx.meshes.keys().chain(ctx.vis.keys()) {
        let f = ctx.data.join(rel);
        if f.exists() {
            if let Err(e) = fs::remove_file(&f) {
                errors.push(format!("{}: {e}", f.display()));
            }
        }
    }
    // Deepest first, so `meshes` is only removed after `meshes\precombined`.
    for (dir, existed) in ctx.data_dirs.iter().rev() {
        if *existed {
            if !dir.exists() {
                let _ = fs::create_dir_all(dir); // The steps RD'd a folder that was there before.
            }
        } else if dir.exists() {
            let _ = fs::remove_dir(dir); // Only succeeds when empty, which is all we want.
        }
    }
    pair.event("cleanup", json!({ "errors": errors, "physical": physical(ctx) }));
}

/// Runs a tool with cwd `Data` (as the batch's `START /D"<Data>"` does) and records its output.
fn run_tool(pair: &mut Pair, ctx: &Ctx, tool: Tool, what: &str, args: Vec<Arg>) -> RunOut {
    let exe = ctx.exe(tool);
    let mut cmd = Command::new(exe);
    cmd.current_dir(&ctx.data);
    let mut shown = vec![quote(&exe.display().to_string())];
    for a in &args {
        match a {
            Arg::Plain(s) => {
                cmd.arg(s);
                shown.push(quote(s));
            }
            Arg::Raw(s) => {
                cmd.raw_arg(s);
                shown.push(s.clone());
            }
        }
    }
    let started = Instant::now();
    let result = cmd.output();
    let millis = started.elapsed().as_millis();
    let (exit, stdout, stderr, spawn_error) = match result {
        Ok(o) => (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            None,
        ),
        Err(e) => (None, String::new(), String::new(), Some(io_err(&e))),
    };
    pair.event(
        what,
        json!({
            "command": shown.join(" "),
            "cwd": ctx.data,
            "exit": exit,
            "millis": millis,
            "stdout": tail(&stdout),
            "stderr": tail(&stderr),
            "spawn_error": spawn_error,
        }),
    );
    RunOut { exit }
}

/// The version line BSArch prints with no arguments, or Archive2's file name.
fn tool_banner(tool: Tool, exe: &Path) -> String {
    if tool == Tool::Archive2 {
        return "Archive2".into();
    }
    let out = Command::new(exe).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let line = text.lines().find(|l| l.contains("BSArch v")).unwrap_or("(no banner)").trim().to_string();
    let expected = if tool == Tool::Bsarch10 { "v1." } else { "v0.9" };
    if line.contains(expected) { line } else { format!("{line}  <-- WARNING: expected {expected}") }
}

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// Deterministic synthetic precombines and previs, named the way the CK names them. Half the
/// files compress well and half are noise, so both stored and zlib-packed records occur. One
/// mesh is tiny.
fn fixtures() -> (Fixture, Fixture) {
    let mut rng = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    fn body(i: u32, size: usize, next: &mut dyn FnMut() -> u64) -> Vec<u8> {
        if i % 2 == 0 {
            (0..size).map(|k| (k % 61) as u8 ^ (i as u8)).collect()
        } else {
            (0..size).map(|_| next() as u8).collect()
        }
    }
    let mut meshes = Fixture::new();
    for i in 0..MESH_COUNT {
        let hash = next() as u32;
        let size = if i == 7 { 40 } else { 700 + (next() % 180_000) as usize };
        let name = format!(r"{PRECOMBINED}\{:08X}_{hash:08X}_OC.nif", 0x0001_0000 + i * 0x37);
        meshes.insert(name, body(i, size, &mut next));
    }
    let mut vis = Fixture::new();
    for i in 0..VIS_COUNT {
        let size = 300 + (next() % 60_000) as usize;
        let name = format!(r"{VIS}\{:08X}.uvd", 0x0002_0000 + i * 0x11);
        vis.insert(name, body(i, size, &mut next));
    }
    (meshes, vis)
}

fn write_fixture(pair: &mut Pair, base: &Path, fx: &Fixture, what: &str) -> Result<(), String> {
    for (rel, bytes) in fx {
        let path = base.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        fs::write(&path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    pair.event(what, json!({ "files": fx.len(), "base": base }));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------------------------

/// Reads an archive with the probe's own BA2 reader and compares it with `expected`.
///
/// It also checks each record's lookup key (name hash, extension, directory hash) against the
/// first archive in this run that held the same path. The game finds files by those hashes,
/// not by the name-table strings, so equal keys mean the game sees the same files whichever
/// tool built the archive. (BSArch writes `/` in the name table where Archive2 writes `\`.)
fn verify_archive(pair: &mut Pair, ctx: &Ctx, item: &str, what: &str, path: &Path, expected: &Fixture) -> bool {
    match read_ba2(path) {
        Err(e) => pair.check(item, what, false, json!({ "archive": path, "error": e })),
        Ok(ba2) => {
            let mut diff = compare(expected, &ba2.entries);
            let source = format!("{} ({} → {})", path.display(), pair.step3.label(), pair.step8.label());
            let mut seen = ctx.lookup_keys.borrow_mut();
            for ((name, _), key) in ba2.entries.iter().zip(&ba2.keys) {
                let key = format!("nameHash={:08x} ext={} dirHash={:08x}", key.0, String::from_utf8_lossy(&key.1).trim_end_matches('\0'), key.2);
                match seen.get(&normalize(name)) {
                    Some((first, from)) if *first != key => {
                        diff.key_differs.push(format!("{name}: {key}, but {first} in {from}"));
                    }
                    Some(_) => {}
                    None => {
                        seen.insert(normalize(name), (key, source.clone()));
                    }
                }
            }
            drop(seen);
            let ok = diff.ok();
            pair.check(
                item,
                what,
                ok,
                json!({
                    "archive": path,
                    "ba2_version": ba2.version,
                    "file_count": ba2.entries.len(),
                    "stored_records": ba2.stored,
                    "first_names": ba2.entries.iter().take(3).map(|(n, _)| n).collect::<Vec<_>>(),
                    "diff": diff.to_json(),
                }),
            )
        }
    }
}

/// The comparison key for an internal path: lower case, `\` separators.
fn normalize(name: &str) -> String {
    name.replace('/', "\\").to_lowercase()
}

/// Compares the loose files under `root` (only the `only` subfolders, when given; top-level
/// `.ba2` files are always skipped) with `expected`, by relative path and bytes.
fn verify_folder(pair: &mut Pair, item: &str, what: &str, root: &Path, expected: &Fixture, only: &[&str]) -> bool {
    let mut found = Vec::new();
    let r = if only.is_empty() {
        collect(root, root, &mut found)
    } else {
        only.iter().try_for_each(|sub| collect(root, &root.join(sub), &mut found))
    };
    if let Err(e) = r {
        return pair.check(item, what, false, json!({ "root": root, "error": e }));
    }
    let diff = compare(expected, &found);
    let ok = diff.ok();
    pair.check(
        item,
        what,
        ok,
        json!({ "root": root, "file_count": found.len(), "first_paths": found.iter().take(3).map(|(n, _)| n).collect::<Vec<_>>(), "diff": diff.to_json() }),
    )
}

/// Path-and-bytes differences between a fixture and what came back. Paths match ignoring case
/// and `/` vs `\` (as the game's hashed lookup does). Spelling changes are reported but don't
/// fail the check. A missing, extra or changed file does, and so does a lookup key that differs
/// from an earlier archive's.
struct Diff {
    missing: Vec<String>,
    extra: Vec<String>,
    mismatched: Vec<String>,
    /// Paths stored with a different spelling than the fixture (case or separators).
    respelled: Vec<String>,
    key_differs: Vec<String>,
}

impl Diff {
    fn ok(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.mismatched.is_empty() && self.key_differs.is_empty()
    }

    fn to_json(&self) -> Value {
        json!({
            "missing": self.missing,
            "extra": self.extra,
            "bytes_differ": self.mismatched,
            "lookup_key_differs_from_earlier_archive": self.key_differs,
            "respelled_count": self.respelled.len(),
            "respelled_examples": self.respelled.iter().take(2).collect::<Vec<_>>(),
        })
    }
}

fn compare(expected: &Fixture, actual: &[(String, Vec<u8>)]) -> Diff {
    let by_key: BTreeMap<String, (&String, &Vec<u8>)> =
        expected.iter().map(|(k, v)| (normalize(k), (k, v))).collect();
    let mut seen = HashSet::new();
    let mut diff = Diff {
        missing: Vec::new(),
        extra: Vec::new(),
        mismatched: Vec::new(),
        respelled: Vec::new(),
        key_differs: Vec::new(),
    };
    for (name, bytes) in actual {
        let key = normalize(name);
        match by_key.get(&key) {
            None => diff.extra.push(name.clone()),
            Some((orig, want)) => {
                seen.insert(key);
                if bytes != *want {
                    diff.mismatched.push(name.clone());
                }
                if name != *orig {
                    diff.respelled.push(format!("{orig} -> {name}"));
                }
            }
        }
    }
    diff.missing = by_key.iter().filter(|(k, _)| !seen.contains(*k)).map(|(_, (o, _))| (*o).clone()).collect();
    diff
}

/// Collects every file under `dir` as (path relative to `root` with `\`, bytes), skipping
/// `.ba2` files directly in `root`.
fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(root, &path, out)?;
            continue;
        }
        let is_top_ba2 = path.parent() == Some(root)
            && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ba2"));
        if is_top_ba2 {
            continue;
        }
        let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
        let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("\\");
        let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        out.push((rel, bytes));
    }
    Ok(())
}

struct Ba2 {
    version: u32,
    entries: Vec<(String, Vec<u8>)>,
    /// Each record's lookup key, parallel to `entries`: (name hash, extension, directory hash).
    keys: Vec<(u32, [u8; 4], u32)>,
    /// Records written uncompressed (packed size 0).
    stored: usize,
}

/// Minimal reader for FO4 `GNRL` BA2 archives. Header: `BTDX`, version, `GNRL`, file count,
/// name-table offset (24 bytes for v1/v7/v8; Starfield's v2/v3 are longer). Then 36-byte
/// records (name hash, ext, dir hash, flags, offset u64, packed u32, unpacked u32, align), then
/// the name table as u16-length-prefixed strings. A packed size of 0 means stored; anything else
/// is a zlib stream.
fn read_ba2(path: &Path) -> Result<Ba2, String> {
    let b = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let get = |o: usize, n: usize| b.get(o..o + n).ok_or_else(|| format!("truncated at byte {o}"));
    let u16_at = |o: usize| get(o, 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |o: usize| get(o, 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()));
    let u64_at = |o: usize| get(o, 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()));

    if get(0, 4)? != b"BTDX" {
        return Err("no BTDX magic".into());
    }
    let version = u32_at(4)?;
    let kind = get(8, 4)?;
    if kind != b"GNRL" {
        return Err(format!("archive type {:?}, expected GNRL", String::from_utf8_lossy(kind)));
    }
    let count = u32_at(12)? as usize;
    let names_at = u64_at(16)? as usize;
    let header_len = match version {
        2 => 32,
        3 => 36,
        _ => 24,
    };

    let mut names = Vec::with_capacity(count);
    let mut p = names_at;
    for _ in 0..count {
        let len = u16_at(p)? as usize;
        names.push(String::from_utf8_lossy(get(p + 2, len)?).into_owned());
        p += 2 + len;
    }

    let mut entries = Vec::with_capacity(count);
    let mut keys = Vec::with_capacity(count);
    let mut stored = 0;
    for (i, name) in names.into_iter().enumerate() {
        let r = header_len + i * 36;
        keys.push((u32_at(r)?, get(r + 4, 4)?.try_into().unwrap(), u32_at(r + 8)?));
        let offset = u64_at(r + 16)? as usize;
        let packed = u32_at(r + 24)? as usize;
        let unpacked = u32_at(r + 28)? as usize;
        let data = if packed == 0 {
            stored += 1;
            get(offset, unpacked)?.to_vec()
        } else {
            let mut out = Vec::with_capacity(unpacked);
            ZlibDecoder::new(get(offset, packed)?)
                .read_to_end(&mut out)
                .map_err(|e| format!("{name}: zlib: {e}"))?;
            if out.len() != unpacked {
                return Err(format!("{name}: inflated {} bytes, header says {unpacked}", out.len()));
            }
            out
        };
        entries.push((name, data));
    }
    Ok(Ba2 { version, entries, keys, stored })
}

// ---------------------------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------------------------

/// Where the probe's files physically sit inside the MO2 instance (overwrite, or any mod
/// folder). `null` outside MO2, where `Data` is the physical location.
fn physical(ctx: &Ctx) -> Value {
    let Some(mo2) = &ctx.mo2 else { return Value::Null };
    let ow = mo2.join("overwrite");
    let first_mesh = ctx.meshes.keys().next().cloned().unwrap_or_default();
    let first_vis = ctx.vis.keys().next().cloned().unwrap_or_default();
    let mut mods = Vec::new();
    if let Ok(entries) = fs::read_dir(mo2.join("mods")) {
        for d in entries.flatten().map(|e| e.path()) {
            let hits: Vec<&str> = [
                (ARCHIVE_NAME, d.join(ARCHIVE_NAME).exists()),
                ("precombines", d.join(&first_mesh).exists()),
                ("previs", d.join(&first_vis).exists()),
            ]
            .into_iter()
            .filter_map(|(n, hit)| hit.then_some(n))
            .collect();
            if !hits.is_empty() {
                mods.push(json!({ "mod": d.file_name().map(|n| n.to_string_lossy().into_owned()), "holds": hits }));
            }
        }
    }
    json!({
        "overwrite_archive": ow.join(ARCHIVE_NAME).exists(),
        "overwrite_precombined_files": count_files(&ow.join(PRECOMBINED)),
        "overwrite_vis_files": count_files(&ow.join(VIS)),
        "mod_folders_holding_probe_files": mods,
        "work_folder_files": count_files(&ctx.work),
    })
}

/// True when MO2's usvfs hook DLL is loaded into this process, i.e. it was started from MO2.
fn usvfs_loaded() -> bool {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::core::w;
    // SAFETY: GetModuleHandleW only looks up an already-loaded module by name. The handle is
    // not kept or used, and no reference count is taken.
    unsafe { GetModuleHandleW(w!("usvfs_x64.dll")).is_ok() || GetModuleHandleW(w!("usvfs_x86.dll")).is_ok() }
}

/// `Installed Path` from the Fallout 4 registry key.
fn registry_fo4() -> Result<PathBuf, String> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\WOW6432Node\Bethesda Softworks\Fallout4")
        .map_err(|e| format!("Fallout 4 registry key: {e}; pass --fo4"))?;
    let installed: String = key.get_value("Installed Path").map_err(|e| format!("Installed Path: {e}; pass --fo4"))?;
    Ok(PathBuf::from(installed.trim_end_matches('\\')))
}

fn save_report(report: &Value, in_mo2: bool) -> Result<PathBuf, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("reports");
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("{unix}-{}.json", if in_mo2 { "inside-mo2" } else { "outside-mo2" }));
    let text = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

fn count_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| e.path())
        .map(|p| if p.is_dir() { count_files(&p) } else { 1 })
        .sum()
}

fn io_err(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => format!("{e} (os error {code})"),
        None => e.to_string(),
    }
}

fn quote(s: &str) -> String {
    if s.contains(' ') { format!("\"{s}\"") } else { s.to_string() }
}

/// The last 4000 characters of a tool's output: enough for an error, without BSArch's progress.
fn tail(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(4000);
    chars[start..].iter().collect()
}
