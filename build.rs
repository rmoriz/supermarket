use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Flattens `config/locales/en.yml` into `key.path => string` at build time so the
/// binary never parses YAML and wording stays a single source of truth.
fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let locales_dir = manifest.join("supermarket-ruby/src/supermarket/config/locales");
    let entry = locales_dir.join("en.yml");
    println!("cargo:rerun-if-changed={}", entry.display());
    println!("cargo:rerun-if-changed={}", locales_dir.display());

    let mut files: Vec<PathBuf> = fs::read_dir(&locales_dir)
        .map(|dir| {
            dir.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "yml"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();

    let mut table: BTreeMap<String, String> = BTreeMap::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("locale file readable");
        let scope = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("en")
            .to_owned();
        for (key, value) in flatten(&text) {
            // The default locale's keys are unscoped; other locale files keep a locale prefix.
            let key = if scope == "en" || key.starts_with(&format!("{scope}.")) {
                key
            } else {
                format!("{scope}.{key}")
            };
            table.entry(key).or_insert(value);
        }
    }

    let table: BTreeMap<String, String> = table
        .into_iter()
        .map(|(k, v)| (k.strip_prefix("en.").unwrap_or(&k).to_owned(), v))
        .collect();
    let mut out = String::from("pub static MESSAGES: &[(&str, &str)] = &[\n");
    for (key, value) in &table {
        out.push_str(&format!("    ({:?}, {:?}),\n", key, value));
    }
    out.push_str("];\n");

    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("locales.rs");
    fs::write(dest, out).expect("write generated locales");
}

fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    let inner = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')));
    match inner {
        Some(inner) => inner
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
            .replace("\\n", "\n"),
        None => raw.to_owned(),
    }
}

/// Two-space-indented YAML subset: enough for the locale files in this repo.
fn flatten(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with("---") {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        let content = trimmed.trim_start();

        let Some((key, value)) = content.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        while stack.last().is_some_and(|(depth, _)| *depth >= indent) {
            stack.pop();
        }

        if value.is_empty() {
            stack.push((indent, key.to_owned()));
            continue;
        }

        let mut path: Vec<&str> = stack.iter().map(|(_, k)| k.as_str()).collect();
        path.push(key);
        let flat = path.join(".");
        out.push((flat, unquote(value)));
    }

    out
}
