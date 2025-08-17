use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
pub struct Cli {
    /// The files or folders to delete.
    pub path: Vec<PathBuf>,
    /// Kill processes without confirm.
    #[arg(short, long)]
    pub yes: bool,
    /// Remove context menu entry.
    #[arg(short, long)]
    pub uninstall: bool,
}
