use chrono::{Datelike, NaiveDate};
use rand::rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct Jadwal {
    pub data: HashMap<String, i64>,
}

impl Jadwal {
    pub fn load() -> Self {
        let content = std::fs::read_to_string("jadwal.json").unwrap_or_else(|_| "{}".to_string());
        let data = serde_json::from_str(&content).unwrap_or_default();
        Self { data }
    }

    pub fn save(&self) {
        let content = serde_json::to_string_pretty(&self.data).unwrap();
        std::fs::write("jadwal.json", content).unwrap();
    }

    pub fn boleh_akses(&self, user_id: i64) -> bool {
        let hari_ini = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.data.get(&hari_ini) == Some(&user_id)
    }

    pub fn hari_dalam_bulan(tahun: i32, bulan: u32) -> u32 {
        NaiveDate::from_ymd_opt(tahun, bulan + 1, 1)
            .unwrap_or(NaiveDate::from_ymd_opt(tahun + 1, 1, 1).unwrap())
            .pred_opt()
            .unwrap()
            .day()
    }

    pub fn generate(&mut self, tahun: i32, bulan: u32, user_ids: &[i64], admin_id: i64) -> String {
        let total_hari = Self::hari_dalam_bulan(tahun, bulan);
        let total_user = user_ids.len();
        let hari_per_user = total_hari as usize / total_user;
        let sisa = total_hari as usize % total_user;

        // Buat pool: setiap user muncul hari_per_user kali
        let mut pool: Vec<i64> = user_ids
            .iter()
            .flat_map(|&uid| std::iter::repeat(uid).take(hari_per_user))
            .collect();

        // Sisa hari → admin
        for _ in 0..sisa {
            pool.push(admin_id);
        }

        // Acak
        pool.shuffle(&mut rng());

        let mut log = Vec::new();
        for (idx, user) in pool.iter().enumerate() {
            let hari = idx as u32 + 1;
            let tgl = NaiveDate::from_ymd_opt(tahun, bulan, hari).unwrap();
            let key = tgl.format("%Y-%m-%d").to_string();
            self.data.insert(key.clone(), *user);
            log.push(format!("{}: {}", key, user));
        }

        self.save();

        log.push(String::new());
        log.push(format!("Total hari  : {}", total_hari));
        log.push(format!("Per agent   : {} hari", hari_per_user));
        log.push(format!("Sisa (admin): {} hari", sisa));

        log.join("\n")
    }

    pub fn jadwal_user(&self, user_id: i64, tahun: i32, bulan: u32) -> String {
        let prefix = format!("{}-{:02}", tahun, bulan);
        let mut entries: Vec<_> = self
            .data
            .iter()
            .filter(|(k, v)| k.starts_with(&prefix) && **v == user_id)
            .collect();

        entries.sort_by_key(|(k, _)| k.to_string());

        if entries.is_empty() {
            return "Tidak ada jadwal bulan ini.".to_string();
        }

        entries
            .iter()
            .map(|(tgl, _)| tgl.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
