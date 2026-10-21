//! The ledgers must agree with each other, and with master.
//!
//! There are four documents that state what is done and what is not:
//! `BACKLOG.md`, `docs/development/OPEN_FINDINGS.md`,
//! `docs/development/DOC_DRIFT_SWEEP.md` and
//! `docs/development/BACKLOG_VALIDATION_AGAINST_MASTER.md`. They are
//! written by hand, across many passes, and they rot: a finding says
//! "no code change has been made" after the change landed, a table row
//! and its own section disagree, a commit SHA points at a side branch
//! that was never merged.
//!
//! This test is the cheap standing check. It does not judge whether a
//! claim is true, which needs a human or an agent with the git history. It
//! checks that the documents cannot silently contradict themselves.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()))
}

/// The four ledger documents.
fn ledgers() -> Vec<&'static str> {
    vec![
        "BACKLOG.md",
        "docs/development/OPEN_FINDINGS.md",
        "docs/development/DOC_DRIFT_SWEEP.md",
        "docs/development/BACKLOG_VALIDATION_AGAINST_MASTER.md",
    ]
}

/// Classify a status cell. `None` means the status is not one of the
/// states this test understands, so the check is skipped for it.
fn status_of(text: &str) -> Option<bool> {
    // true = closed, false = open
    //
    // The status is the FIRST word of the cell, not any word in it: a cell
    // that says "**OPEN** - the fix is on a branch, PR #39 was closed
    // unmerged" is open, and a cell that merely mentions "closed" later in
    // a sentence must not be read as closed.
    let mut t = text.trim().trim_start_matches('*').trim_start();
    for prefix in ["**Status:**", "**Status**:", "Status:", "status:"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim_start();
            break;
        }
    }
    let head: String = t
        .trim_start_matches('*')
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == ' ')
        .collect::<String>()
        .to_ascii_uppercase();
    let head = head.trim();
    if head.is_empty() {
        return None;
    }
    if head.starts_with("NOT ACTIONABLE") {
        return None;
    }
    if head.starts_with("CLOSED") || head.starts_with("FIXED") {
        return Some(true);
    }
    if head.starts_with("OPEN") || head.starts_with("IN PROGRESS") || head.starts_with("PENDING") {
        return Some(false);
    }
    None
}

/// A table row split into its cells, without the empty leading and
/// trailing cells the `|` separators produce.
fn split_row(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if !line.starts_with('|') {
        return None;
    }
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    if cells.len() < 4 {
        return None;
    }
    let id = cells[1].trim_matches('*').trim().to_string();
    let severity = cells[3].trim_matches('*').trim().to_string();
    Some((id, severity))
}

fn is_finding_id(id: &str) -> bool {
    id.starts_with('F') && id.len() > 1 && id[1..].chars().all(|c| c.is_ascii_digit())
}

fn looks_like_severity(cell: &str) -> bool {
    let c = cell.to_ascii_uppercase();
    c.contains("P1") || c.contains("P2") || c.contains("P3") || c.contains("N/A")
}

struct SweepEntry {
    start: usize,
    path: String,
}

/// The `**F<n> (SEVERITY)** `file`` headings of the drift sweep.
fn sweep_entries(text: &str) -> Vec<SweepEntry> {
    let mut out = Vec::new();
    for (start, line) in text.split_inclusive('\n').enumerate() {
        let _ = start;
        let _ = line;
    }
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.starts_with("**F")
            && let Some(close) = trimmed.find(")**")
        {
            // The path is not always right after the severity - one claim
            // names several files - so count the claim either way and
            // take the path when there is one.
            let after = &trimmed[close + 3..];
            let path = after
                .find('`')
                .and_then(|tick| {
                    // `offset` is where this line starts in the file, so
                    // the global index is offset + the position in the
                    // line. Mixing the two silently reads the wrong text.
                    let path_start = offset + indent + close + 3 + tick + 1;
                    text[path_start..]
                        .find('`')
                        .map(|end| text[path_start..path_start + end].to_string())
                })
                .unwrap_or_default();
            out.push(SweepEntry {
                start: offset,
                path,
            });
        }
        offset += line.len();
    }
    out
}

