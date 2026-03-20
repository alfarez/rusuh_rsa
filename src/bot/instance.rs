use crate::bot::sender::compress_file;
use crate::scraper::ambil_data::{download_hapdown, download_managersa};
use crate::scraper::proses_rsa::{edit_rsa_concurrent, parse_input};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::{prelude::*, types::InputFile};
use tokio::sync::Mutex;

// State: simpan chat_id yang sedang menunggu file .txt
type WaitingSet = Arc<Mutex<HashSet<i64>>>;

fn load_allowed_users() -> Vec<i64> {
    let content =
        std::fs::read_to_string("allowed_users.json").expect("allowed_users.json tidak ditemukan");
    serde_json::from_str(&content).expect("Format JSON salah")
}

pub async fn jalankan_bot() {
    let bot = Bot::from_env();
    let waiting: WaitingSet = Arc::new(Mutex::new(HashSet::new()));
    let allowed_user_ids: Arc<Vec<i64>> = Arc::new(load_allowed_users());

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let waiting = Arc::clone(&waiting);
        let allowed_user_ids = Arc::clone(&allowed_user_ids);
        async move {
            if !allowed_user_ids.contains(&msg.chat.id.0) {
                let _ = bot
                    .send_message(msg.chat.id, "❌ Anda tidak diizinkan.")
                    .await;
                return Ok(());
            }

            // Handler /download
            if msg.text() == Some("/download") {
                handle_download(&bot, &msg).await?;
                return Ok(());
            }

            // Handler /rsa — minta file
            if msg.text() == Some("/rsa") {
                let _ = bot
                    .send_message(
                        msg.chat.id,
                        "📎 Silakan kirim file .txt berisi data RSA.\nFormat per baris:\nwitel;lc;dp;po",
                    )
                    .await;
                waiting.lock().await.insert(msg.chat.id.0);
                return Ok(());
            }

            // Handler file .txt (jika sedang menunggu)
            if let Some(doc) = msg.document() {
                let sedang_tunggu = waiting.lock().await.contains(&msg.chat.id.0);
                if !sedang_tunggu {
                    return Ok(());
                }

                // Validasi ekstensi
                let nama = doc.file_name.clone().unwrap_or_default();
                if !nama.ends_with(".txt") {
                    let _ = bot
                        .send_message(msg.chat.id, "❌ File harus berformat .txt")
                        .await;
                    return Ok(());
                }

                // Download file dari Telegram
                let file = bot.get_file(doc.file.id.clone()).await?;
                let mut buf = Vec::new();
                bot.download_file(&file.path, &mut buf).await?;

                let teks = match String::from_utf8(buf) {
                    Ok(t) => t,
                    Err(_) => {
                        let _ = bot
                            .send_message(msg.chat.id, "❌ File tidak valid (bukan UTF-8).")
                            .await;
                        return Ok(());
                    }
                };

                // Parse dan validasi
                let data_list = parse_input(&teks);
                if data_list.is_empty() {
                    let _ = bot
                        .send_message(
                            msg.chat.id,
                            "❌ Tidak ada data valid.\nPastikan format: witel;lc;dp;po per baris",
                        )
                        .await;
                    return Ok(());
                }

                // Hapus dari waiting
                waiting.lock().await.remove(&msg.chat.id.0);

                let _ = bot
                    .send_message(
                        msg.chat.id,
                        format!("Memproses {} baris RSA...", data_list.len()),
                    )
                    .await;

                // Proses concurrent
                let (hasil, log_path) = edit_rsa_concurrent(data_list).await;

                let sukses = hasil.iter().filter(|(_, ok, _)| *ok).count();
                let gagal = hasil.len() - sukses;

                let _ = bot
                    .send_message(
                        msg.chat.id,
                        format!("✅ Selesai!\nSukses: {}\nGagal: {}", sukses, gagal),
                    )
                    .await;

                // Kirim file debug
                let _ = bot
                .send_document(msg.chat.id, InputFile::file(&log_path)).await;

                //Hapus file dan folder log setelah dikirim
                let _ = std::fs::remove_file(&log_path);
                let _ = std::fs::remove_dir("logs");
            }

            Ok(())
        }
    })
    .await;
}

// Pisah handler download biar tidak numpuk di repl
async fn handle_download(bot: &Bot, msg: &Message) -> Result<(), teloxide::RequestError> {
    let _ = bot
        .send_message(msg.chat.id, "Mulai download file...")
        .await;

    let (hasil_hap, hasil_mgr) = tokio::join!(download_hapdown(), download_managersa());
    let sukses = hasil_hap.is_ok() && hasil_mgr.is_ok();

    let pesan = match (hasil_hap, hasil_mgr) {
        (Ok(_), Ok(_)) => "File berhasil diunduh!".to_string(),
        (Err(e), _) => format!("History APDown error: {}", e),
        (_, Err(e)) => format!("ManageRSA error: {}", e),
    };

    let _ = bot.send_message(msg.chat.id, pesan).await;

    if sukses {
        let files: Vec<_> = std::fs::read_dir("PRABAK_CACHE")
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();

        let total = files.len();
        if total == 0 {
            let _ = bot.send_message(msg.chat.id, "⚠️ Folder kosong!").await;
            return Ok(());
        }

        let progress_id = bot
            .send_message(msg.chat.id, format!("Kompres & kirim {} file...", total))
            .await
            .unwrap()
            .id;

        for (i, file) in files.iter().enumerate() {
            let path = file.path();
            let size_mb = std::fs::metadata(&path).unwrap().len() / 1_000_000;

            let kirim_path: PathBuf = if size_mb > 1 {
                match compress_file(&path) {
                    Ok(p) => {
                        let hasil_mb = std::fs::metadata(&p).unwrap().len() / 1_000_000;
                        let _ = bot
                            .send_message(msg.chat.id, format!("{} MB → {} MB", size_mb, hasil_mb))
                            .await;
                        p
                    }
                    Err(e) => {
                        let _ = bot
                            .send_message(msg.chat.id, format!("❌ Gagal kompres: {}", e))
                            .await;
                        continue;
                    }
                }
            } else {
                path.clone()
            };

            match bot
                .send_document(msg.chat.id, InputFile::file(&kirim_path))
                .await
            {
                Ok(_) => println!("Terkirim: {:?}", kirim_path),
                Err(e) => println!("Gagal kirim: {}", e),
            }

            if kirim_path != path {
                let _ = std::fs::remove_file(&kirim_path);
            }

            let _ = bot
                .edit_message_text(
                    msg.chat.id,
                    progress_id,
                    format!("{}/{} file terkirim...", i + 1, total),
                )
                .await;
        }

        let _ = bot
            .edit_message_text(msg.chat.id, progress_id, "✅ Semua file terkirim!")
            .await;

        // Bersihkan folder
        for file in std::fs::read_dir("PRABAK_CACHE").unwrap().flatten() {
            let _ = std::fs::remove_file(file.path());
        }
        let _ = std::fs::remove_dir("PRABAK_CACHE");
    }

    Ok(())
}
