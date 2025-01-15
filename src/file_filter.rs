use std::fs;
#[cfg(not(windows))]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use crate::privacy::FilePrivacySettings;

const LARGE_FILE_SIZE_THRESHOLD: u64 = 180*1024; // 180k files (180k is ~0.2% of all files on our dataset)
const SMALL_FILE_SIZE_THRESHOLD: u64 = 5;        // 5 Bytes

pub const SOURCE_FILE_EXTENSIONS: &[&str] = &[
    "c", "cpp", "cc", "h", "hpp", "cs", "java", "py", "rb", "go", "rs", "swift",
    "php", "js", "jsx", "ts", "tsx", "lua", "pl", "r", "sh", "bat", "cmd", "ps1",
    "m", "kt", "kts", "groovy", "dart", "fs", "fsx", "fsi", "html", "htm", "css",
    "scss", "sass", "less", "json", "xml", "yml", "yaml", "md", "sql", "db", "sqlite",
    "mdf", "cfg", "conf", "ini", "toml", "dockerfile", "ipynb", "rmd", "xml", "kt",
    "xaml", "unity", "gd", "uproject", "uasset", "asm", "s", "tex", "makefile", "mk",
    "cmake", "gradle", "liquid"
];

pub fn is_valid_file(path: &PathBuf, allow_hidden_folders: bool, ignore_size_thresholds: bool) -> Result<(), Box<dyn std::error::Error>> {
    if !path.is_file() {
        return Err("Path is not a file".into());
    }

    if !allow_hidden_folders && path.ancestors().any(|ancestor| {
        ancestor.file_name()
            .map(|name| name.to_string_lossy().starts_with('.'))
            .unwrap_or(false)
    }) {
        return Err("Parent dir stars with a dot".into());
    }

    if let Ok(metadata) = fs::metadata(path) {
        let file_size = metadata.len();
        if !ignore_size_thresholds && file_size < SMALL_FILE_SIZE_THRESHOLD {
            return Err("File size is too small".into());
        }
        if !ignore_size_thresholds && file_size > LARGE_FILE_SIZE_THRESHOLD {
            return Err("File size is too large".into());
        }
        #[cfg(not(windows))]
        {
            let permissions = metadata.permissions();
            if permissions.mode() & 0o400 == 0 {
                return Err("File has no read permissions".into());
            }
        }
    } else {
        return Err("Unable to access file metadata".into());
    }
    Ok(())
}

pub fn is_this_inside_blacklisted_dir(path: &PathBuf, privacy_settings: &FilePrivacySettings) -> bool {
    let mut path = path.clone();
    while path.parent().is_some() {
        path = path.parent().unwrap().to_path_buf();
        if is_this_blacklisted_file(&path, &privacy_settings) {
            return true;
        }
    }
    false
}


pub fn is_this_blacklisted_file(path: &PathBuf, privacy_settings: &FilePrivacySettings) -> bool {
    if let Some(file_name) = path.file_name() {
        let filename_str = file_name.to_str().unwrap_or_default().to_string();
        if privacy_settings.blacklisted.contains(&filename_str)
            && !privacy_settings.whitelisted.contains(&filename_str) {
            return true;
        }
        // TODO: this is not obvious code
        if let Some(file_name_str) = file_name.to_str() {
            if file_name_str.starts_with(".") {
                return true;
            }
            if let Some(file_name_str) = file_name.to_str() {
                if file_name_str.starts_with(".") {
                    return true;
                }
            }
        }
    }
    false
}

