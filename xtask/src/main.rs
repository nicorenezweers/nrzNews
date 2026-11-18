//! Hilfsaufgaben für nrzNews: `cargo xtask <aufgabe>` (Alias in `.cargo/config.toml`).
//!
//!   cargo xtask css                 ui/tailwind.css aus ui/tailwind.rs schreiben
//!   cargo xtask ui                  Oberfläche (Dioxus, ui/) nach ui/dist/public bauen
//!   cargo xtask umbrel-release      Paket für den nrzStore (Zweig `umbrel`, Tag `umbrel-<version>`)
//!       --push                      danach Zweig und Tag pushen          (oder PUSH=1)
//!       --notes "…"                 Text des Tags = releaseNotes          (oder NOTES=…)
//!       --skip-build                schon gebaute Programme nehmen        (oder SKIP_BUILD=1)

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../ui/tailwind.rs"]
mod style;

type Result<T = ()> = std::result::Result<T, String>;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("css") => css().map(|path| println!("{}", path.display())),
        Some("ui") => ui(),
        Some("umbrel-release") => umbrel_release(&args[1..]),
        Some("help") | Some("-h") | Some("--help") | None => {
            println!("{}", HELP);
            Ok(())
        }
        Some(other) => Err(format!("Unbekannte Aufgabe „{other}“.\n\n{HELP}")),
    };
    if let Err(e) = result {
        eprintln!("Fehler: {e}");
        exit(1);
    }
}

const HELP: &str =
    "Aufgaben: cargo xtask css | ui | umbrel-release [--push] [--notes TEXT] [--skip-build]";

/// Hauptordner des Projekts (über xtask/).
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask liegt im Projektordner")
        .to_path_buf()
}

/// PATH mit der stabilen Toolchain von rustup und ~/.cargo/bin vorne (für `dx`, `cargo zigbuild`).
fn tool_path() -> String {
    let mut parts = Vec::new();
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        let toolchain = home.join(".rustup/toolchains/stable-aarch64-apple-darwin/bin");
        if toolchain.is_dir() {
            parts.push(toolchain);
        }
        parts.push(home.join(".cargo/bin"));
    }
    if let Some(path) = env::var_os("PATH") {
        parts.extend(env::split_paths(&path));
    }
    env::join_paths(parts)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn run(cmd: &mut Command) -> Result {
    let shown = format!("{cmd:?}");
    let status = cmd
        .env("PATH", tool_path())
        .status()
        .map_err(|e| format!("{shown}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{shown} ist fehlgeschlagen ({status})"))
    }
}

