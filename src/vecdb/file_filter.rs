use std::fs;
use std::io::Read;
use std::path::PathBuf;

const LARGE_FILE_SIZE_THRESHOLD: u64 = 1_000_000;  // 1 MB
const SMALL_FILE_SIZE_THRESHOLD: u64 = 10;  // 10 Bytes

const SOURCE_FILE_EXTENSIONS: &[&str] = &[
    "c", "cpp", "cc", "h", "hpp", "cs", "java", "py", "rb", "go", "rs", "swift",
    "php", "js", "jsx", "ts", "tsx", "lua", "pl", "r", "sh", "bat", "cmd", "ps1",
    "m", "kt", "kts", "groovy", "dart", "fs", "fsx", "fsi", "html", "htm", "css",
    "scss", "sass", "less", "json", "xml", "yml", "yaml", "md", "sql", "db", "sqlite",
    "mdf", "cfg", "conf", "ini", "toml", "dockerfile", "ipynb", "rmd", "swift", "java",
    "xml", "kt", "xaml", "unity", "gd", "uproject", "uasset", "asm", "s", "tex",
    "makefile", "mk", "cmake", "gradle",
];

pub fn is_valid_file(path: &PathBuf) -> bool {
    // Check if the path points to a file
    if !path.is_file() {
        return false;
    }

    // Check if the file is in a hidden directory
    if path.ancestors().any(|ancestor| {
        ancestor.file_name()
            .map(|name| name.to_string_lossy().starts_with('.'))
            .unwrap_or(false)
    }) {
        return false;
    }

    // Check if the file is a source file
    if let Some(extension) = path.extension() {
        if !SOURCE_FILE_EXTENSIONS.contains(&extension.to_str().unwrap_or_default()) {
            return false;
        }
    } else {
        // No extension, not a source file
        return false;
    }

    // Check file size
    if let Ok(metadata) = fs::metadata(path) {
        let file_size = metadata.len();
        if file_size < SMALL_FILE_SIZE_THRESHOLD || file_size > LARGE_FILE_SIZE_THRESHOLD {
            return false;
        }
    } else {
        // Unable to access file metadata
        return false;
    }

    // Check for read permissions
    if fs::read(&path).is_err() {
        return false;
    }

    // Check if the file is not UTF-8
    let mut file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).is_err() {
        return false;
    }
    if String::from_utf8(buffer).is_err() {
        return false;
    }

    // All checks passed
    true
}

