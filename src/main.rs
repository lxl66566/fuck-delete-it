//! Kill all processes occupying a file/folder, and enforce deletion.
//! [Inspired by](https://t.me/withabsolutex/1537)

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

use anyhow::{Context, Result, bail};
use clap::Parser;
use cli::Cli;
use inquire::Confirm;
use windows::{
    Wdk::Storage::FileSystem::{FileProcessIdsUsingFileInformation, NtQueryInformationFile},
    Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE, STATUS_SUCCESS},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::{
            Console::GetConsoleProcessList,
            IO::IO_STATUS_BLOCK,
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW,
                TerminateProcess, WaitForSingleObject,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_WRITE},
};

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

/// Variable-length struct: `process_id_list` actually holds
/// `number_of_process_ids_in_list` entries.
#[repr(C)]
struct FileProcessIdsUsingFileInformation {
    number_of_process_ids_in_list: u32,
    process_id_list: [usize; 1],
}

/// Upper bound for the query buffer; the pid list never needs more than this.
const MAX_QUERY_BUFFER: u32 = 1 << 20;

/// Registry locations of the context menu entry, relative to
/// `HKCU\Software\Classes`.
const MENU_LOCATIONS: [(&str, &str); 2] =
    [("*\\shell", "files"), ("Directory\\shell", "directories")];
const MENU_KEY_NAME: &str = "FUCK, DELETE IT!";

fn remove_any(path: &Path) -> io::Result<()> {
    if path.is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

/// Query the full image path of a process by pid. Needs only
/// `PROCESS_QUERY_LIMITED_INFORMATION`, so it also works for elevated
/// processes whose modules could not be enumerated.
fn get_process_name_by_pid(pid: u32) -> Option<String> {
    // SAFETY: plain FFI calls with a valid pid and a correctly sized buffer.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 512];
        let mut len = buffer.len() as u32;
        let name = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .ok()
        .map(|_| String::from_utf16_lossy(&buffer[..len as usize]));
        let _ = CloseHandle(handle);
        name
    }
}

