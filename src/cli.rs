use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
pub struct Cli {
    /// The file or folder to delete.
    #[clap(required = true)]
    pub path: PathBuf,
    /// Kill process without confirm.
    #[arg(short, long)]
    pub yes: bool,
}
