//! test

#[warn(clippy::nursery, clippy::cargo)]
mod cli;

use clap::Parser;
use cli::Cli;
use std::path::Path;
use std::process::Command;
use std::{fs, io};
use windows::core::PCWSTR;
use windows::Wdk::System::Threading::{
    NtQueryInformationProcess, ProcessGroupInformation, ProcessHandleInformation,
};
use windows::Win32::Foundation::{NTSTATUS, STATUS_SUCCESS};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION, FILE_SHARE_MODE,
};

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;
// const FILE_PROCESS_IDS_INFO: u32 = 47;

#[allow(clippy::missing_safety_doc)]
pub unsafe fn get_pid_from_image_path(path: &str) -> Result<Vec<usize>, String> {
    let mut file: Vec<u16> = path.encode_utf16().collect();
    file.push(0);

    let handle = CreateFileW(
        PCWSTR(file.as_mut_ptr()),
        FILE_READ_ATTRIBUTES,
        FILE_SHARE_MODE(FILE_SHARE),
        Some(std::ptr::null_mut()),
        FILE_CREATION_DISPOSITION(OPEN_EXISTING),
        FILE_ATTRIBUTE_NORMAL,
        None,
    )
    .map_err(|e| e.to_string())?;
    dbg!(handle);

    let mut buffer;
    let mut bytes = 400_u32;
    let mut query_status: NTSTATUS = STATUS_SUCCESS;
    for _ in 0..1 {
        buffer = vec![0u8; bytes as usize];
        let ptr = std::mem::transmute(buffer.as_ptr());
        let mut returnlength: u32 = 0;

        query_status = NtQueryInformationProcess(
            handle,
            ProcessHandleInformation,
            ptr,
            bytes,
            &mut returnlength,
        );

        if query_status != STATUS_SUCCESS {
            bytes *= 2;
        } else {
            let fpi: *mut Vec<usize> = std::mem::transmute(ptr);
            let fpi_collect: Box<Vec<usize>> = Box::from_raw(fpi);
            return Ok(fpi_collect.into_iter().filter(|x| x > &4).collect());
        }
    }
    Err(format!(
        "Call to NtQueryInformationProcess failed with status: {:X}",
        query_status.0 as u32
    ))
}

/// delete the given file/folder.
fn visit<F>(path: &Path, cb: &F) -> std::io::Result<()>
where
    F: Fn(&Path) -> Result<&Path, String>,
{
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            visit(&path, cb)?;
        }
        fs::remove_dir(path)?;
    } else if let Err(reason) = cb(path) {
        eprintln!("{reason}");
        eprintln!("Error occurs when deleting {}.", &path.display());
    }
    Ok(())
}

fn get_user_input() -> String {
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .expect("Failed to read line");
    input.trim().to_lowercase()
}

fn yn_selector(message: &str, default: bool) -> bool {
    println!(
        "{} ({}/{}): ",
        message,
        if default { "Y" } else { "y" },
        if !default { "N" } else { "n" }
    );
    loop {
        let input = get_user_input();
        match input.as_str() {
            "" => return default,
            "y" | "yes" => return true,
            "n" | "no" => return false,
            _ => print!("Invalid input. Please enter 'y' or 'n' :"),
        }
    }
}

fn kill_process(pid: usize) -> std::io::Result<()> {
    let _ = Command::new("taskkill")
        .arg("/F")
        .arg("/PID")
        .arg(pid.to_string())
        .status()?;

    // no need to print status because the `taskkill` will status it as well.
    // if status.success() {
    //     println!("Successfully killed process with pid {}", pid);
    // } else {
    //     eprintln!("Failed to kill process with pid {}", pid);
    // }

    Ok(())
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    unsafe {
        dbg!("{:?}", get_pid_from_image_path(cli.path.to_str().unwrap()));
    }
    // visit(cli.path.as_path(), &|path| unsafe {
    //     if let Err(e) = std::fs::remove_file(path) {
    //         eprint!("Failed to delete file {path:?}: {e} ");
    //         let pid = get_pid_from_image_path(path.to_str().ok_or("Not a valid utf-8 filename.")?)?;
    //         if cli.yes || yn_selector(&format!("Kill process with pid {pid:?}?"), true) {
    //             for p in pid {
    //                 kill_process(p)
    //                     .map_err(|e| format!("Failed to kill process with pid {p}: {e}"))?;
    //             }
    //         }
    //         let mut removed = Err(format!(
    //             "Cannot delete file {} even if the occupying process has been killed.",
    //             path.display()
    //         ));
    //         for _ in 0..10 {
    //             std::thread::sleep(std::time::Duration::from_millis(100));
    //             if std::fs::remove_file(path).is_ok() {
    //                 removed = Ok(());
    //                 break;
    //             }
    //         }
    //         removed?;
    //     }
    //     println!("Deleted file {path:?}");
    //     Ok(path)
    // })?;
    Ok(())
}
