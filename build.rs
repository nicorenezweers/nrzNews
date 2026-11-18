// Migrationen sind ins Programm eingebettet (sqlx::migrate!) – bei Änderungen neu übersetzen.
//
// Die Oberfläche (Dioxus, `ui/`) wird mit `scripts/ui.sh` nach `ui/dist/public` gebaut und hier
// ins Programm eingebettet: Für jede Datei entsteht ein Eintrag in `$OUT_DIR/spa.rs`, Text und
// WebAssembly zusätzlich gzip-komprimiert. Fehlt der Ordner, bleibt die Liste leer und der Server
// zeigt einen Hinweis statt der Oberfläche.
//
// Ebenso eingebettet: die Übersetzungen (`locales/<sprache>.json` → `$OUT_DIR/locales.rs`) und
// Schrift und Icons aus `static/` (→ `$OUT_DIR/static_files.rs`). Es landet nur, was beim
// Übersetzen da ist – ein fehlendes Icon lässt den Build also nicht scheitern.
use std::io::Write;
use std::path::{Path, PathBuf};

/// Dateien aus `static/` mit diesen Endungen kommen ins Programm.
const STATIC_EXTENSIONS: &[&str] = &["woff2", "woff", "png", "svg", "ico", "webp", "webmanifest"];
/// Grössere Dateien in `static/` bleiben draussen (Screenshots o. ä. gehören nicht ins Programm).
const STATIC_MAX_BYTES: u64 = 2 * 1024 * 1024;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    println!("cargo:rerun-if-changed=ui/dist/public");
    println!("cargo:rerun-if-changed=locales");
    println!("cargo:rerun-if-changed=static");

    // ---------------------------------------------------------------- Oberfläche
    let root = manifest.join("ui/dist/public");
    let mut files = Vec::new();
    if root.join("index.html").is_file() {
        collect(&root, &root, &mut files);
    }
    files.sort();
    let mut code = String::from("pub static FILES: &[SpaFile] = &[\n");
    for (i, rel) in files.iter().enumerate() {
        let src = root.join(rel);
        println!("cargo:rerun-if-changed={}", src.display());
        let gz = gzip_include(&out, &format!("spa-{i}.gz"), &src, rel);
        code += &format!(
            "    SpaFile {{ path: {rel:?}, body: include_bytes!({:?}), gzip: {gz} }},\n",
            src.display().to_string()
        );
    }
    code += "];\n";
    write(&out.join("spa.rs"), &code);

    // ---------------------------------------------------------------- Übersetzungen
    // Nur `<code>.json` mit zweibuchstabigem Code (nicht etwa `strings.json`, die Textliste).
    let dir = manifest.join("locales");
    let mut locales: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            let ok = p.extension()? == "json"
                && stem.len() == 2
                && stem.chars().all(|c| c.is_ascii_lowercase());
            ok.then_some((stem, p))
        })
        .collect();
    locales.sort();
    let mut code = String::from("pub static LOCALES: &[Embedded] = &[\n");
    for (i, (lang, path)) in locales.iter().enumerate() {
        println!("cargo:rerun-if-changed={}", path.display());
        let rel = format!("locales/{lang}.json");
        let gz = gzip_include(&out, &format!("locale-{i}.gz"), path, &rel);
        code += &format!(
            "    Embedded {{ path: {rel:?}, body: include_bytes!({:?}), gzip: {gz} }},\n",
            path.display().to_string()
        );
    }
    code += "];\n";
    write(&out.join("locales.rs"), &code);

    // ---------------------------------------------------------------- Schrift und Icons
    let root = manifest.join("static");
    let mut files = Vec::new();
    if root.is_dir() {
        collect(&root, &root, &mut files);
    }
    files.retain(|rel| {
        let ext_ok = rel
            .rsplit_once('.')
            .is_some_and(|(_, ext)| STATIC_EXTENSIONS.contains(&ext));
        let size_ok = std::fs::metadata(root.join(rel)).is_ok_and(|m| m.len() <= STATIC_MAX_BYTES);
        if ext_ok && !size_ok {
            println!("cargo:warning=static/{rel} ist grösser als 2 MiB und wird nicht eingebettet");
        }
        ext_ok && size_ok
    });
    files.sort();
    let mut code = String::from("pub static STATIC_FILES: &[Embedded] = &[\n");
    for (i, rel) in files.iter().enumerate() {
        let src = root.join(rel);
        println!("cargo:rerun-if-changed={}", src.display());
        let gz = gzip_include(&out, &format!("static-{i}.gz"), &src, rel);
        code += &format!(
            "    Embedded {{ path: {rel:?}, body: include_bytes!({:?}), gzip: {gz} }},\n",
            src.display().to_string()
        );
    }
    code += "];\n";
    write(&out.join("static_files.rs"), &code);
}

/// Für komprimierbare Dateien eine gzip-Fassung in `OUT_DIR` schreiben und den passenden
/// `Some(include_bytes!(…))`-Ausdruck liefern, sonst `None`.
fn gzip_include(out: &Path, name: &str, src: &Path, rel: &str) -> String {
    if !compressible(rel) {
        return "None".into();
    }
    let data = std::fs::read(src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(&data).expect("gzip");
    let gz = enc.finish().expect("gzip");
    let path = out.join(name);
    write_bytes(&path, &gz);
    format!("Some(include_bytes!({:?}))", path.display().to_string())
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, files);
        } else if let Ok(rel) = path.strip_prefix(root) {
            files.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn compressible(path: &str) -> bool {
    [
        ".html",
        ".js",
        ".mjs",
        ".css",
        ".wasm",
        ".svg",
        ".json",
        ".txt",
        ".webmanifest",
    ]
    .iter()
    .any(|e| path.ends_with(e))
}

fn write(path: &Path, content: &str) {
    write_bytes(path, content.as_bytes());
}

fn write_bytes(path: &Path, content: &[u8]) {
    std::fs::write(path, content).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}
