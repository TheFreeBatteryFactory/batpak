use crate::bench;
use crate::docs;
use crate::util::{cargo, command_succeeds, repo_root, run};
use crate::{
    BenchSurface, ChaosArgs, DocsArgs, FuzzArgs, MutantMode, MutantsArgs, ReleaseArgs, SetupArgs,
};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, Eq, PartialEq)]
enum InstallStrategy {
    PreferBinstall,
    SourceOnly,
}

pub(crate) fn setup(args: SetupArgs) -> Result<()> {
    let required = [
        (
            "cargo-nextest",
            "cargo-nextest@0.9.132",
            InstallStrategy::PreferBinstall,
        ),
        (
            "cargo-deny",
            "cargo-deny@0.19.0",
            InstallStrategy::PreferBinstall,
        ),
        (
            "cargo-audit",
            "cargo-audit@0.22.1",
            InstallStrategy::PreferBinstall,
        ),
        (
            "cargo-llvm-cov",
            "cargo-llvm-cov@0.8.5",
            InstallStrategy::PreferBinstall,
        ),
        (
            "cargo-mutants",
            "cargo-mutants@27.0.0",
            InstallStrategy::SourceOnly,
        ),
    ];

    let mut missing = Vec::new();
    for (tool, _, _) in required {
        if !command_succeeds(tool, ["--version"]) {
            missing.push(tool);
        }
    }

    if missing.is_empty() {
        println!("All developer tools are installed.");
    } else if args.install_tools {
        if required.iter().any(|(tool, _, strategy)| {
            missing.contains(tool) && *strategy == InstallStrategy::PreferBinstall
        }) {
            ensure_binstall_helper()?;
        }
        for (tool, spec, strategy) in required {
            if missing.contains(&tool) {
                install_tool(spec, strategy)?;
            }
        }
    } else {
        println!("Missing tools: {}", missing.join(", "));
        println!("Run `cargo xtask setup --install-tools` to install the standard toolchain.");
    }

    if cfg!(windows) {
        println!("Native Windows detected. `cargo xtask doctor` will validate the host toolchain.");
    } else {
        println!("Use the checked-in devcontainer for the canonical environment.");
    }
    Ok(())
}

fn ensure_binstall_helper() -> Result<()> {
    if command_succeeds("cargo", ["binstall", "--version"]) {
        return Ok(());
    }

    let mut install = Command::new("cargo");
    install.args(["install", "cargo-binstall"]);
    run(install)
}

fn install_tool(spec: &str, strategy: InstallStrategy) -> Result<()> {
    // Prefer prebuilt binaries when available so `cargo xtask setup` and the
    // devcontainer bootstrap do not spend unnecessary time compiling the
    // helper toolchain from source.
    if strategy == InstallStrategy::PreferBinstall
        && command_succeeds("cargo", ["binstall", "--version"])
    {
        let mut binstall = Command::new("cargo");
        binstall.args(["binstall", "--no-confirm", spec]);
        if run(binstall).is_ok() {
            return Ok(());
        }
        eprintln!("binstall fallback: `{spec}` binary install failed; retrying with cargo install");
    }

    cargo(["install", "--locked", spec])
}

pub(crate) fn quickstart() -> Result<()> {
    cargo(["run", "--example", "quickstart"])
}