/// Query which processes currently hold handles to the given file/dir.
fn processes_using_file(path: &Path) -> Result<Vec<ProcessInfo>> {
    let file: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: `file` is NUL-terminated; the returned handle is closed on
    // every path before returning.
    let pids = unsafe {
        let handle = CreateFileW(
            PCWSTR(file.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            // FILE_FLAG_BACKUP_SEMANTICS is required to open directories
            // (found empirically: without it CreateFileW fails with access
            // denied on directories, so occupied folders could never be deleted).
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .with_context(|| format!("Cannot open {}", path.display()))?;
        let result = query_processes_using_file(handle);
        // Close in every path so the handle is not leaked when the query fails.
        CloseHandle(handle).context("CloseHandle failed")?;
        result
    }?;
    Ok(pids
        .into_iter()
        .map(|pid| ProcessInfo {
            pid,
            name: get_process_name_by_pid(pid).unwrap_or_else(|| "<unknown>".to_string()),
        })
        .collect())
}

/// Query the pids of processes using an open file handle, retrying with a
/// larger buffer until the pid list fits.
fn query_processes_using_file(handle: HANDLE) -> Result<Vec<u32>> {
    // SAFETY: all pointers point to local buffers sized to match the calls.
    unsafe {
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
            // NtQueryInformationFile leaves the transfer size in the iosb
            // union (Pointer field); clear it, or closing the file handle
            // fails with access denied (found empirically).
            iosb.Anonymous.Pointer = ptr::null_mut();

            if status != STATUS_SUCCESS {
                if bytes >= MAX_QUERY_BUFFER {
                    bail!(
                        "NtQueryInformationFile failed with status {:#X}",
                        status.0 as u32
                    );
                }
                bytes *= 2;
                continue;
            }
            let pids = parse_process_ids(&buffer)
                .context("Pid count reported by the query exceeds the buffer size")?;
            return Ok(pids);
        }
    }
}

/// Extract the pid list from a `FileProcessIdsUsingFileInformation` buffer.
/// Returns `None` if the reported count does not fit in the buffer.
fn parse_process_ids(buffer: &[u8]) -> Option<Vec<u32>> {
    let offset = offset_of!(FileProcessIdsUsingFileInformation, process_id_list);
    // SAFETY: the struct is trivially readable from any sufficiently long
    // byte buffer (plain integers, no invalid representations).
    let info = unsafe { &*(buffer.as_ptr() as *const FileProcessIdsUsingFileInformation) };
    let count = info.number_of_process_ids_in_list as usize;
    if buffer.len() < offset + count * size_of::<usize>() {
        return None;
    }
    // SAFETY: `count` entries were just verified to fit in the buffer.
    let list = unsafe { std::slice::from_raw_parts(info.process_id_list.as_ptr(), count) };
    Some(list.iter().map(|&pid| pid as u32).collect())
}

/// Delete a single file/dir; on errors other than permission denial, find and
/// kill the occupying processes (after confirmation unless `yes`), then retry.
fn delete_one(path: &Path, yes: bool) -> bool {
    if let Err(e) = remove_any(path) {
        eprintln!("Failed to delete {path:?}: {e}");
        if e.kind() == io::ErrorKind::PermissionDenied {
            eprintln!("Permission denied, skipping...");
            return false;
        }
        let pinfos = match processes_using_file(path) {
            Ok(pinfos) => pinfos,
            Err(e) => {
                eprintln!("Error occurs when deleting {}: {e:#}", path.display());
                return false;
            },
        };
        let list: String = pinfos
            .iter()
            .map(|p| format!("  {} ({})\n", p.name, p.pid))
            .collect();
        let kill = yes
            || match Confirm::new(&format!("Kill processes:\n{list}?"))
                .with_default(true)
                .prompt()
            {
                Ok(kill) => kill,
                Err(e) => {
                    eprintln!("{e}");
                    return false;
                },
            };
        if !kill {
            eprintln!("Skipped deleting {path:?}");
            return false;
        }
        for pinfo in &pinfos {
            if let Err(e) = kill_process(pinfo.pid) {
                eprintln!(
                    "Failed to kill process `{}` [{}]: {e:#}",
                    pinfo.name, pinfo.pid
                );
                return false;
            }
        }
        // Handles may take a brief moment to be released even after the
        // processes have exited, so retry a few times.
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
            println!("Deleted {path:?}");
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
}

/// Delete the given file/folder tree, post-order.
/// Returns whether everything under `path` was deleted successfully;
/// a failure on one entry does not abort the remaining ones.
fn remove_tree(path: &Path, delete_one: &dyn Fn(&Path) -> bool) -> bool {
    let mut ok = true;
    if path.is_dir() {
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => ok &= remove_tree(&entry.path(), delete_one),
                        Err(e) => {
                            eprintln!("Failed to read dir entry in {}: {e}", path.display());
                            ok = false;
                        },
                    }
                }
            },
            Err(e) => {
                eprintln!("Failed to read dir {}: {e}", path.display());
                ok = false;
            },
        }
    }
    delete_one(path) & ok
}

/// Terminate a process and wait (bounded) for its real exit.
fn kill_process(pid: u32) -> Result<()> {
    // SAFETY: plain FFI calls with a valid pid; no invariants to uphold.
    unsafe {
        // Open with PROCESS_SYNCHRONIZE so we can wait for the actual exit.
        let handle = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid)
            .with_context(|| format!("OpenProcess({pid}) failed"))?;
        let result = TerminateProcess(handle, 1);
        if result.is_ok() {
            // TerminateProcess is asynchronous: wait (bounded to 5s) for the
            // real exit so the file handles are released before we retry.
            let _ = WaitForSingleObject(handle, 5000);
        }
        let close = CloseHandle(handle);
        result.with_context(|| format!("TerminateProcess({pid}) failed"))?;
        close.with_context(|| format!("CloseHandle({pid}) failed"))?;
        Ok(())
    }
}

