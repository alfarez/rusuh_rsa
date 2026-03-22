use crate::bot::sender::compress_file;
use crate::config::jadwal::Jadwal;
use crate::config::users::Users;
use crate::scraper::ambil_data::{download_hapdown, download_managersa};
use crate::scraper::proses_rsa::{edit_rsa_concurrent, parse_input, validasi_input};
use chrono::{Datelike, Timelike};
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::{prelude::*, types::InputFile};
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep, timeout};

type WaitingSet = Arc<Mutex<HashSet<i64>>>;
const AUTORSA_TOTAL_TIMEOUT_SECS: u64 = 600;

fn bersihkan_prabak_cache() {
    let dir = Path::new("PRABAK_CACHE");
    if !dir.exists() {
        return;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let _ = std::fs::remove_dir(dir);
}

fn pilih_path_autorsa() -> Result<(String, String), String> {
    let dir = Path::new("PRABAK_CACHE");
    if !dir.exists() {
        return Err("Folder PRABAK_CACHE belum ada".to_string());
    }

    let mut xls_files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Gagal baca PRABAK_CACHE: {}", e))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    let ext = ext.to_ascii_lowercase();
                    ext == "xls" || ext == "xlsx"
                })
                .unwrap_or(false)
        })
        .collect();

    if xls_files.is_empty() {
        return Err("Tidak ada file .xls/.xlsx di PRABAK_CACHE".to_string());
    }

    xls_files.sort();

    let mut hap_candidates: Vec<PathBuf> = xls_files
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|name| {
                    if !(name.ends_with(".xls") || name.ends_with(".xlsx")) {
                        return false;
                    }

                    let no_ext = name
                        .trim_end_matches(".xls")
                        .trim_end_matches(".xlsx")
                        .trim();

                    // Terima pola "Data_YYYYMMDD" dan "Data YYYYMMDD".
                    let tail = no_ext
                        .strip_prefix("Data_")
                        .or_else(|| no_ext.strip_prefix("Data "));

                    tail.map(|s| s.len() == 8 && s.chars().all(|c| c.is_ascii_digit()))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    hap_candidates.sort();
    let hap = hap_candidates
        .last()
        .cloned()
        .ok_or_else(|| "File APDown (Data_YYYYMMDD.xls) tidak ditemukan".to_string())?;

    let mgr_exact = xls_files.iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| {
                let no_ext = n.trim_end_matches(".xls").trim_end_matches(".xlsx").trim();
                no_ext == "Data_" || no_ext == "Data"
            })
            .unwrap_or(false)
    });

    let mgr = if let Some(p) = mgr_exact {
        p.clone()
    } else {
        xls_files
            .iter()
            .find(|p| **p != hap)
            .cloned()
            .ok_or_else(|| "File ManageRSA tidak ditemukan".to_string())?
    };

    Ok((
        hap.to_string_lossy().to_string(),
        mgr.to_string_lossy().to_string(),
    ))
}

async fn jalankan_autorsa_ke_chat(bot: &Bot, users: &Users, chat_id: ChatId, sumber: &str) {
    if !users.is_admin(chat_id.0) {
        println!(
            "[AutoRSA] Skip kirim notifikasi/proses karena target {} bukan admin",
            chat_id.0
        );
        return;
    }

    let _ = bot
        .send_message(chat_id, format!("⚙️ Memulai AutoRSA ({})...", sumber))
        .await;

    // Selalu ambil file terbaru agar tidak bergantung pada cache lama.
    let (hasil_hap, hasil_mgr) = tokio::join!(download_hapdown(), download_managersa());
    if let Err(e) = hasil_hap {
        let _ = bot
            .send_message(chat_id, format!("❌ Gagal unduh History APDown: {}", e))
            .await;
        bersihkan_prabak_cache();
        return;
    }
    if let Err(e) = hasil_mgr {
        let _ = bot
            .send_message(chat_id, format!("❌ Gagal unduh ManageRSA: {}", e))
            .await;
        bersihkan_prabak_cache();
        return;
    }

    let (path_hap, path_mgr) = match pilih_path_autorsa() {
        Ok(paths) => paths,
        Err(e) => {
            let _ = bot
                .send_message(chat_id, format!("❌ AutoRSA gagal menyiapkan file: {}", e))
                .await;
            bersihkan_prabak_cache();
            return;
        }
    };

    match timeout(
        Duration::from_secs(AUTORSA_TOTAL_TIMEOUT_SECS),
        crate::scraper::auto_rsa::process_auto_rsa(&path_hap, &path_mgr),
    )
    .await
    {
        Ok(Ok(hasil)) => {
            let _ = bot
                .send_message(
                    chat_id,
                    format!(
                        "✅ AutoRSA Selesai!\n📊 Total request: {}\n✅ Sukses: {}\n❌ Gagal: {}\n⏱️ Waktu: {:.2}s",
                        hasil.sukses + hasil.gagal,
                        hasil.sukses,
                        hasil.gagal,
                        hasil.total_waktu
                    ),
                )
                .await;

            let _ = bot
                .send_document(chat_id, InputFile::file(&hasil.output_path))
                .await;

            let _ = std::fs::remove_file(&hasil.output_path);
        }
        Ok(Err(e)) => {
            let _ = bot
                .send_message(chat_id, format!("❌ AutoRSA gagal: {}", e))
                .await;
        }
        Err(_) => {
            let _ = bot
                .send_message(
                    chat_id,
                    format!(
                        "⏰ AutoRSA timeout: proses melebihi {} menit",
                        AUTORSA_TOTAL_TIMEOUT_SECS / 60
                    ),
                )
                .await;
        }
    }

    bersihkan_prabak_cache();
}

