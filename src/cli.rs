use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
pub struct Cli {
    /// The files or folders to delete.
    pub paths: Vec<PathBuf>,
    /// Kill processes without confirm.
    #[arg(short, long)]
    pub yes: bool,
    /// Remove context menu entry.
    #[arg(short, long, conflicts_with = "paths")]
    pub uninstall: bool,
}
