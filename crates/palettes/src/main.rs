//! The `palettes` command: runs panel documents.

use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
usage: palettes [FILE...]
       palettes --check FILE...
       palettes msg ACTION [ARGUMENT...]

Opens each panel document on every output it asks for and reloads it when any
of its files is saved; FILE.luau beside FILE.aegle runs with it. Without FILE:
$XDG_CONFIG_HOME/palettes/shell.aegle.
--check parses and checks the documents and their scripts, then exits.
msg runs a host action in the running shell, such as `toggle launcher.aegle`
or a function a script defines, and prints what it returns.";

fn default_document() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("palettes/shell.aegle"))
}

fn run() -> palettes::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "msg") {
        if args.len() < 2 {
            return Err("msg needs an action".into());
        }
        let answer = palettes::ipc::send(&args[1..])?;
        if !answer.is_empty() {
            println!("{answer}");
        }
        return Ok(());
    }
    let check = args.first().is_some_and(|a| a == "--check");
    if check {
        args.remove(0);
    }
    let mut files: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    if files.is_empty() {
        files.extend(default_document());
    }
    // Before the first document makes its directory the working directory.
    let files = files
        .into_iter()
        .map(|f| std::fs::canonicalize(&f).map_err(|e| format!("{}: {e}", f.display())))
        .collect::<Result<Vec<_>, _>>()?;
    if check {
        for file in &files {
            palettes::markup::check(file)?;
        }
        return Ok(());
    }
    let mut documents = palettes::markup::Documents::new()?;
    for file in files {
        documents = documents.open(file)?;
    }
    documents.run()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("palettes: {e}");
            ExitCode::FAILURE
        }
    }
}