async fn download_dan_kirim_ke_chat(bot: &Bot, chat_id: ChatId, sumber: &str) {
    let _ = bot
        .send_message(
            chat_id,
            format!("📥 Mulai download file RSA ({})...", sumber),
        )
        .await;

    let (hasil_hap, hasil_mgr) = tokio::join!(download_hapdown(), download_managersa());
    let sukses = hasil_hap.is_ok() && hasil_mgr.is_ok();

    let pesan = match (hasil_hap, hasil_mgr) {
        (Ok(_), Ok(_)) => "✅ File RSA berhasil diunduh, sedang dikirim...".to_string(),
        (Err(e), _) => format!("❌ Gagal unduh History APDown: {}", e),
        (_, Err(e)) => format!("❌ Gagal unduh ManageRSA: {}", e),
    };

    let _ = bot.send_message(chat_id, pesan).await;

    if !sukses {
        bersihkan_prabak_cache();
        return;
    }

    let files: Vec<_> = match std::fs::read_dir("PRABAK_CACHE") {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            let _ = bot
                .send_message(chat_id, format!("❌ Gagal baca PRABAK_CACHE: {}", e))
                .await;
            bersihkan_prabak_cache();
            return;
        }
    };

    let total = files.len();
    if total == 0 {
        let _ = bot.send_message(chat_id, "⚠️ Folder kosong!").await;
        bersihkan_prabak_cache();
        return;
    }

    let progress_id = match bot
        .send_message(chat_id, format!("Kompres & kirim {} file...", total))
        .await
    {
        Ok(msg) => Some(msg.id),
        Err(_) => None,
    };

    for (i, file) in files.iter().enumerate() {
        let path = file.path();
        let size_mb = std::fs::metadata(&path)
            .map(|m| m.len() / 1_000_000)
            .unwrap_or(0);

        let kirim_path: PathBuf = if size_mb > 1 {
            match compress_file(&path) {
                Ok(p) => {
                    let hasil_mb = std::fs::metadata(&p)
                        .map(|m| m.len() / 1_000_000)
                        .unwrap_or(0);
                    let _ = bot
                        .send_message(
                            chat_id,
                            format!("Compressed dari {} MB -> {} MB", size_mb, hasil_mb),
                        )
                        .await;
                    p
                }
                Err(e) => {
                    let _ = bot
                        .send_message(chat_id, format!("❌ Gagal kompres: {}", e))
                        .await;
                    continue;
                }
            }
        } else {
            path.clone()
        };

        let _ = bot
            .send_document(chat_id, InputFile::file(&kirim_path))
            .await;

        if kirim_path != path {
            let _ = std::fs::remove_file(&kirim_path);
        }

        if let Some(progress_id) = progress_id {
            let _ = bot
                .edit_message_text(
                    chat_id,
                    progress_id,
                    format!("{}/{} file terkirim...", i + 1, total),
                )
                .await;
        }
    }

    if let Some(progress_id) = progress_id {
        let _ = bot
            .edit_message_text(chat_id, progress_id, "✅ Semua file terkirim!")
            .await;
    } else {
        let _ = bot.send_message(chat_id, "✅ Semua file terkirim!").await;
    }

    bersihkan_prabak_cache();
}

