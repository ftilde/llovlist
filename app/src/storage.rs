use std::fs;

use crate::platform;

pub fn load(key: &str) -> Option<String> {
    fs::read_to_string(platform::data_dir().join(key)).ok()
}

pub fn save(key: &str, value: &str) {
    let directory = platform::data_dir();
    let _ = fs::create_dir_all(&directory);
    let _ = fs::write(directory.join(key), value);
}

pub fn random_u64() -> u64 {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).expect("the OS refused to provide randomness");
    u64::from_le_bytes(bytes)
}
