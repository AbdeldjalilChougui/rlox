//! Runs every `tests/lox/*.lox` file against both back ends and checks the
//! expectations written in comments:
//!
//! - `// expect: <line>`                 one line of stdout, in order
//! - `// expect runtime error: <msg>`    stderr's first line, exit code 70
//! - `// expect compile error: <text>`   stderr contains text, exit code 65

use std::path::Path;
use std::process::Command;

struct Expect {
    stdout: Vec<String>,
    runtime_error: Option<String>,
    compile_error: Option<String>,
}

fn parse_expectations(src: &str) -> Expect {
    let mut e = Expect { stdout: Vec::new(), runtime_error: None, compile_error: None };
    for line in src.lines() {
        if let Some(i) = line.find("// expect: ") {
            e.stdout.push(line[i + "// expect: ".len()..].to_string());
        } else if let Some(i) = line.find("// expect runtime error: ") {
            e.runtime_error = Some(line[i + "// expect runtime error: ".len()..].to_string());
        } else if let Some(i) = line.find("// expect compile error: ") {
            e.compile_error = Some(line[i + "// expect compile error: ".len()..].to_string());
        }
    }
    e
}

fn check(path: &Path, backend: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(path).unwrap();
    let want = parse_expectations(&src);
    let out = Command::new(env!("CARGO_BIN_EXE_rlox")).arg(backend).arg(path).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let got: Vec<&str> = stdout.lines().collect();
    if got != want.stdout {
        return Err(format!("stdout mismatch\n  want: {:?}\n  got:  {:?}\n  stderr: {stderr}", want.stdout, got));
    }
    let code = out.status.code().unwrap_or(-1);
    if let Some(msg) = &want.runtime_error {
        if code != 70 || stderr.lines().next() != Some(msg.as_str()) {
            return Err(format!("want runtime error {msg:?} (exit 70), got exit {code}, stderr {stderr:?}"));
        }
    } else if let Some(msg) = &want.compile_error {
        if code != 65 || !stderr.contains(msg.as_str()) {
            return Err(format!("want compile error {msg:?} (exit 65), got exit {code}, stderr {stderr:?}"));
        }
    } else if code != 0 {
        return Err(format!("unexpected exit {code}, stderr {stderr:?}"));
    }
    Ok(())
}

fn run_suite(backend: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lox");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.retain(|p| p.extension().is_some_and(|e| e == "lox"));
    files.sort();
    assert!(files.len() >= 20, "test suite missing");
    let mut failures = Vec::new();
    for f in &files {
        if let Err(e) = check(f, backend) {
            failures.push(format!("{} [{backend}]: {e}", f.file_name().unwrap().to_string_lossy()));
        }
    }
    assert!(failures.is_empty(), "{} of {} failed:\n{}", failures.len(), files.len(), failures.join("\n\n"));
}

#[test]
fn tree_walker_passes_suite() {
    run_suite("--tree");
}

#[test]
fn bytecode_vm_passes_suite() {
    run_suite("--vm");
}

#[test]
fn repl_keeps_state_between_lines() {
    use std::io::Write;
    for backend in ["--tree", "--vm"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rlox"))
            .arg(backend)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"var a = 40;\nprint oops;\nfun f(x) { return x + 2; }\nprint f(a);\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("42"), "{backend}: {stdout}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("Undefined variable 'oops'."));
    }
}
