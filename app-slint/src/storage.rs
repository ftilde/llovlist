use std::fs;

#[cfg(target_os = "android")]
mod location {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

    pub fn remember(path: PathBuf) {
        let _ = DATA_DIR.set(path);
    }

    pub fn directory() -> PathBuf {
        DATA_DIR.get().expect("android_main records the data directory first").clone()
    }
}

#[cfg(not(target_os = "android"))]
mod location {
    use std::path::PathBuf;

    pub fn directory() -> PathBuf {
        dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("1kauf")
    }
}

pub use location::*;

pub fn load(key: &str) -> Option<String> {
    fs::read_to_string(directory().join(key)).ok()
}

pub fn save(key: &str, value: &str) {
    let directory = directory();
    let _ = fs::create_dir_all(&directory);
    let _ = fs::write(directory.join(key), value);
}

pub fn random_u64() -> u64 {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).expect("the OS refused to provide randomness");
    u64::from_le_bytes(bytes)
}
