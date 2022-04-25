//
// Mntdf - Df program with mnt crate. 
// Copyright (C) 2022 Łukasz Szpakowski
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, version 3 of the License.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.
//
use std::cmp::max;
use std::env;
use std::ffi::*;
use std::fs;
use std::io::*;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::*;
use std::process::*;
use std::result;
use getopt::Opt;
use libc;
use mnt;
use mnt::MountEntry;
use mnt::MountIter;

struct Options
{
    kilo_flag: bool,
}

struct FormatEntry
{
    file_system: String,
    total: String,
    used: String,
    available: String,
    capacity: String,
    mount_point: String,
}

struct FormatMaxLengths
{
    max_file_system_len: usize,
    max_total_len: usize,
    max_used_len: usize,
    max_available_len: usize,
    max_capacity_len: usize,
    max_mount_point_len: usize,
}

#[allow(dead_code)]
struct StatVFs
{
    bsize: u64,
    frsize: u64,
    blocks: u64,
    bfree: u64,
    bavail: u64,
    files: u64,
    ffree: u64,
    favail: u64,
    fsid: u64,
    flag: u64,
    namemax: u64,
}

fn statvfs<P: AsRef<Path>>(path: P) -> Result<StatVFs>
{
    let path_cstring = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
    let mut statvfs_buf: libc::statvfs = unsafe { MaybeUninit::uninit().assume_init() };
    let res = unsafe { libc::statvfs(path_cstring.as_ptr(), &mut statvfs_buf as *mut libc::statvfs) };
    if res != -1 {
        Ok(StatVFs {
                bsize: statvfs_buf.f_bsize as u64,
                frsize: statvfs_buf.f_frsize as u64,
                blocks: statvfs_buf.f_blocks as u64,
                bfree: statvfs_buf.f_bfree as u64,
                bavail: statvfs_buf.f_bavail as u64,
                files: statvfs_buf.f_files as u64,
                ffree: statvfs_buf.f_ffree as u64,
                favail: statvfs_buf.f_favail as u64,
                fsid: statvfs_buf.f_fsid as u64,
                flag: statvfs_buf.f_flag as u64,
                namemax: statvfs_buf.f_namemax as u64,
        })
    } else {
        Err(Error::last_os_error())
    }
}

fn get_mounts() -> result::Result<Vec<MountEntry>, mnt::ParseError>
{
    let iter = MountIter::new_from_proc()?;
    let mut entries: Vec<MountEntry> = Vec::new();
    for entry in iter {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(err)  => return Err(err),
        }
    }
    Ok(entries)
}

fn find_mount<P: AsRef<Path>>(path: P) -> result::Result<Option<MountEntry>, mnt::ParseError>
{
    let iter = MountIter::new_from_proc()?;
    let mut mount_entry: Option<MountEntry> = None;
    let mut file_len = 0;
    for entry in iter {
        match entry {
            Ok(entry) => {
                let spec = path.as_ref().to_string_lossy().into_owned();
                if entry.spec.starts_with("/") && entry.spec == spec {
                    mount_entry = Some(entry.clone());
                    file_len = usize::MAX;
                } else if path.as_ref().starts_with(&entry.file) {
                    let tmp_file_len = entry.file.as_path().to_string_lossy().len();
                    if tmp_file_len > file_len {
                        mount_entry = Some(entry.clone());
                        file_len = tmp_file_len;
                    }
                }
            },
            Err(err)  => return Err(err),
        }
    }
    Ok(mount_entry)
}

fn header_format_entry(opts: &Options) -> FormatEntry
{
    let total = if opts.kilo_flag { String::from("1024-blocks") } else { String::from("512-blocks") };
    FormatEntry {
        file_system: String::from("Filesystem"),
        total,
        used: String::from("Used"),
        available: String::from("Available"),
        capacity: String::from("Capacity"),
        mount_point: String::from("Mounted on"),
    }
}

fn mount_entry_to_format_entry(mount_entry: &MountEntry, opts: &Options, is_vfs: bool) -> Option<Option<FormatEntry>>
{
    match statvfs(mount_entry.file.as_path()) {
        Ok(statvfs) => {
            let unit_size = if opts.kilo_flag { 1024 } else { 512 };
            let total_blocks = statvfs.blocks;
            if total_blocks != 0 || is_vfs {
                let used_blocks = statvfs.blocks - statvfs.bfree;
                let available_blocks = statvfs.bavail;
                let total_blocks2 = used_blocks + available_blocks;
                let capacity = if total_blocks2 != 0 {
                    format!("{}%", (used_blocks * 100 + total_blocks2 - 1) / total_blocks2)
                } else {
                    String::from("0%")
                };
                let file_system = mount_entry.spec.clone();
                let total = format!("{}", (total_blocks * statvfs.frsize + unit_size - 1) / unit_size);
                let used = format!("{}", (used_blocks * statvfs.frsize + unit_size - 1) / unit_size);
                let available = format!("{}", (available_blocks * statvfs.frsize) / unit_size);
                let mount_point = format!("{}", mount_entry.file.as_path().to_string_lossy());
                Some(Some(FormatEntry {
                        file_system,
                        total,
                        used,
                        available,
                        capacity,
                        mount_point,
                }))
            } else {
                Some(None)
            }
        },
        Err(err) => {
            eprintln!("{}: {}", mount_entry.file.as_path().to_string_lossy(), err);
            None
        },
    }
}

