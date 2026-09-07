//! Kill all processes occupying a file/folder, and enforce deletion.
//! [Inspired by](https://t.me/withabsolutex/1537)

#![warn(clippy::nursery, clippy::cargo)]
mod cli;

use std::{
    env, fs, io,
    mem::{offset_of, size_of},
    os::windows::ffi::OsStrExt,
    path::Path,
    process::Command,
    ptr,
    thread::sleep,
    time::Duration,
};

use clap::Parser;
use cli::Cli;
use inquire::Confirm;
use windows::{
    core::PCWSTR,
    Wdk::Storage::FileSystem::{FileProcessIdsUsingFileInformation, NtQueryInformationFile},
    Win32::{
        Foundation::{CloseHandle, HANDLE, STATUS_SUCCESS},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION, FILE_SHARE_MODE,
        },
        System::{
            ProcessStatus::{EnumProcessModules, GetModuleBaseNameW},
            Threading::{
                OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_INFORMATION,
                PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, PROCESS_VM_READ,
            },
            IO::IO_STATUS_BLOCK,
        },
    },
};
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_WRITE},
    RegKey,
};

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;

/// Variable-length struct: `process_id_list` actually holds
/// `number_of_process_ids_in_list` entries.
#[repr(C)]
struct FileProcessIdsUsingFileInformation {
    number_of_process_ids_in_list: u32,
    process_id_list: [usize; 1],
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

impl std::fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn remove_any(path: &Path) -> io::Result<()> {
    if path.is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

/// 通过进程ID获取进程名称
#[allow(clippy::missing_safety_doc)]
unsafe fn get_process_name_by_pid(pid: u32) -> Option<String> {
    // 打开目标进程
    let process_handle = match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
    {
        Ok(handle) => handle,
        Err(_) => return None,
    };

    if process_handle.is_invalid() {
        return None;
    }

    let mut module_handles: [windows::Win32::Foundation::HMODULE; 1024] = std::mem::zeroed();
    let mut cb_needed = 0;

    // 枚举进程模块
    if EnumProcessModules(
        process_handle,
        module_handles.as_mut_ptr(),
        size_of::<[windows::Win32::Foundation::HMODULE; 1024]>() as u32,
        &mut cb_needed,
    )
    .is_err()
    {
        CloseHandle(process_handle).ok();
        return None;
    }

    let mut module_name_buffer: [u16; 256] = [0; 256];
    // 获取第一个模块（即可执行文件）的名称
    let name_len = GetModuleBaseNameW(
        process_handle,
        Some(module_handles[0]),
        &mut module_name_buffer,
    );

    CloseHandle(process_handle).ok();

    if name_len == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(
        &module_name_buffer[..name_len as usize],
    ))
}

#[allow(clippy::missing_safety_doc)]
pub unsafe fn get_process_info_from_file_path(path: &Path) -> Result<Vec<ProcessInfo>, String> {
    let mut file: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

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

    let result = query_processes_using_file(handle);
    // Close in every path so the handle is not leaked when the query fails.
    CloseHandle(handle).map_err(|e| e.to_string())?;
    result
}

/// Upper bound for the query buffer; the pid list never needs more than this.
const MAX_QUERY_BUFFER: u32 = 1 << 20;

#[allow(clippy::missing_safety_doc)]
unsafe fn query_processes_using_file(handle: HANDLE) -> Result<Vec<ProcessInfo>, String> {
    let mut iosb: IO_STATUS_BLOCK = std::mem::zeroed();
    let mut bytes = size_of::<FileProcessIdsUsingFileInformation>() as u32;
    loop {
        let mut buffer = vec![0u8; bytes as usize];
        let status = NtQueryInformationFile(
            handle,
            &mut iosb,
            buffer.as_mut_ptr().cast(),
            bytes,
            FileProcessIdsUsingFileInformation,
        );
        // NtQueryInformationFile leaves the transfer size in the iosb union
        // (Pointer field); Clear it, or CloseHandle fails with access denied.
        iosb.Anonymous.Pointer = ptr::null_mut();

        if status != STATUS_SUCCESS {
            if bytes >= MAX_QUERY_BUFFER {
                return Err(format!(
                    "NtQueryInformationFile failed with status: {:X}",
                    status.0 as u32
                ));
            }
            bytes *= 2;
            continue;
        }

        let pids = parse_process_ids(&buffer)
            .ok_or("Pid count reported by the query exceeds the buffer size")?;
        return Ok(pids
            .into_iter()
            .map(|pid| ProcessInfo {
                pid,
                name: get_process_name_by_pid(pid).unwrap_or_else(|| "<Unknown>".to_string()),
            })
            .collect());
    }
}

/// Extract the pid list from a `FileProcessIdsUsingFileInformation` buffer.
/// Returns `None` if the reported count does not fit in the buffer.
fn parse_process_ids(buffer: &[u8]) -> Option<Vec<u32>> {
    let offset = offset_of!(FileProcessIdsUsingFileInformation, process_id_list);
    // SAFETY: the struct is trivially readable from any sufficiently long,
    // aligned-free byte buffer (plain integers, no invalid representations).
    let info = unsafe { &*(buffer.as_ptr() as *const FileProcessIdsUsingFileInformation) };
    let count = info.number_of_process_ids_in_list as usize;
    if buffer.len() < offset + count * size_of::<usize>() {
        return None;
    }
    // SAFETY: `count` entries were just verified to fit in the buffer.
    let list = unsafe { std::slice::from_raw_parts(info.process_id_list.as_ptr(), count) };
    Some(list.iter().map(|&pid| pid as u32).collect())
}

/// Delete the given file/folder, post-order.
/// Returns whether everything under `path` was deleted successfully;
/// a failure on one entry does not abort the remaining ones.
fn visit<F>(path: &Path, remove_fun: &F) -> bool
where
    F: Fn(&Path) -> bool,
{
    let mut ok = true;
    if path.is_dir() {
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => ok &= visit(&entry.path(), remove_fun),
                        Err(e) => {
                            eprintln!("Failed to read dir entry in {}: {e}", path.display());
                            ok = false;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to read dir {}: {e}", path.display());
                ok = false;
            }
        }
    }
    remove_fun(path) & ok
}

