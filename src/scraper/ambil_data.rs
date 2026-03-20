use chrono::Timelike;
use reqwest::Method;
use std::{path::Path, time::Instant};

use crate::config::heade::header;

fn url_rsa() -> String {
    std::env::var("BASE_URL_RSA").expect("BASE_URL_RSA gak ada di .env")
}

fn format_periode() -> String {
    // Format YMD
    // Jika waktu sekarang adalah 20240625 dan sebelum jam 06:00 pada tanggal 20240626, maka ambil periode 20240625
    if chrono::Local::now().hour() < 6 {
        chrono::Local::now().format("%Y%m%d").to_string()
    } else {
        (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y%m%d")
            .to_string()
    }
}

async fn download_file(
    url: &str,
    meth: Method,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // log untuk menunjukkan bahwa proses download dimulai
    let start = Instant::now();
    println!("Mulai download file dari URL: {}", url);

    // Download file menggunakan reqwest
    let client = reqwest::Client::new();

    // ambil header dari config/heade.rs -> header
    let headers = header().await;

    // Buat request dengan method dan URL yang diberikan, serta tambahkan header
    let mut request = client.request(meth, url);
    for (key, value) in headers {
        request = request.header(&key, &value);
    }

    // Kirim request dan tunggu respons
    let response = request.send().await?;
    if response.status().is_success() {
        // Ambil content-disposition untuk mendapatkan nama file
        let nama_file = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|cd| cd.to_str().ok())
            .and_then(|cd| {
                cd.split("filename=")
                    .nth(1)
                    .map(|name| name.trim_matches('"').to_string())
            })
            .unwrap_or_else(|| "file_unduhan".to_string());

        // Download file
        let bytes = response.bytes().await?;

        // jika ukuran file kurang dari 1MB maka anggap sebagai file gagal diunduh
        if bytes.len() < 1_000_000 {
            eprintln!(
                "File yang diunduh terlalu kecil ({} bytes), gagal diunduh.",
                bytes.len()
            );
            return Err("File yang diunduh terlalu kecil, gagal diunduh.".into());
        }

        // Simpan file ke folder
        // Buat folder "PRABAK_CACHE" jika belum ada
        let folder_path = Path::new("PRABAK_CACHE");
        if !folder_path.exists() {
            std::fs::create_dir_all(folder_path)?;
        }
        std::fs::write(folder_path.join(&nama_file.trim()), &bytes)?;
        println!(
            "File berhasil di download: {}, dalam waktu {:.2?} detik",
            nama_file,
            start.elapsed()
        );
    }

    Ok(())
}

pub(crate) async fn download_hapdown() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let periode = format_periode();
    let url = format!(
        "{}historyapdown?periode={}&reg=6&witel=ALL&download=1",
        url_rsa(),
        periode
    );
    download_file(&url, Method::GET).await
}

pub(crate) async fn download_managersa() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let uname = std::env::var("UNAME").expect("UNAME tidak ada di .env");
    let url = format!(
        "{}managersa?kd=&dp=&bt=&po=&lc=&ter=6&uname={}&witel=&loc_id=&lokasi=&download=1",
        url_rsa(),
        uname
    );
    download_file(&url, Method::GET).await
}
