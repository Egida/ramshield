//! The analytics README must agree with the crate.
//!
//! An audit of the file against the source found the memory sizes wrong
//! (the Count-Min Sketch is 2 MiB, not 512 KiB; the Welford is 24 bytes,
//! not 32), a usage snippet that does not compile (the types are not
//! re-exported at the crate root, `insert` needs a `mut` binding, and
//! the CMS has `increment`, not `insert`), and a claim that the crate
//! replaces the detection pipeline's `HashSet<IpAddr>` when no other
//! crate depends on it at all.
//!
//! A README that cannot be checked drifts, so the claims that CAN be
//! checked are checked here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is the package dir: crates/ramshield-analytics.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/ramshield-analytics is two levels under the repo root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()))
}

fn readme() -> String {
    read("crates/ramshield-analytics/README.md")
}

/// The backticked names on the line that starts with `prefix`.
fn listed_names(text: &str, prefix: &str) -> BTreeSet<String> {
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with(prefix))
        .unwrap_or_else(|| panic!("the README has no line starting with {prefix:?}"));
    let claim = line
        .trim_start()
        .strip_prefix(prefix)
        .unwrap_or_default()
        .split(". ")
        .next()
        .unwrap_or_default();
    claim
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn the_readme_dependency_list_is_the_crate_dependency_list() {
    let toml = read("crates/ramshield-analytics/Cargo.toml");
    let deps_section = toml
        .split("[dependencies]")
        .nth(1)
        .expect("Cargo.toml has a [dependencies] section")
        .split("\n[")
        .next()
        .unwrap();
    let mut actual = BTreeSet::new();
    for line in deps_section.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            actual.insert(name.trim().to_string());
        }
    }
    assert!(
        !actual.is_empty(),
        "no dependencies were parsed from Cargo.toml"
    );

    let claimed = listed_names(&readme(), "Declared:");
    assert_eq!(
        claimed, actual,
        "the README's dependency list does not match crates/ramshield-analytics/Cargo.toml"
    );
}

#[test]
fn every_import_in_the_usage_snippet_is_a_real_path() {
    let readme = readme();
    let lib = read("crates/ramshield-analytics/src/lib.rs");

    let mut imports = BTreeSet::new();
    for line in readme.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("use ramshield_analytics::") {
            imports.insert(rest.trim_end_matches(';').to_string());
        }
    }
    assert!(
        imports.len() >= 3,
        "the usage snippet imports almost nothing: {imports:?}"
    );

    for path in imports {
        let mut parts = path.split("::");
        let module = parts.next().expect("a path has a module");
        let ty = parts.next().expect("a path has a type");
        assert!(
            parts.next().is_none(),
            "the import `{path}` is deeper than a module and a type"
        );
        assert!(
            lib.contains(&format!("pub mod {module}")),
            "the usage snippet imports `{path}`, but the crate has no public module `{module}`"
        );
        let source = read(&format!("crates/ramshield-analytics/src/{module}.rs"));
        assert!(
            source.contains(&format!("pub struct {ty}")),
            "the usage snippet imports `{path}`, but `{module}.rs` declares no public struct `{ty}`"
        );
    }
}

#[test]
fn the_readme_test_count_is_the_real_test_count() {
    let mut count = 0usize;
    for file in [
        "crates/ramshield-analytics/src/lib.rs",
        "crates/ramshield-analytics/src/hll.rs",
        "crates/ramshield-analytics/src/cms.rs",
        "crates/ramshield-analytics/src/welford.rs",
    ] {
        count += read(file).matches("#[test]").count();
        count += read(file).matches("#[tokio::test]").count();
    }
    assert!(count > 0, "no tests were counted");

    let readme = readme();
    let line = readme
        .lines()
        .find(|l| {
            l.split_whitespace()
                .next()
                .is_some_and(|w| w.trim_end_matches(':').parse::<usize>().is_ok())
                && l.contains("test")
        })
        .expect("the README states a test count");
    let claimed: usize = line
        .split_whitespace()
        .next()
        .and_then(|w| w.trim_end_matches(':').parse().ok())
        .unwrap_or_else(|| panic!("no test count in {line:?}"));
    assert_eq!(
        claimed, count,
        "the README says {claimed} tests; the crate has {count}"
    );
}

#[test]
fn the_readme_memory_sizes_are_the_real_sizes() {
    use ramshield_analytics::cms::{CMS_COLS, CMS_ROWS};
    use ramshield_analytics::hll::SubnetHll;
    use ramshield_analytics::welford::Welford;

    let readme = readme();

    // the HLL
    let hll = std::mem::size_of::<SubnetHll>();
    assert_eq!(hll, 1024, "the HLL's register array changed size");
    assert!(
        paragraph_with(&readme, "SubnetHll").contains("1024 bytes"),
        "the README does not state the HLL's real size (1024 bytes)"
    );

    // the CMS
    let cms = CMS_ROWS * CMS_COLS * std::mem::size_of::<std::sync::atomic::AtomicU64>();
    assert_eq!(cms, 2 * 1024 * 1024, "the CMS's bucket array changed size");
    assert!(
        paragraph_with(&readme, "DecayingCountMinSketch").contains("2 MiB"),
        "the README does not state the CMS's real size (2 MiB)"
    );

    // the Welford
    let welford = std::mem::size_of::<Welford>();
    assert_eq!(welford, 24, "the Welford's size changed");
    assert!(
        paragraph_with(&readme, "Welford").contains("24 bytes"),
        "the README does not state the Welford's real size (24 bytes)"
    );
}

/// The bullet lines of the README section that starts at the heading
/// containing `needle`.
fn paragraph_with(text: &str, needle: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.contains(needle) && l.starts_with('#'))
        .unwrap_or_else(|| panic!("the README has no section about {needle:?}"));
    let mut end = start + 1;
    while end < lines.len() && !lines[end].starts_with('#') {
        end += 1;
    }
    lines[start..end].join("\n")
}

#[test]
fn no_other_crate_depends_on_the_analytics_crate() {
    // The README says nothing in the daemon uses these primitives. That is
    // only true while no other crate depends on this one.
    let root = repo_root();
    let mut dependents = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join("crates")) {
        for entry in entries.flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if manifest.is_file() {
                let text = std::fs::read_to_string(&manifest).unwrap();
                if text.contains("ramshield-analytics") {
                    dependents.push(entry.file_name().to_string_lossy().to_string());
                }
            }
        }
    }
    assert_eq!(
        dependents,
        vec!["ramshield-analytics".to_string()],
        "another crate now depends on ramshield-analytics: the README's \
         \"nothing in the daemon uses them yet\" must be rewritten"
    );
}
