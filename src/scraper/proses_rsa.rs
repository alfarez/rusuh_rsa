use crate::config::heade::header;
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;

const SUCCESS_MESSAGE: &str = "Data Berhasil di update..";
const DEFAULT_SEMAPHORE: usize = 10;

pub type RsaRow = (String, String, String, String); // witel, lc, dp, po
pub type RsaResult = (usize, bool, String);
pub struct ValidasiError {
    pub baris: usize,
    pub data: String,
    pub alasan: String,
}

#[derive(Deserialize)]
struct RsaResponse {
    message: Option<String>,
    transaction: Option<bool>,
}

pub fn validasi_input(data_list: &[RsaRow]) -> Vec<ValidasiError> {
    let mut errors = Vec::new();

    for (idx, (witel, lc, dp, po)) in data_list.iter().enumerate() {
        let baris = idx + 1;

        if witel.is_empty() {
            errors.push(ValidasiError {
                baris,
                data: format!("{};{};{};{}", witel, lc, dp, po),
                alasan: "witel kosong".to_string(),
            });
        }
        if lc.is_empty() {
            errors.push(ValidasiError {
                baris,
                data: format!("{};{};{};{}", witel, lc, dp, po),
                alasan: "lc kosong".to_string(),
            });
        }
        if dp.is_empty() {
            errors.push(ValidasiError {
                baris,
                data: format!("{};{};{};{}", witel, lc, dp, po),
                alasan: "dp kosong".to_string(),
            });
        }
        if po.is_empty() {
            errors.push(ValidasiError {
                baris,
                data: format!("{};{};{};{}", witel, lc, dp, po),
                alasan: "po kosong".to_string(),
            });
        }
        if po.parse::<i64>().is_err() {
            errors.push(ValidasiError {
                baris,
                data: format!("{};{};{};{}", witel, lc, dp, po),
                alasan: format!("po '{}' bukan angka", po),
            });
        }
    }

    errors
}

async fn edit_rsa_bulk(
    client: Arc<reqwest::Client>,
    semaphore: Arc<Semaphore>,
    url: String,
    witel: String,
    lc: String,
    dp: String,
    po: String,
) -> (bool, String) {
    let _permit = semaphore.acquire().await.unwrap();
    let headers = header().await;

    let params = [
        ("dp", dp.as_str()),
        ("po", po.as_str()),
        ("lc", lc.as_str()),
        ("witel", witel.as_str()),
        ("loc_id", lc.as_str()),
    ];

    let mut req = client.post(&url);
    for (key, value) in &headers {
        req = req.header(key.as_str(), value.as_str());
    }

    match req.form(&params).send().await {
        Ok(resp) => match resp.text().await {
            Ok(text) => {
                let sukses = serde_json::from_str::<RsaResponse>(&text)
                    .map(|body| {
                        body.message.as_deref() == Some(SUCCESS_MESSAGE)
                            && body.transaction == Some(true)
                    })
                    .unwrap_or(false);
                (sukses, text)
            }
            Err(e) => (false, e.to_string()),
        },
        Err(e) => (false, e.to_string()),
    }
}

pub async fn edit_rsa_concurrent(data_list: Vec<RsaRow>) -> (Vec<RsaResult>, PathBuf) {
    let semaphore = Arc::new(Semaphore::new(DEFAULT_SEMAPHORE));
    let client = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap(),
    );
    let url = std::env::var("URL_EDIT_RSA").expect("URL_EDIT_RSA tidak ada di .env");

    let tasks: Vec<_> = data_list
        .into_iter()
        .enumerate()
        .map(|(idx, (witel, lc, dp, po))| {
            let client = Arc::clone(&client);
            let semaphore = Arc::clone(&semaphore);
            let url = url.clone();
            tokio::spawn(async move {
                let hasil = edit_rsa_bulk(client, semaphore, url, witel, lc, dp, po).await;
                (idx + 1, hasil.0, hasil.1)
            })
        })
        .collect();

    let mut output = Vec::new();
    let mut log_lines = Vec::new();
    for task in tasks {
        match task.await {
            Ok((idx, sukses, pesan)) => {
                output.push((idx, sukses, pesan.clone()));
                log_lines.push(format!("Baris {}: {}", idx, pesan));
            }
            Err(e) => output.push((0, false, e.to_string())),
        }
    }
    // Simpan log ke file dan taruh di path "logs/hasil_rsa.txt"
    if !std::path::Path::new("logs").exists() {
        std::fs::create_dir("logs").unwrap();
    }
    let log_path = PathBuf::from("logs/hasil_rsa.txt");
    std::fs::write(&log_path, log_lines.join("\n")).unwrap();

    (output, log_path)
}

// Parse teks dari user: "witel;lc;dp;po" per baris
pub fn parse_input(text: &str) -> Vec<RsaRow> {
    text.lines()
        .filter_map(|baris| {
            let parts: Vec<&str> = baris.split(';').collect();
            if parts.len() == 4 {
                Some((
                    parts[0].trim().to_string(),
                    parts[1].trim().to_string(),
                    parts[2].trim().to_string(),
                    parts[3].trim().to_string(),
                ))
            } else {
                None
            }
        })
        .collect()
}