fn add_context_menu_entry() -> Result<()> {
    let exe_path = env::current_exe()?;
    let exe_path = exe_path
        .to_str()
        .context("Executable path is not valid UTF-8")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes_key = hkcu
        .open_subkey_with_flags("Software\\Classes", KEY_WRITE)
        .or_else(|_| hkcu.create_subkey("Software\\Classes").map(|x| x.0))?;

    for (shell_key, desc) in MENU_LOCATIONS {
        let (key, _) =
            classes_key.create_subkey(format!("{shell_key}\\{MENU_KEY_NAME}\\command"))?;
        key.set_value("", &format!("\"{exe_path}\" \"%1\""))?;
        println!("Added context menu entry for {desc}.");
    }
    Ok(())
}

fn remove_context_menu_entry() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes_key = hkcu.open_subkey("Software\\Classes")?;

    let not_found = Some(ERROR_FILE_NOT_FOUND.0 as i32);
    for (key_path, desc) in MENU_LOCATIONS {
        match classes_key.open_subkey_with_flags(key_path, KEY_ALL_ACCESS) {
            Ok(shell_key) => match shell_key.delete_subkey_all(MENU_KEY_NAME) {
                Ok(()) => println!("Removed context menu entry for {desc}."),
                Err(e) if e.raw_os_error() == not_found => {
                    println!(
                        "Context menu entry for {desc} not found (already removed or never \
                         existed)."
                    );
                },
                Err(e) => eprintln!("Error removing context menu entry for {desc}: {e}"),
            },
            Err(e) if e.raw_os_error() == not_found => {
                println!("Parent key `{key_path}` not found, entry likely doesn't exist.");
            },
            Err(e) => eprintln!("Error opening {key_path} key: {e}"),
        }
    }
    Ok(())
}

/// Wait for a key press, but only when this process is the sole owner of the
/// console — i.e. launched from Explorer's context menu or a shortcut
fn pause() {
    let mut list = [0u32; 2];
    // SAFETY: `list` is a valid buffer matching the slice length.
    let count = unsafe { GetConsoleProcessList(&mut list) };
    if count == 1 {
        let _ = Command::new("cmd").args(["/C", "pause"]).status();
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.uninstall {
        remove_context_menu_entry()?;
        println!("Context menu entries removed.");
        pause();
        return Ok(());
    }
    if cli.paths.is_empty() {
        // No arguments provided, add context menu entry
        add_context_menu_entry()?;
        println!(
            "Context menu entries added. You can now right-click on files/folders to use the \
             program."
        );
        pause();
        return Ok(());
    }

    let mut all_deleted = true;
    for path in &cli.paths {
        if !path.exists() {
            eprintln!("Path does not exist: {}", path.display());
            all_deleted = false;
            continue;
        }
        let confirm = cli.yes
            || Confirm::new(&format!(
                "Are you sure you want to delete {}? The {} cannot be recovered.",
                path.display(),
                if path.is_dir() {
                    "folder"
                } else {
                    "file"
                }
            ))
            .with_default(true)
            .prompt()?;
        if !confirm {
            eprintln!("Skipped {}", path.display());
            continue;
        }
        all_deleted &= remove_tree(path, &|path| delete_one(path, cli.yes));
    }
    if !all_deleted {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack a query buffer: count, alignment padding, then the pid list.
    fn pack(count: u32, pids: &[usize]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&count.to_ne_bytes());
        v.extend_from_slice(&[0; 4]);
        for pid in pids {
            v.extend_from_slice(&pid.to_ne_bytes());
        }
        v
    }

    #[test]
    fn parse_empty_list() {
        assert_eq!(parse_process_ids(&pack(0, &[])), Some(vec![]));
    }

    #[test]
    fn parse_pids() {
        assert_eq!(
            parse_process_ids(&pack(3, &[4, 1024, 999_999])),
            Some(vec![4, 1024, 999_999])
        );
    }

    #[test]
    fn reject_count_beyond_buffer() {
        // count says 2 pids, buffer only holds 1
        assert_eq!(parse_process_ids(&pack(2, &[7])), None);
    }

    #[test]
    fn reject_short_buffer() {
        assert_eq!(parse_process_ids(&[0; 4]), None);
    }

    #[test]
    fn parse_ignores_trailing_slack() {
        // The retry loop may hand us an oversized buffer.
        let mut buf = pack(1, &[42]);
        buf.extend_from_slice(&[0xaa; 24]);
        assert_eq!(parse_process_ids(&buf), Some(vec![42]));
    }
}