#[test]
fn every_finding_has_a_table_row_and_a_section() {
    let text = read("docs/development/OPEN_FINDINGS.md");

    // Table rows: `| F1 | finding | severity | status | owner |`.
    let mut table: Vec<String> = Vec::new();
    for line in text.lines() {
        let cells = split_row(line);
        let Some((id, severity)) = cells.as_ref().map(|(i, s)| (i, s)) else {
            continue;
        };
        if is_finding_id(id) && looks_like_severity(severity) {
            table.push(id.clone());
        }
    }

    let mut seen = BTreeSet::new();
    let mut dupes = Vec::new();
    for id in &table {
        if !seen.insert(id.clone()) {
            dupes.push(id.clone());
        }
    }
    assert!(
        dupes.is_empty(),
        "OPEN_FINDINGS.md table has duplicate rows: {dupes:?}"
    );
    assert!(
        !table.is_empty(),
        "OPEN_FINDINGS.md table rows were not found - the format changed"
    );

    // Sections: `## F1 - title`.
    let mut sections: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if id.starts_with('F') && id[1..].chars().all(|c| c.is_ascii_digit()) {
                sections.push(id);
            }
        }
    }

    let table_set: BTreeSet<String> = table.iter().cloned().collect();
    let section_set: BTreeSet<String> = sections.iter().cloned().collect();

    let missing_section: Vec<&String> = table_set.difference(&section_set).collect();
    assert!(
        missing_section.is_empty(),
        "OPEN_FINDINGS.md table rows without a section: {missing_section:?}"
    );
    let missing_row: Vec<&String> = section_set.difference(&table_set).collect();
    assert!(
        missing_row.is_empty(),
        "OPEN_FINDINGS.md sections without a table row: {missing_row:?}"
    );
}

