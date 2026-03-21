use crate::bot::sender::compress_file;
use crate::config::jadwal::Jadwal;
use crate::config::users::Users;
use crate::scraper::ambil_data::{download_hapdown, download_managersa};
use crate::scraper::proses_rsa::{edit_rsa_concurrent, parse_input, validasi_input};
use chrono::Datelike;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::{prelude::*, types::InputFile};
use tokio::sync::Mutex;

type WaitingSet = Arc<Mutex<HashSet<i64>>>;

pub async fn jalankan_bot() {
    let bot = Bot::from_env();
    let waiting: WaitingSet = Arc::new(Mutex::new(HashSet::new()));
    let users: Arc<Users> = Arc::new(Users::load());
    let jadwal: Arc<Mutex<Jadwal>> = Arc::new(Mutex::new(Jadwal::load()));

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let waiting = Arc::clone(&waiting);
        let users = Arc::clone(&users);
        let jadwal = Arc::clone(&jadwal);

        async move {
            let uid = msg.chat.id.0;

            // Cek allowed
            if !users.is_allowed(uid) {
                let _ = bot
                    .send_message(msg.chat.id, "❌ Anda tidak diizinkan.")
                    .await;
                return Ok(());
            }

            let nama = users.nama(uid);
            let teks = msg.text().unwrap_or("");

            // Generate jadwal — pakai all_user_ids()
            if teks == "/generate" && users.is_admin(uid) {
                let now = chrono::Local::now();
                let user_ids = users.all_user_ids();
                let log = jadwal
                    .lock()
                    .await
                    .generate(now.year(), now.month(), &user_ids, uid);

                let path = PathBuf::from("jadwal_generated.txt");
                std::fs::write(&path, &log).unwrap();
                let _ = bot.send_document(msg.chat.id, InputFile::file(&path)).await;
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }

            if teks == "/listjadwal" && users.is_admin(uid) {
                let now = chrono::Local::now();
                // Tampilkan nama bukan user_id
                let jadwal_lock = jadwal.lock().await;
                let prefix = format!("{}-{:02}", now.year(), now.month());
                let mut entries: Vec<_> = jadwal_lock.data
                    .iter()
                    .filter(|(k, _)| k.starts_with(&prefix))
                    .collect();
                entries.sort_by_key(|(k, _)| k.to_string());

                let isi = entries
                    .iter()
                    .map(|(tgl, uid)| {
                        let nama = users.nama(**uid);
                        format!("{}: {}", tgl, nama)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                let _ = bot.send_message(msg.chat.id, isi).await;
                return Ok(());
            }

            if teks == "/jadwal" {
                let now = chrono::Local::now();
                let isi = jadwal.lock().await.jadwal_user(uid, now.year(), now.month());
                let _ = bot
                    .send_message(
                        msg.chat.id,
                        format!("📅 Jadwal {} bulan ini:\n{}", nama, isi),
                    )
                    .await;
                return Ok(());
            }

            if teks == "/download" || teks == "/rsa" {
                let boleh = jadwal.lock().await.boleh_akses(uid);
                if !boleh {
                    let now = chrono::Local::now();
                    let jadwal_user = jadwal.lock().await.jadwal_user(uid, now.year(), now.month());
                    let _ = bot
                        .send_message(
                            msg.chat.id,
                            format!("⏰ Bukan jadwal kamu {}.\n📅 Jadwalmu:\n{}", nama, jadwal_user),
                        )
                        .await;
                    return Ok(());
                }
            }

            // ... sisa handler /download, /rsa, file .txt sama seperti sebelumnya
            if teks == "/download" {
                handle_download(&bot, &msg).await?;
                return Ok(());
            }

            if teks == "/rsa" {
                let _ = bot
                    .send_message(
                        msg.chat.id,
                        "📎 Silakan kirim file .txt berisi data RSA.\nFormat per baris:\nwitel;lc;dp;po",
                    )
                    .await;
                waiting.lock().await.insert(uid);
                return Ok(());
            }

            if let Some(doc) = msg.document() {
                let sedang_tunggu = waiting.lock().await.contains(&uid);
                if !sedang_tunggu {
                    return Ok(());
                }

                let nama_file = doc.file_name.clone().unwrap_or_default();
                if !nama_file.ends_with(".txt") {
                    let _ = bot
                        .send_message(msg.chat.id, "❌ File harus berformat .txt")
                        .await;
                    return Ok(());
                }

                let file = bot.get_file(doc.file.id.clone()).await?;
                let mut buf = Vec::new();
                bot.download_file(&file.path, &mut buf).await?;

                let teks_file = match String::from_utf8(buf) {
                    Ok(t) => t,
                    Err(_) => {
                        let _ = bot
                            .send_message(msg.chat.id, "❌ File tidak valid (bukan UTF-8).")
                            .await;
                        return Ok(());
                    }
                };

                // setelah parse_input
                let data_list = parse_input(&teks_file);
                if data_list.is_empty() {
                    let _ = bot
                        .send_message(
                            msg.chat.id,
                            "❌ Tidak ada data valid.\nPastikan format: witel;lc;dp;po per baris",
                        )
                        .await;
                    return Ok(());
                }

                // Validasi sebelum kirim ke server
                let errors = validasi_input(&data_list);
                if !errors.is_empty() {
                    let pesan_error = errors
                        .iter()
                        .map(|e| format!("Baris {}: {} → {}", e.baris, e.data, e.alasan))
                        .collect::<Vec<_>>()
                        .join("\n");

                    let _ = bot
                        .send_message(
                            msg.chat.id,
                            format!("❌ Data tidak valid, perbaiki dulu:\n\n{}", pesan_error),
                        )
                        .await;
                    return Ok(());
                }

                waiting.lock().await.remove(&uid);

                let _ = bot
                    .send_message(
                        msg.chat.id,
                        format!("⚙️ Memproses {} baris RSA...", data_list.len()),
                    )
                    .await;

                let (hasil, log_path) = edit_rsa_concurrent(data_list).await;
                let sukses = hasil.iter().filter(|(_, ok, _)| *ok).count();
                let gagal = hasil.len() - sukses;

                let _ = bot
                    .send_message(
                        msg.chat.id,
                        format!("✅ Selesai!\nSukses: {}\nGagal: {}", sukses, gagal),
                    )
                    .await;

                let _ = bot.send_document(msg.chat.id, InputFile::file(&log_path)).await;
                let _ = std::fs::remove_file(&log_path);
                let _ = std::fs::remove_dir("logs");
            }

            Ok(())
        }
    })
    .await;
}
async fn handle_download(bot: &Bot, msg: &Message) -> Result<(), teloxide::RequestError> {
    let _ = bot
        .send_message(msg.chat.id, "Mulai download file...")
        .await;

    let (hasil_hap, hasil_mgr) = tokio::join!(download_hapdown(), download_managersa());
    let sukses = hasil_hap.is_ok() && hasil_mgr.is_ok();

    let pesan = match (hasil_hap, hasil_mgr) {
        (Ok(_), Ok(_)) => "✅ File RSA berhasil diunduh, silakan cek file!".to_string(),
        (Err(e), _) => format!("❌ Gagal unduh History APDown: {}", e),
        (_, Err(e)) => format!("❌ Gagal unduh ManageRSA: {}", e),
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
                            .send_message(
                                msg.chat.id,
                                format!("🗜 {} MB → {} MB", size_mb, hasil_mb),
                            )
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

        for file in std::fs::read_dir("PRABAK_CACHE").unwrap().flatten() {
            let _ = std::fs::remove_file(file.path());
        }
        let _ = std::fs::remove_dir("PRABAK_CACHE");
    }

    Ok(())
}
