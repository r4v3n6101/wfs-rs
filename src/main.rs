use std::{fs::read as fread, io::Cursor, path::PathBuf};

use clap::Parser;
use fuser::MountOption;
use tracing::Level;

mod fs;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Path of mount point
    mount_point: PathBuf,

    /// Paths of WAD files will be loaded
    wads: Vec<PathBuf>,
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    let args = Args::parse();

    let mut fs = fs::WadFS::new();
    for path in args.wads {
        if let Err(err) = fread(&path).and_then(|buf| fs.append_entries(Cursor::new(buf))) {
            tracing::warn!(%err, ?path, "failed reading wad");
        }
    }

    fuser::mount2(
        fs,
        args.mount_point,
        &[
            MountOption::RO,
            MountOption::AllowOther,
            MountOption::AutoUnmount,
        ],
    )
    .unwrap();
}