#[test]
fn a_finding_s_table_row_and_its_section_agree_on_the_status() {
    let text = read("docs/development/OPEN_FINDINGS.md");
    let mut problems: Vec<String> = Vec::new();

    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 5 {
            continue;
        }
        let id = cells[1].trim_matches('*').trim();
        if !(id.starts_with('F') && id[1..].chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        let table_status = status_of(cells[4]);
        let Some(table_closed) = table_status else {
            continue;
        };

        // Find the section for this id and take its first `**Status:**` line.
        let heading = format!("## {id} ");
        let Some(start) = text.find(&heading) else {
            continue;
        };
        let rest = &text[start..];
        let end = rest[1..].find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
        let section = &rest[..end];

        let mut section_status = None;
        for sl in section.lines() {
            if let Some(t) = sl.trim().strip_prefix("**Status:**") {
                section_status = status_of(t);
                break;
            }
        }
        let Some(section_closed) = section_status else {
            continue;
        };
        if table_closed != section_closed {
            problems.push(format!(
                "{id}: table says \"{}\" but the section says \"{}\" (line {})",
                cells[4],
                section
                    .lines()
                    .find(|l| l.contains("**Status:**"))
                    .unwrap_or(""),
                idx + 1
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "OPEN_FINDINGS.md status disagreements:\n{}",
        problems.join("\n")
    );
}

#[test]
fn a_closed_finding_does_not_still_say_it_is_not_started() {
    let text = read("docs/development/OPEN_FINDINGS.md");
    let mut problems: Vec<String> = Vec::new();

    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 5 {
            continue;
        }
        let id = cells[1].trim_matches('*').trim();
        if !(id.starts_with('F') && id[1..].chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        let Some(true) = status_of(cells[4]) else {
            continue;
        };

        let heading = format!("## {id} ");
        let Some(start) = text.find(&heading) else {
            continue;
        };
        let rest = &text[start..];
        let end = rest[1..].find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
        let section = &rest[..end];
        for sl in section.lines() {
            let s = sl.trim();
            if s.starts_with("**Not started")
                || s.contains("No code change has been made")
                || s.contains("has not been actioned")
            {
                problems.push(format!(
                    "{id} is closed in the table (line {}) but its section still says \"{}\"",
                    idx + 1,
                    s
                ));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "OPEN_FINDINGS.md closed findings that still read as not started:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_drift_sweep_counts_match_its_own_entries() {
    let text = read("docs/development/DOC_DRIFT_SWEEP.md");

    // Entries are `**F<n> (SEVERITY)** `file`` on a line of their own.
    let mut fixed = 0usize;
    let mut remaining = 0usize;
    let mut by_readme: std::collections::BTreeMap<String, usize> = Default::default();

    let entries = sweep_entries(&text);
    assert!(!entries.is_empty(), "the sweep entries were not found");
    for (i, entry) in entries.iter().enumerate() {
        let end = entries.get(i + 1).map(|e| e.start).unwrap_or(text.len());
        let block = &text[entry.start..end];
        let readme = entry
            .path
            .split(':')
            .next()
            .unwrap_or(&entry.path)
            .to_string();
        if block.contains("Status: FIXED") {
            fixed += 1;
        } else {
            remaining += 1;
            *by_readme.entry(readme).or_default() += 1;
        }
    }

    // The header states the split. Parse it.
    let header = text.lines().take(12).collect::<Vec<_>>().join(" ");
    // "**62 claims: 17 fixed, 45 open.**"
    let claimed_total = extract_count(&header, " claims: ")
        .and_then(|total| extract_count(&header, " fixed, ").map(|fixed| (fixed, total)));
    let Some((claimed_fixed, claimed_total)) = claimed_total else {
        panic!("the sweep header no longer states \"N of the M claims are fixed\"");
    };
    assert_eq!(
        claimed_fixed, fixed,
        "the sweep header claims {claimed_fixed} fixed claims, the file has {fixed}"
    );
    assert_eq!(
        claimed_total,
        fixed + remaining,
        "the sweep header claims {claimed_total} claims, the file has {}",
        fixed + remaining
    );

    // The header also splits the remainder by README.
    for (readme, count) in &by_readme {
        // the crate, not the file: every README is called README.md
        let crate_name = readme
            .rsplit('/')
            .nth(1)
            .unwrap_or(readme)
            .trim_start_matches("ramshield-");
        let mentioned = header
            .split("(")
            .any(|chunk| chunk.contains(crate_name) && chunk.contains(&count.to_string()));
        assert!(
            mentioned,
            "the sweep header does not mention {readme} ({crate_name}) with {count} remaining claims"
        );
    }
}

/// The number that sits immediately before `marker`.
fn extract_count(text: &str, marker: &str) -> Option<usize> {
    let i = text.find(marker)?;
    let head = &text[..i];
    let digits: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.chars().rev().collect::<String>().parse().ok()
}

#[test]
fn every_sha_cited_in_a_ledger_names_a_commit() {
    // The ledgers cite the commits that did the work. A typo, or a commit
    // that was never made, is the same class of error as a wrong status: it
    // points the reader at something that does not exist.
    //
    // So a ledger may only cite a SHA that a reader can find in the
    // shipped tree. A SHA from an unpublished or unmerged branch is not
    // one: a clone of this repository does not have that commit, so the
    // citation points at nothing. Name the branch instead - a branch name
    // is a handle that survives, a parked tip SHA does not.
    //
    // A SHALLOW checkout cannot answer this: the objects for the older
    // commits are simply not there, and the cited SHAs reach 788 commits
    // back. So the check says what it could not verify and does not claim
    // a pass it did not earn. A full clone - every developer's machine -
    // enforces it completely.
    let Some(git) = git_available() else {
        eprintln!("NOT RUN: git is not available, the SHA check did not run");
        return;
    };
    if is_shallow(&git) {
        let mut cited = 0usize;
        for rel in ledgers() {
            for line in read(rel).lines() {
                cited += shas_in(line).len();
            }
        }
        eprintln!(
            "NOT RUN: the checkout is shallow, so none of the {cited} cited SHAs \
             could be checked for existence. A full clone enforces this check."
        );
        return;
    }
    let mut problems: Vec<String> = Vec::new();
    for rel in ledgers() {
        let text = read(rel);
        for (idx, line) in text.lines().enumerate() {
            for sha in shas_in(line) {
                if !object_exists(&git, &sha) {
                    problems.push(format!(
                        "{rel}:{}: {sha} is not a commit in this repository",
                        idx + 1
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "ledger SHAs that name no commit:\n{}",
        problems.join("\n")
    );
}

/// A token that looks like a commit SHA rather than a word.
///
/// Lowercase only: a real SHA is lowercase hex, and this excludes
/// `Ed25519` and `SHA-256`. At least one letter: this excludes the
/// all-digit GitHub Actions run ids the ledgers cite. The length floor
/// is 7, the shortest a git will abbreviate to.
///
/// A token with no digit at all (`deadbee`) is still checked: the
/// ledgers contain no such token today, and a guard that silently
/// skips a whole class of typo is worse than one that names a token a
/// future document has to avoid.
fn shas_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_hexdigit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let token: String = chars[start..i].iter().collect();
            let len = token.len();
            if (7..=40).contains(&len)
                && token
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
                && token.chars().any(|c| ('a'..='f').contains(&c))
            {
                out.push(token);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn is_shallow(git: &Path) -> bool {
    let Ok(out) = std::process::Command::new(git)
        .arg("-C")
        .arg(repo_root())
        .args(["rev-parse", "--is-shallow-repository"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim() == "true"
}

fn git_available() -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["rev-parse", "--git-dir"])
        .output()
        .ok()?;
    if out.status.success() {
        Some(PathBuf::from("git"))
    } else {
        None
    }
}

fn object_exists(git: &Path, sha: &str) -> bool {
    std::process::Command::new(git)
        .arg("-C")
        .arg(repo_root())
        .args(["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