pub(crate) fn consumer_smoke() -> Result<()> {
    let root = repo_root()?;
    let smoke_root = root.join("target").join("consumer-smoke");
    if smoke_root.exists() {
        fs::remove_dir_all(&smoke_root).context("clear target/consumer-smoke")?;
    }

    let packaged_root = smoke_root.join("packaged");
    let consumer_root = smoke_root.join("consumer");
    fs::create_dir_all(&packaged_root).context("create packaged crate dir")?;
    fs::create_dir_all(consumer_root.join("src")).context("create consumer src dir")?;

    let mut cargo_package = Command::new("cargo");
    cargo_package
        .current_dir(&root)
        .args(["package", "--allow-dirty", "--no-verify"]);
    run(cargo_package)?;

    let archive = latest_packaged_crate(&root.join("target").join("package"))?;
    let mut unpack = Command::new("tar");
    unpack.current_dir(&packaged_root).arg("xf").arg(&archive);
    run(unpack)?;

    let unpacked_name = unpacked_package_dir(&packaged_root)?;

    fs::write(
        consumer_root.join("Cargo.toml"),
        format!(
            "[package]\n\
             name = \"batpak-consumer-smoke\"\n\
             version = \"0.1.0\"\n\
             edition = \"2021\"\n\
             publish = false\n\
             \n\
             [workspace]\n\
             \n\
             [dependencies]\n\
             batpak = {{ path = \"../packaged/{unpacked_name}\", features = [\"blake3\"] }}\n"
        ),
    )
    .context("write consumer smoke manifest")?;
    fs::write(
        consumer_root.join("src").join("main.rs"),
        "use batpak::prelude::*;\n\
         \n\
         fn main() -> Result<(), Box<dyn std::error::Error>> {\n\
         \x20   let dir = std::env::temp_dir().join(format!(\"batpak-consumer-smoke-{}\", std::process::id()));\n\
         \x20   if dir.exists() {\n\
         \x20       std::fs::remove_dir_all(&dir)?;\n\
         \x20   }\n\
         \x20   std::fs::create_dir_all(&dir)?;\n\
         \n\
         \x20   let config = StoreConfig::new(&dir)\n\
         \x20       .with_sync_every_n_events(1)\n\
         \x20       .with_sync_mode(SyncMode::SyncData);\n\
         \x20   let store = Store::open(config)?;\n\
         \x20   let coord = Coordinate::new(\"consumer:smoke\", \"scope:packaged\")?;\n\
         \x20   let receipt = store.append(&coord, EventKind::custom(0xF, 1), &\"payload\")?;\n\
         \x20   let fetched = store.get(receipt.event_id)?;\n\
         \x20   assert_eq!(fetched.coordinate.scope(), \"scope:packaged\");\n\
         \x20   store.close()?;\n\
         \x20   std::fs::remove_dir_all(&dir)?;\n\
         \x20   Ok(())\n\
         }\n",
    )
    .context("write consumer smoke source")?;

    let mut cargo_run = Command::new("cargo");
    cargo_run
        .current_dir(&consumer_root)
        .args(["run", "--quiet"]);
    run(cargo_run)
}

pub(crate) fn integrity<const N: usize>(subcommand: &str, extra: [&str; N]) -> Result<()> {
    let mut args = vec!["run", "--package", "batpak-integrity", "--", subcommand];
    args.extend(extra);
    cargo(args)
}

pub(crate) fn deny_split() -> Result<()> {
    cargo(["deny", "check"])?;
    cargo(["audit", "--deny", "warnings"])
}

fn count_mutants_file(output_dir: &Path, filename: &str) -> Result<usize> {
    let path = output_dir.join(filename);
    if !path.exists() {
        return Ok(0);
    }
    let contents = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(contents.lines().filter(|l| !l.trim().is_empty()).count())
}

fn assert_mutation_score(output_dir: &Path, min_catch_pct: u32) -> Result<()> {
    let caught = count_mutants_file(output_dir, "caught.txt")?;
    let missed = count_mutants_file(output_dir, "missed.txt")?;
    let tested = caught + missed;

    if tested == 0 {
        eprintln!(
            "mutants: no tested mutants found in {}; skipping score gate",
            output_dir.display()
        );
        return Ok(());
    }

    let score_pct = (caught * 100) / tested;
    println!(
        "mutants: {caught} caught / {tested} tested = {score_pct}% (threshold: {min_catch_pct}%)"
    );

    if score_pct < min_catch_pct as usize {
        bail!(
            "mutation score {score_pct}% is below the required {min_catch_pct}% \
             ({caught} caught, {missed} missed out of {tested} tested mutants). \
             Add tests that catch the mutations listed in {}.",
            output_dir.join("missed.txt").display()
        );
    }
    Ok(())
}

