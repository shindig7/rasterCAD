use std::path::PathBuf;
use std::process::ExitCode;

mod gui;

const USAGE: &str = "\
usage:
  rastercad [FILE.rcad]             open the editor
  rastercad FILE.rcad -o OUT.png    render to a PNG without opening a window";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => gui::run(None),
        [flag] if flag == "-h" || flag == "--help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [file] => gui::run(Some(PathBuf::from(file))),
        [file, flag, out] if flag == "-o" => render_to_file(file, out),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn render_to_file(file: &str, out: &str) -> ExitCode {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {file}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let output = rastercad::render(&source, Some(std::path::Path::new(file)));
    for line in &output.log {
        eprintln!("{line}");
    }
    match output.image {
        Ok(pixmap) => match pixmap.save_png(out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: cannot write {out}: {e}");
                ExitCode::FAILURE
            }
        },
        Err(diag) => {
            eprintln!("error: {diag}");
            ExitCode::FAILURE
        }
    }
}
