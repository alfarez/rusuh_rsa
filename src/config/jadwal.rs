use chrono::{Datelike, NaiveDate};
use rand::rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::config::users::Users;

#[derive(Serialize, Deserialize, Clone)]
pub struct Jadwal {
    pub data: HashMap<String, i64>,
}

impl Jadwal {
    pub fn load(users: &Users) -> Self {
        let content = std::fs::read_to_string("jadwal.json").unwrap_or_else(|_| "{}".to_string());

        // format baru per-bulan: {"YYYY-MM": {"YYYY-MM-DD": "Nama"}}
        if let Ok(data_flat) = serde_json::from_str::<HashMap<String, i64>>(&content) {
            return Self { data: data_flat };
        }

        let monthly =
            serde_json::from_str::<HashMap<String, HashMap<String, String>>>(&content).unwrap();

        let nama_to_uid: HashMap<String, i64> = users
            .users
            .iter()
            .filter_map(|(uid, nama)| uid.parse::<i64>().ok().map(|id| (nama.clone(), id)))
            .collect();

        let mut data = HashMap::new();
        for (_, days) in monthly {
            for (tanggal, nama) in days {
                if nama.eq_ignore_ascii_case("none") {
                    continue;
                }

                if let Some(uid) = nama_to_uid.get(&nama) {
                    data.insert(tanggal, *uid);
                }
            }
        }

        Self { data }
    }

    pub fn save(&self, users: &Users) {
        let mut grouped: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

        for (tanggal, uid) in &self.data {
            if tanggal.len() < 7 {
                continue;
            }

            let bulan_key = tanggal[..7].to_string();
            let nama = users.nama(*uid);
            grouped
                .entry(bulan_key)
                .or_default()
                .insert(tanggal.clone(), nama);
        }

        let content = serde_json::to_string_pretty(&grouped).unwrap();
        std::fs::write("jadwal.json", content).unwrap();
    }

    pub fn boleh_akses(&self, user_id: i64) -> bool {
        let hari_ini = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.petugas_di_tanggal(&hari_ini) == Some(user_id)
    }

    pub fn petugas_di_tanggal(&self, tanggal: &str) -> Option<i64> {
        self.data.get(tanggal).copied()
    }

    pub fn entri_bulan(&self, tahun: i32, bulan: u32) -> Vec<(String, i64)> {
        let prefix = format!("{}-{:02}", tahun, bulan);
        let mut entries: Vec<(String, i64)> = self
            .data
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        entries.sort_by_key(|(tgl, _)| tgl.clone());
        entries
    }

    pub fn hari_dalam_bulan(tahun: i32, bulan: u32) -> u32 {
        NaiveDate::from_ymd_opt(tahun, bulan + 1, 1)
            .unwrap_or(NaiveDate::from_ymd_opt(tahun + 1, 1, 1).unwrap())
            .pred_opt()
            .unwrap()
            .day()
    }

    pub fn generate(
        &mut self,
        tahun: i32,
        bulan: u32,
        user_ids: &[i64],
        admin_id: i64,
        users: &Users,
    ) -> String {
        let total_hari = Self::hari_dalam_bulan(tahun, bulan);
        let total_user = user_ids.len();
        let hari_per_user = total_hari as usize / total_user;
        let sisa = total_hari as usize % total_user;

        // Buat pool: setiap user muncul hari_per_user kali
        let mut pool: Vec<i64> = user_ids
            .iter()
            .flat_map(|&uid| std::iter::repeat_n(uid, hari_per_user))
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

        self.save(users);

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