async fn scheduler_autorsa_admin_harian(bot: Bot, users: Arc<Users>, jadwal: Arc<Mutex<Jadwal>>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    let mut diproses_hari: Option<String> = None;

    loop {
        ticker.tick().await;

        let now = chrono::Local::now();
        if now.hour() != 21 {
            continue;
        }

        let hari_ini = now.format("%Y-%m-%d").to_string();
        if diproses_hari.as_deref() == Some(&hari_ini) {
            continue;
        }

        let petugas_hari_ini = {
            let j = jadwal.lock().await;
            j.petugas_di_tanggal(&hari_ini)
        };

        let Some(user_id_petugas) = petugas_hari_ini else {
            println!(
                "[AutoRSA Scheduler] Skip: tidak ada jadwal untuk {}",
                hari_ini
            );
            diproses_hari = Some(hari_ini);
            continue;
        };

        if !users.is_admin(user_id_petugas) {
            println!(
                "[AutoRSA Scheduler] Skip: petugas {} bukan admin (agent)",
                user_id_petugas
            );
            diproses_hari = Some(hari_ini);
            continue;
        }

        println!(
            "[AutoRSA Scheduler] Menjalankan AutoRSA jam 21:00 untuk admin {}",
            user_id_petugas
        );
        diproses_hari = Some(hari_ini);
        jalankan_autorsa_ke_chat(
            &bot,
            users.as_ref(),
            ChatId(user_id_petugas),
            "Scheduler 21:00",
        )
        .await;
    }
}

async fn scheduler_download_agent_harian(bot: Bot, users: Arc<Users>, jadwal: Arc<Mutex<Jadwal>>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    let mut diproses_hari: Option<String> = None;

    loop {
        ticker.tick().await;

        let now = chrono::Local::now();
        if now.hour() != 21 {
            continue;
        }

        let hari_ini = now.format("%Y-%m-%d").to_string();
        if diproses_hari.as_deref() == Some(&hari_ini) {
            continue;
        }

        let petugas_hari_ini = {
            let j = jadwal.lock().await;
            j.petugas_di_tanggal(&hari_ini)
        };

        let Some(user_id_petugas) = petugas_hari_ini else {
            println!(
                "[Download Scheduler] Skip: tidak ada jadwal untuk {}",
                hari_ini
            );
            diproses_hari = Some(hari_ini);
            continue;
        };

        if users.is_admin(user_id_petugas) {
            println!(
                "[Download Scheduler] Skip: petugas {} adalah admin, bukan agent",
                user_id_petugas
            );
            diproses_hari = Some(hari_ini);
            continue;
        }

        println!(
            "[Download Scheduler] Kirim file otomatis jam 21:00 ke agent {}",
            user_id_petugas
        );

        diproses_hari = Some(hari_ini);
        let chat_id = ChatId(user_id_petugas);
        let _ = bot
            .send_message(
                chat_id,
                "🔔 Pengingat jadwal: bot akan otomatis download dan kirim file RSA untuk kamu malam ini.",
            )
            .await;
        download_dan_kirim_ke_chat(&bot, chat_id, "Scheduler 21:00").await;
    }
}

async fn jalankan_tes_scheduler_1_menit(
    bot: Bot,
    users: Arc<Users>,
    jadwal: Arc<Mutex<Jadwal>>,
    requester_chat: ChatId,
    mode: &str,
) {
    sleep(Duration::from_secs(60)).await;

    let hari_ini = chrono::Local::now().format("%Y-%m-%d").to_string();
    let petugas_hari_ini = {
        let j = jadwal.lock().await;
        j.petugas_di_tanggal(&hari_ini)
    };

    let Some(user_id_petugas) = petugas_hari_ini else {
        let _ = bot
            .send_message(
                requester_chat,
                format!(
                    "❌ Tes scheduler batal: tidak ada jadwal untuk {}",
                    hari_ini
                ),
            )
            .await;
        return;
    };

    match mode {
        "autorsa" => {
            if !users.is_admin(user_id_petugas) {
                let _ = bot
                    .send_message(
                        requester_chat,
                        format!(
                            "❌ Tes AutoRSA skip: petugas hari ini ({}) bukan admin.",
                            user_id_petugas
                        ),
                    )
                    .await;
                return;
            }

            let target = ChatId(user_id_petugas);
            let _ = bot
                .send_message(
                    requester_chat,
                    format!(
                        "🧪 Menjalankan tes AutoRSA sekarang ke admin terjadwal: {}",
                        user_id_petugas
                    ),
                )
                .await;
            jalankan_autorsa_ke_chat(&bot, users.as_ref(), target, "Tes Scheduler +1 Menit").await;
            let _ = bot
                .send_message(requester_chat, "✅ Tes AutoRSA selesai dipicu.")
                .await;
        }
        "download" => {
            if users.is_admin(user_id_petugas) {
                let _ = bot
                    .send_message(
                        requester_chat,
                        format!(
                            "❌ Tes Download skip: petugas hari ini ({}) adalah admin, bukan agent.",
                            user_id_petugas
                        ),
                    )
                    .await;
                return;
            }

            let target = ChatId(user_id_petugas);
            let _ = bot
                .send_message(
                    requester_chat,
                    format!(
                        "🧪 Menjalankan tes download sekarang ke agent terjadwal: {}",
                        user_id_petugas
                    ),
                )
                .await;
            let _ = bot
                .send_message(
                    target,
                    "🔔 Ini tes scheduler: bot akan otomatis download dan kirim file RSA.",
                )
                .await;
            download_dan_kirim_ke_chat(&bot, target, "Tes Scheduler +1 Menit").await;
            let _ = bot
                .send_message(requester_chat, "✅ Tes download agent selesai dipicu.")
                .await;
        }
        _ => {
            let _ = bot
                .send_message(
                    requester_chat,
                    "❌ Mode tes tidak dikenal. Pakai: /testscheduler autorsa atau /testscheduler download",
                )
                .await;
        }
    }
}

