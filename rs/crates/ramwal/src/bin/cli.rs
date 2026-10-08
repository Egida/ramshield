use ramwal::recovery::{recover_dir, verify_dir};
use std::env::args;
use std::io::Write;

fn main() {
    let a = args().collect::<Vec<String>>();
    let cmd = a.get(1).map(String::as_str).unwrap_or("");
    let dir = a.get(2).map(String::as_str).unwrap_or("");

    let (mode, mutation, r) = match cmd {
        // verify: never mutates. Torn tails are reported, not repaired.
        "verify" => ("VERIFY", "NO", verify_dir(dir)),
        // recover: repair is explicitly allowed.
        "recover" => ("RECOVER", "ALLOWED", recover_dir(dir)),
        _ => {
            std::io::stderr()
                .write_all(b"Usage: ramwal-cli <verify|recover> <wal-dir>\n")
                .ok();
            std::process::exit(2);
        }
    };

    match r {
        Ok(rep) => {
            let s = format!(
                "\
RAMWAL

mode:       {mode}
mutation:   {mutation}
segments:   {}
records:    {}
append_off: {}
first_lsn:  {:?}
last_lsn:   {:?}
tail:       {:?}
repaired:   {}
truncated:  {}
status:     OK
",
                rep.segments,
                rep.records.len(),
                rep.append_offset,
                rep.first_lsn,
                rep.last_lsn,
                rep.tail,
                rep.repaired,
                rep.truncated_bytes,
            );
            std::io::stdout().write_all(s.as_bytes()).ok();
        }
        Err(e) => {
            let s = format!(
                "RAMWAL\n\nmode:       {mode}\nmutation:   {mutation}\nstatus:     FAILED\nreason:     {e}\n"
            );
            std::io::stderr().write_all(s.as_bytes()).ok();
            std::process::exit(1);
        }
    }
}