#[cfg(windows)]
unsafe fn kill_process(pid: u32) -> std::io::Result<()> {
    // Open with PROCESS_SYNCHRONIZE so we can wait for the actual exit.
    let process_handle = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid)
        .map_err(io::Error::other)?;

    let result = TerminateProcess(process_handle, 1);

    // TerminateProcess is asynchronous: wait (bounded) for the real exit so
    // the file handles are released before we retry the deletion.
    if result.is_ok() {
        let _ = WaitForSingleObject(process_handle, 5000);
    }

    CloseHandle(process_handle)?;
    result.map_err(io::Error::other)
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
    if cli.path.is_empty() {
        // No arguments provided, add context menu entry
        add_context_menu_entry()?;
        println!("Context menu entries added. You can now right-click on files/folders to use the program.");
        let _ = Command::new("cmd").args(["/C", "pause"]).status();
    }
    let mut all_deleted = true;
    for path in cli.path {
        if !path.exists() {
            eprintln!("Path does not exist: {}", path.display());
            all_deleted = false;
            continue;
        }
        let confirm = cli.yes
            || Confirm::new(&format!(
                "Are you sure you want to delete {path:?}? The {} cannot be recovered.",
                if path.is_dir() { "folder" } else { "file" }
            ))
            .with_default(true)
            .prompt()?;
        if !confirm {
            eprintln!("Skipped {path:?}");
            continue;
        }
        all_deleted &= visit(path.as_path(), &|path| unsafe {
            if let Err(e) = remove_any(path) {
                eprintln!("Failed to delete {path:?}: {e}");
                if e.kind() == io::ErrorKind::PermissionDenied {
                    eprintln!("Permission denied, skipping...");
                    return false;
                }
                let pinfos = match get_process_info_from_file_path(path) {
                    Ok(pinfos) => pinfos,
                    Err(e) => {
                        eprintln!("Error occurs when deleting {}: {e}", path.display());
                        return false;
                    }
                };
                let kill = cli.yes
                    || match Confirm::new(&format!("Kill processes:\n{pinfos:#?}?"))
                        .with_default(true)
                        .prompt()
                    {
                        Ok(kill) => kill,
                        Err(e) => {
                            eprintln!("{e}");
                            return false;
                        }
                    };
                if !kill {
                    eprintln!("Skipped deleting {path:?}");
                    return false;
                }
                for pinfo in pinfos {
                    if let Err(e) = kill_process(pinfo.pid) {
                        eprintln!(
                            "Failed to kill process `{}` [{}]: {e}",
                            pinfo.name, pinfo.pid
                        );
                        return false;
                    }
                }
                // Handles may take a brief moment to be released even after
                // the processes have exited, so retry a few times.
                let mut deleted = false;
                for attempt in 0..3 {
                    if remove_any(path).is_ok() {
                        deleted = true;
                        break;
                    }
                    if attempt < 2 {
                        sleep(Duration::from_millis(50));
                    }
                }
                if deleted {
                    println!("Deleted file {path:?}");
                    true
                } else {
                    eprintln!(
                        "Cannot delete {} even if the occupying process has been killed.",
                        path.display()
                    );
                    false
                }
            } else {
                true
            }
        });
    }
    if !all_deleted {
        std::process::exit(1);
    }
    Ok(())
}