pub async fn jalankan_bot() {
    let bot = Bot::from_env();
    let waiting: WaitingSet = Arc::new(Mutex::new(HashSet::new()));
    let users: Arc<Users> = Arc::new(Users::load());
    let jadwal: Arc<Mutex<Jadwal>> = Arc::new(Mutex::new(Jadwal::load(users.as_ref())));

    let bot_scheduler = bot.clone();
    let users_scheduler = Arc::clone(&users);
    let jadwal_scheduler = Arc::clone(&jadwal);
    tokio::spawn(async move {
        scheduler_autorsa_admin_harian(bot_scheduler, users_scheduler, jadwal_scheduler).await;
    });

    let bot_scheduler_download = bot.clone();
    let users_scheduler_download = Arc::clone(&users);
    let jadwal_scheduler_download = Arc::clone(&jadwal);
    tokio::spawn(async move {
        scheduler_download_agent_harian(
            bot_scheduler_download,
            users_scheduler_download,
            jadwal_scheduler_download,
        )
        .await;
    });

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
                    .generate(now.year(), now.month(), &user_ids, uid, users.as_ref());

                let path = PathBuf::from("jadwal_generated.txt");
                std::fs::write(&path, &log).unwrap();
                let _ = bot.send_document(msg.chat.id, InputFile::file(&path)).await;
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }

            if teks == "/autorsa" {
                // Hanya admin yang bisa akses
                if !users.is_admin(uid) {
                    let _ = bot.send_message(msg.chat.id, "❌ Hanya admin yang bisa akses perintah ini.").await;
                    return Ok(());
                }

                jalankan_autorsa_ke_chat(&bot, users.as_ref(), msg.chat.id, "Manual").await;

                return Ok(());
            }

            if teks.starts_with("/testscheduler") {
                if !users.is_admin(uid) {
                    let _ = bot
                        .send_message(msg.chat.id, "❌ Hanya admin yang bisa akses command ini.")
                        .await;
                    return Ok(());
                }

                let mode = teks
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("autorsa")
                    .to_ascii_lowercase();

                let _ = bot
                    .send_message(
                        msg.chat.id,
                        format!(
                            "🧪 Tes scheduler mode '{}' dijadwalkan 1 menit lagi.",
                            mode
                        ),
                    )
                    .await;

                let bot_tes = bot.clone();
                let users_tes = Arc::clone(&users);
                let jadwal_tes = Arc::clone(&jadwal);
                let requester = msg.chat.id;
                tokio::spawn(async move {
                    jalankan_tes_scheduler_1_menit(
                        bot_tes,
                        users_tes,
                        jadwal_tes,
                        requester,
                        mode.as_str(),
                    )
                    .await;
                });

                return Ok(());
            }

            if teks == "/listjadwal" && users.is_admin(uid) {
                let now = chrono::Local::now();
                let jadwal_lock = jadwal.lock().await;
                let entries = jadwal_lock.entri_bulan(now.year(), now.month());

                let isi = entries
                    .iter()
                    .map(|(tgl, uid)| {
                        let nama = users.nama(*uid);
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
                let boleh = jadwal.lock().await.boleh_akses(uid) || users.is_admin(uid);
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
    download_dan_kirim_ke_chat(bot, msg.chat.id, "Manual").await;
    Ok(())
}
