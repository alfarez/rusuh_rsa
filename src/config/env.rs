use std::fs;

pub(crate) fn load_env() {
    let content = fs::read_to_string(".env").expect("Gagal baca file .env, cek lagi");
    for baris in content.lines() {
        if baris.is_empty() || baris.starts_with("#") {
            continue; // Lewati baris kosong atau komentar
        }
        if let Some((key, value)) = baris.split_once("=") {
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}