fn output(cmd: &mut Command) -> Result<String> {
    let shown = format!("{cmd:?}");
    let out = cmd.output().map_err(|e| format!("{shown}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "{shown} ist fehlgeschlagen: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn succeeds(cmd: &mut Command) -> bool {
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

fn remove_dir(path: &Path) -> Result {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(path)(e)),
        _ => Ok(()),
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result {
    fs::create_dir_all(to).map_err(io_err(to))?;
    for entry in fs::read_dir(from).map_err(io_err(from))? {
        let entry = entry.map_err(io_err(from))?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(io_err(&entry.path()))?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(io_err(&target))?;
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// `ui/tailwind.css` schreiben – `dx` erzeugt daraus `ui/assets/tailwind.css`.
fn css() -> Result<PathBuf> {
    let path = root().join("ui/tailwind.css");
    if fs::read_to_string(&path).ok().as_deref() != Some(style::TAILWIND_CSS) {
        fs::write(&path, style::TAILWIND_CSS).map_err(io_err(&path))?;
    }
    Ok(path)
}

/// Oberfläche bauen und nach `ui/dist/public` legen – build.rs bettet sie ins Programm ein.
/// Danach `cargo build` im Hauptordner, damit die neue Oberfläche im Programm landet.
/// Braucht: rustup mit Ziel wasm32-unknown-unknown und die Dioxus-CLI (`dx`, Version 0.7).
fn ui() -> Result {
    css()?;
    let ui = root().join("ui");
    // `dx bundle` räumt seinen Zwischenordner nicht auf und kopiert alles daraus mit; darum den
    // vorher leeren, in einen frischen Ordner bauen und das Ergebnis ganz ersetzen (sonst landen
    // alte WASM-Dateien mit im Programm).
    remove_dir(&ui.join("target/dx/nrznews-ui/release/web/public"))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out = PathBuf::from(format!("target/ui-bundle-{stamp}"));
    let dx_version =
        output(Command::new("dx").arg("--version").env("PATH", tool_path())).map_err(|_| {
            "Dioxus-CLI `dx` nicht gefunden (cargo install dioxus-cli --version 0.7.10)"
        })?;
    if !dx_version.contains("dioxus") {
        return Err(format!(
            "`dx` im PATH ist nicht die Dioxus-CLI ({}) – cargo install dioxus-cli --version 0.7.10",
            dx_version.lines().next().unwrap_or_default()
        ));
    }
    run(Command::new("dx")
        .args(["bundle", "--web", "--release", "--out-dir"])
        .arg(&out)
        .current_dir(&ui))?;
    let public = ui.join("dist/public");
    remove_dir(&public)?;
    copy_dir(&ui.join(&out).join("public"), &public)?;
    remove_dir(&ui.join(&out))?;
    if !public.join("index.html").is_file() {
        return Err(format!("{} fehlt", public.join("index.html").display()));
    }
    println!(
        "Oberfläche gebaut: ui/dist/public ({:.1} MB)",
        dir_size(&public) as f64 / 1_000_000.0
    );
    Ok(())
}

fn flag_or_env(args: &[String], flag: &str, var: &str) -> bool {
    args.iter().any(|a| a == flag) || env::var(var).is_ok_and(|v| v == "1")
}

/// Version aus der Cargo.toml im Hauptordner (erste Zeile `version = "…"`).
fn version() -> Result<String> {
    let path = root().join("Cargo.toml");
    let text = fs::read_to_string(&path).map_err(io_err(&path))?;
    text.lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .map(str::to_string)
        .ok_or_else(|| "Keine Version in Cargo.toml".to_string())
}

const TARGETS: [(&str, &str); 2] = [
    ("x86_64-unknown-linux-gnu", "amd64"),
    ("aarch64-unknown-linux-gnu", "arm64"),
];

/// Paket für die App nrz-rss (Anzeigename nrzNews) im nrzStore: baut Oberfläche und Programm für
/// Linux amd64 und arm64 und legt sie als neuen Stand auf den Zweig `umbrel` (nur fertige
/// Dateien) mit dem Tag `umbrel-<version>`. hooks/pre-start der App lädt sie von dort (nrzGit).
/// Braucht: dx 0.7, cargo-zigbuild mit zig, die Rust-Ziele aus `TARGETS`, gzip und shasum.
fn umbrel_release(args: &[String]) -> Result {
    let root = root();
    let git = || {
        let mut c = Command::new("git");
        c.current_dir(&root);
        c
    };
    let version = version()?;
    let tag = format!("umbrel-{version}");
    let notes = args
        .iter()
        .position(|a| a == "--notes")
        .and_then(|i| args.get(i + 1).cloned())
        .or_else(|| env::var("NOTES").ok())
        .unwrap_or_else(|| format!("nrzNews {version}"));

    if succeeds(git().args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])) {
        return Err(format!(
            "Tag {tag} gibt es schon – Version in Cargo.toml erhöhen."
        ));
    }

    if !flag_or_env(args, "--skip-build", "SKIP_BUILD") {
        ui()?;
        // glibc 2.31: läuft auf Debian ab bullseye (die App nimmt debian:trixie-slim)
        for (target, _) in TARGETS {
            run(Command::new("cargo")
                .args(["zigbuild", "--release", "--locked", "--target"])
                .arg(format!("{target}.2.31"))
                .current_dir(&root))?;
        }
    }

    let out = env::temp_dir().join(format!("nrznews-umbrel-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let out_str = out.to_string_lossy().into_owned();
    let result = (|| -> Result {
        if succeeds(git().args(["rev-parse", "-q", "--verify", "refs/heads/umbrel"])) {
            run(git().args(["worktree", "add", "-q", &out_str, "umbrel"]))?;
        } else if succeeds(git().args(["ls-remote", "--exit-code", "--heads", "origin", "umbrel"]))
        {
            run(git().args(["fetch", "-q", "origin", "umbrel:umbrel"]))?;
            run(git().args(["worktree", "add", "-q", &out_str, "umbrel"]))?;
        } else {
            run(git().args(["worktree", "add", "-q", "--detach", &out_str]))?;
            run(git().args(["-C", &out_str, "checkout", "-q", "--orphan", "umbrel"]))?;
            run(git().args(["-C", &out_str, "rm", "-rfq", "."]))?;
        }

        for (target, arch) in TARGETS {
            let binary = root.join(format!("target/{target}/release/nrznews"));
            let gz = out.join(format!("nrznews-{arch}.gz"));
            let file = fs::File::create(&gz).map_err(io_err(&gz))?;
            run(Command::new("gzip")
                .args(["-9", "-n", "-c"])
                .arg(&binary)
                .stdout(file))?;
        }
        fs::copy(root.join("Rocket.toml"), out.join("Rocket.toml"))
            .map_err(io_err(&out.join("Rocket.toml")))?;
        let sums = output(
            Command::new("shasum")
                .args([
                    "-a",
                    "256",
                    "nrznews-amd64.gz",
                    "nrznews-arm64.gz",
                    "Rocket.toml",
                ])
                .current_dir(&out),
        )?;
        fs::write(out.join("SHA256SUMS"), format!("{sums}\n")).map_err(io_err(&out))?;
        fs::write(
            out.join("README.md"),
            format!(
                "# nrzNews – Paket für den nrzStore\n\n\
                 Nur fertige Dateien für die App `nrz-rss` (nrzNews) im nrzStore: das Programm für Linux\n\
                 (`nrznews-amd64.gz`, `nrznews-arm64.gz`, Oberfläche eingebettet), `Rocket.toml` und\n\
                 `SHA256SUMS`. Erstellt mit `cargo xtask umbrel-release` aus dem Zweig `main`.\n\n\
                 Version: {version}\n"
            ),
        )
        .map_err(io_err(&out))?;

        run(git().args(["-C", &out_str, "add", "-A"]))?;
        run(git().args([
            "-C",
            &out_str,
            "commit",
            "-qm",
            &format!("nrzNews {version} fuer den nrzStore"),
        ]))?;
        let head = output(git().args(["-C", &out_str, "rev-parse", "HEAD"]))?;
        run(git().args(["tag", "-a", &tag, "-m", &notes, &head]))?;
        Ok(())
    })();
    if succeeds(git().args(["worktree", "remove", "--force", &out_str])) {
        // erledigt
    } else {
        let _ = fs::remove_dir_all(&out);
    }
    result?;

    println!("Zweig umbrel und Tag {tag} erstellt.");
    if flag_or_env(args, "--push", "PUSH") {
        run(git().args(["push", "origin", "umbrel", &tag]))?;
    }
    Ok(())
}
