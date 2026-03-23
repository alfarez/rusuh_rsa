use std::fs;

pub(crate) fn load_env() {
    let Ok(content) = fs::read_to_string(".env") else {
        return; // .env tidak ada, lanjut — env var dari Docker tetap terbaca
    };

    for baris in content.lines() {
        if baris.is_empty() || baris.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = baris.split_once('=') {
            unsafe {
                std::env::set_var(key.trim(), value.trim());
            }
        }
    }
}
