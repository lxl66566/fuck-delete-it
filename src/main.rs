//! Kill all processes occupying a file/folder, and enforce deletion.
//! [Inspired by](https://t.me/withabsolutex/1537)

#![warn(clippy::nursery, clippy::cargo)]
mod cli;

use clap::Parser;
use cli::Cli;
use inquire::Confirm;
use std::env;
use std::mem::size_of;
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use std::{fs, io, ptr};
use windows::core::PCWSTR;
use windows::Wdk::Storage::FileSystem::{
    FileProcessIdsUsingFileInformation, NtQueryInformationFile,
};
use windows::Win32::Foundation::{CloseHandle, NTSTATUS, STATUS_SUCCESS};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION, FILE_SHARE_MODE,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;
use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_WRITE};
use winreg::RegKey;

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;

#[allow(dead_code)]
struct FileProcessIdsUsingFileInformation {
    pub number_of_process_ids_in_list: u32,
    pub process_id_list: [usize; 400],
}

fn remove_any(path: &Path) -> io::Result<()> {
    if path.is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

#[allow(clippy::missing_safety_doc)]
pub unsafe fn get_pid_from_image_path(path: &str) -> Result<Vec<usize>, String> {
    let mut file: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

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

    let mut buffer;
    let mut bytes = size_of::<FileProcessIdsUsingFileInformation>() as u32;
    let mut query_status: NTSTATUS = STATUS_SUCCESS;
    for _ in 0..20 {
        buffer = vec![0u8; bytes as usize];
        let ptr = std::mem::transmute::<*const u8, *mut std::ffi::c_void>(buffer.as_ptr());
        let mut ios: Vec<u8> = vec![0u8; size_of::<IO_STATUS_BLOCK>()];
        let iosb: *mut IO_STATUS_BLOCK = ios.as_mut_ptr() as *mut _;

        query_status =
            NtQueryInformationFile(handle, iosb, ptr, bytes, FileProcessIdsUsingFileInformation);

        if query_status != STATUS_SUCCESS {
            bytes *= 2;
            continue;
        }

        let fpi: *mut FileProcessIdsUsingFileInformation = std::mem::transmute(ptr);
        // Access denied error occurs if this pointer is not liberated.
        (*iosb).Anonymous.Pointer = ptr::null_mut();
        CloseHandle(handle).map_err(|e| e.to_string())?;
        return Ok((*fpi)
            .process_id_list
            .into_iter()
            .filter(|&x| x > 40 && x < (1_usize << 20))
            .collect());
    }
    Err(format!(
        "Call to NtQueryInformationProcess failed with status: {:X}",
        query_status.0 as u32
    ))
}

/// delete the given file/folder.
fn visit<F>(path: &Path, remove_fun: &F) -> std::io::Result<()>
where
    F: Fn(&Path) -> Result<&Path, String>,
{
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            visit(&path, remove_fun)?;
        }
    }
    if let Err(reason) = remove_fun(path) {
        eprintln!("{reason}\nError occurs when deleting {}.", &path.display());
    }
    Ok(())
}

#[cfg(windows)]
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

fn add_context_menu_entry() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let current_exe = env::current_exe()?;
    let exe_path = current_exe.to_str().ok_or("Invalid executable path")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes_key = hkcu
        .open_subkey_with_flags("Software\\Classes", KEY_WRITE)
        .or_else(|_| hkcu.create_subkey("Software\\Classes").map(|x| x.0))?;

    // For files (only for current user)
    let (key, _) = classes_key.create_subkey("*\\shell\\FUCK, DELETE IT!\\command")?;
    key.set_value("", &format!("\"{}\" \"%1\"", exe_path))?;
    println!("Added context menu entry for files.");

    // For folders (only for current user)
    let (key, _) = classes_key.create_subkey("Directory\\shell\\FUCK, DELETE IT!\\command")?;
    key.set_value("", &format!("\"{}\" \"%1\"", exe_path))?;
    println!("Added context menu entry for directories.");

    Ok(())
}

fn remove_context_menu_entry() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes_key = hkcu.open_subkey("Software\\Classes")?;

    let key_paths = ["*\\shell", "Directory\\shell"];
    for key_path in key_paths {
        match classes_key.open_subkey_with_flags(key_path, KEY_ALL_ACCESS) {
            Ok(shell_key) => match shell_key.delete_subkey_all("FUCK, DELETE IT!") {
                Ok(_) => {
                    println!("Removed context menu entry for files.");
                }
                Err(e) => {
                    if e.raw_os_error() == Some(2) {
                        println!("Context menu entry for files not found (already removed or never existed).");
                    } else {
                        eprintln!("Error removing file context menu entry: {}", e);
                    }
                }
            },
            Err(e) => {
                if e.raw_os_error() == Some(2) {
                    println!(
                        "Parent key `{}` for files not found, entry likely doesn't exist.",
                        key_path
                    );
                } else {
                    eprintln!("Error opening {} key: {}", key_path, e);
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    if cli.uninstall {
        remove_context_menu_entry()?;
        println!("Context menu entries removed.");
        let _ = Command::new("cmd").args(["/C", "pause"]).status();
        return Ok(());
    }
    if let Some(path) = cli.path.as_ref() {
        visit(path, &|path| unsafe {
            if let Err(e) = remove_any(path) {
                eprintln!("Failed to delete file {path:?}: {e} ");
                let pid =
                    get_pid_from_image_path(path.to_str().ok_or("Not a valid utf-8 filename.")?)?;
                if cli.yes
                    || Confirm::new(&format!("Kill process with pid {pid:?}?"))
                        .with_default(true)
                        .prompt()
                        .map_err(|e| e.to_string())?
                {
                    for p in pid {
                        kill_process(p)
                            .map_err(|e| format!("Failed to kill process with pid {p}: {e}"))?;
                    }
                    sleep(Duration::from_millis(50)); // wait for the process to be killed.
                    remove_any(path).map_err(|_| {
                        format!(
                            "Cannot delete file {} even if the occupying process has been killed.",
                            path.display()
                        )
                    })?;
                    println!("Deleted file {path:?}");
                } else {
                    eprintln!("Skipped deleting file {path:?}");
                }
            }
            Ok(path)
        })?;
    } else {
        // No arguments provided, add context menu entry
        add_context_menu_entry()?;
        println!("Context menu entries added. You can now right-click on files/folders to use the program.");
        let _ = Command::new("cmd").args(["/C", "pause"]).status();
    }

    Ok(())
}