pub(crate) fn mutants(args: MutantsArgs) -> Result<()> {
    let base_all = [
        "mutants",
        "--file",
        "src/store/*.rs",
        "--file",
        "src/wire.rs",
        "--file",
        "src/guard/*.rs",
        "--file",
        "src/pipeline/*.rs",
        "--exclude",
        "src/store/ancestors_clock.rs",
        "--all-features",
        "--test-tool",
        "cargo",
    ];
    let base_min = [
        "mutants",
        "--file",
        "src/store/*.rs",
        "--exclude",
        "src/store/ancestors_hash.rs",
        "--no-default-features",
        "--test-tool",
        "cargo",
    ];

    let output_dir = Path::new("target/mutants.out");
    const MIN_CATCH_PCT: u32 = 20;

    let clear_output = || {
        let _ = std::fs::remove_dir_all(output_dir);
    };

    match args.mode {
        MutantMode::Smoke => {
            clear_output();
            let _ = cargo(
                base_all
                    .into_iter()
                    .chain(["--shard", "1/12"])
                    .collect::<Vec<_>>(),
            );
            assert_mutation_score(output_dir, MIN_CATCH_PCT)?;
            clear_output();
            let _ = cargo(
                base_min
                    .into_iter()
                    .chain(["--shard", "1/12"])
                    .collect::<Vec<_>>(),
            );
            assert_mutation_score(output_dir, MIN_CATCH_PCT)
        }
        MutantMode::Full => {
            clear_output();
            let _ = cargo(base_all);
            assert_mutation_score(output_dir, MIN_CATCH_PCT)?;
            clear_output();
            let _ = cargo(base_min);
            assert_mutation_score(output_dir, MIN_CATCH_PCT)
        }
    }
}

pub(crate) fn fuzz(args: FuzzArgs) -> Result<()> {
    let cases = if args.deep { "100000" } else { "10000" };
    let mut command = Command::new("cargo");
    command.env("PROPTEST_CASES", cases);
    command.args([
        "test",
        "--test",
        "fuzz_targets",
        "--all-features",
        "--release",
        "--",
        "--nocapture",
    ]);
    run(command)
}

pub(crate) fn chaos(args: ChaosArgs) -> Result<()> {
    let iterations = if args.deep { "5000" } else { "2000" };
    let mut command = Command::new("cargo");
    command.env("CHAOS_ITERATIONS", iterations);
    command.args([
        "test",
        "--test",
        "chaos_testing",
        "--all-features",
        "--release",
        "--",
        "--nocapture",
    ]);
    run(command)
}

pub(crate) fn ci() -> Result<()> {
    integrity("doctor", ["--strict"])?;
    integrity("traceability-check", [])?;
    integrity("structural-check", [])?;
    cargo(["fmt", "--check"])?;
    cargo([
        "clippy",
        "--all-features",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    deny_split()?;
    cargo(["nextest", "run", "--profile", "ci", "--all-features"])?;
    cargo(["test", "--doc", "--all-features"])?;
    cargo(["check", "--all-features"])?;
    cargo(["check", "--no-default-features"])?;
    bench::bench_compile(BenchSurface::Neutral)?;
    bench::bench_compile(BenchSurface::Native)
}

pub(crate) fn perf_gates() -> Result<()> {
    cargo([
        "nextest",
        "run",
        "--test",
        "perf_gates",
        "--all-features",
        "--run-ignored",
        "only",
    ])
}

pub(crate) fn release(args: ReleaseArgs) -> Result<()> {
    ci()?;
    consumer_smoke()?;
    docs::docs(DocsArgs { open: false })?;
    if args.dry_run {
        cargo(["publish", "--dry-run", "--allow-dirty"])
    } else {
        bail!("release without --dry-run is intentionally disabled in xtask")
    }
}

fn latest_packaged_crate(package_dir: &Path) -> Result<std::path::PathBuf> {
    let mut latest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in fs::read_dir(package_dir)
        .with_context(|| format!("read packaged crate directory {}", package_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !path.is_file() || !file_name.starts_with("batpak-") || !file_name.ends_with(".crate") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .with_context(|| format!("read modified time for {}", path.display()))?;
        match &latest {
            Some((current, _)) if modified <= *current => {}
            _ => latest = Some((modified, path)),
        }
    }

    latest
        .map(|(_, path)| path)
        .context("could not locate packaged batpak .crate archive")
}

fn unpacked_package_dir(packaged_root: &Path) -> Result<String> {
    let mut unpacked = None;
    for entry in fs::read_dir(packaged_root)
        .with_context(|| format!("read unpacked package dir {}", packaged_root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if entry.path().join("Cargo.toml").is_file() {
            unpacked = Some(entry.file_name().to_string_lossy().into_owned());
            break;
        }
    }

    unpacked.context("could not locate unpacked batpak package directory")
}