fn calculate_format_max_lens(format_entries: &[FormatEntry]) -> FormatMaxLengths
{
    let mut max_lens = FormatMaxLengths {
        max_file_system_len: 0,
        max_total_len: 0,
        max_used_len: 0,
        max_available_len: 0,
        max_capacity_len: 0,
        max_mount_point_len: 0,
    };
    for format_entry in format_entries.iter() {
        max_lens.max_file_system_len = max(max_lens.max_file_system_len, format_entry.file_system.chars().fold(0, |x, _| x + 1));
        max_lens.max_total_len = max(max_lens.max_total_len, format_entry.total.chars().fold(0, |x, _| x + 1));
        max_lens.max_used_len = max(max_lens.max_used_len, format_entry.used.chars().fold(0, |x, _| x + 1));
        max_lens.max_available_len = max(max_lens.max_available_len, format_entry.available.chars().fold(0, |x, _| x + 1));
        max_lens.max_capacity_len = max(max_lens.max_capacity_len, format_entry.capacity.chars().fold(0, |x, _| x + 1));
        max_lens.max_mount_point_len = max(max_lens.max_mount_point_len, format_entry.mount_point.chars().fold(0, |x, _| x + 1));
    }
    max_lens
}

fn print_format_entries(format_entries: &[FormatEntry], max_lens: &FormatMaxLengths)
{
    for format_entry in format_entries.iter() {
        print!("{:<width$}", format_entry.file_system, width = max_lens.max_file_system_len);
        print!(" ");
        print!("{:>width$}", format_entry.total, width = max_lens.max_total_len);
        print!(" ");
        print!("{:>width$}", format_entry.used, width = max_lens.max_used_len);
        print!(" ");
        print!("{:>width$}", format_entry.available, width = max_lens.max_available_len);
        print!(" ");
        print!("{:>width$}", format_entry.capacity, width = max_lens.max_capacity_len);
        print!(" ");
        print!("{}", format_entry.mount_point);
        println!("");
    }
}

fn main()
{
    let args: Vec<String> = env::args().collect();
    let mut opt_parser = getopt::Parser::new(&args, "kP");
    let mut opts = Options {
        kilo_flag: false,
    };
    loop {
        match opt_parser.next() {
            Some(Ok(Opt('k', _))) => opts.kilo_flag = true,
            Some(Ok(Opt('P', _))) => (),
            Some(Ok(Opt(c, _))) => {
                eprintln!("unknown option -- {:?}", c);
                exit(1);
            },
            Some(Err(err)) => {
                eprintln!("{}", err);
                exit(1);
            },
            None => break,
        }
    }
    let mut status = 0;
    let paths: Vec<&String> = args.iter().skip(opt_parser.index()).collect();
    let mut format_entries: Vec<FormatEntry> = Vec::new();
    format_entries.push(header_format_entry(&opts));
    if !paths.is_empty() {
        for path in paths {
            match fs::metadata(path) {
                Ok(_) => {
                    match find_mount(path) {
                        Ok(Some(mount_entry)) => {
                            match mount_entry_to_format_entry(&mount_entry, &opts, true) {
                                Some(Some(format_entry)) => format_entries.push(format_entry),
                                Some(None)               => (),
                                None                     => status = 1,
                            }
                        },
                        Ok(None) => {
                            eprintln!("{}: Can't find mount entry", path);
                            status = 1;
                        },
                        Err(err) => {
                            eprintln!("{}", err);
                            status = 1;
                        },
                    }
                },
                Err(err) => {
                    eprintln!("{}: {}", path, err);
                    status = 1;
                },
            }
        }
    } else {
        match get_mounts() {
            Ok(mount_entries) => {
                for mount_entry in &mount_entries {
                    match mount_entry_to_format_entry(mount_entry, &opts, false) {
                        Some(Some(format_entry)) => format_entries.push(format_entry),
                        Some(None)               => (),
                        None                     => status = 1,
                    }
                }
            },
            Err(err) => {
                eprintln!("{}", err);
                status = 1;
            },
        }
    }
    if format_entries.len() > 1 {
        let format_max_lens = calculate_format_max_lens(format_entries.as_slice());
        print_format_entries(format_entries.as_slice(), &format_max_lens);
    }
    exit(status);
}
