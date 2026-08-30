//! Standalone worktree daemon entry point used by release layouts.

use std::path::PathBuf;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let workspace = match (arguments.next(), arguments.next()) {
        (Some(flag), Some(path)) if flag == "--workspace" => PathBuf::from(path),
        _ => {
            eprintln!("usage: walaru-daemon --workspace <path>");
            std::process::exit(2);
        }
    };
    if let Err(error) = walaru_daemon::DaemonServer::serve(workspace) {
        eprintln!("walaru-daemon: {error}");
        std::process::exit(3);
    }
}
