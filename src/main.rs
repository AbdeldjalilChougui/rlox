use rlox::{Backend, Session};
use std::io::{BufRead, Write};
use std::process::ExitCode;

const USAGE: &str = "usage: rlox [--tree | --vm] [script.lox]";

fn main() -> ExitCode {
    // Deeply recursive Lox programs recurse in the tree-walker; give it room.
    let child = std::thread::Builder::new().stack_size(512 * 1024 * 1024).spawn(real_main).expect("spawn");
    child.join().unwrap_or(ExitCode::from(70))
}

fn real_main() -> ExitCode {
    let mut backend = Backend::Vm;
    let mut file = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--tree" => backend = Backend::Tree,
            "--vm" => backend = Backend::Vm,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ if file.is_none() => file = Some(a),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(64);
            }
        }
    }
    let mut session = Session::new(backend, Box::new(std::io::stdout()));
    match file {
        Some(path) => {
            let src = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Could not read '{path}': {e}");
                    return ExitCode::from(74);
                }
            };
            match session.run(&src) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(e.exit_code())
                }
            }
        }
        None => repl(&mut session),
    }
}

fn repl(session: &mut Session) -> ExitCode {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        print!("> ");
        let _ = std::io::stdout().flush();
        let Some(Ok(line)) = lines.next() else {
            println!();
            return ExitCode::SUCCESS;
        };
        if let Err(e) = session.run(&line) {
            eprintln!("{e}");
        }
    }
}
