use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct Users {
    pub admins: Vec<i64>,
    pub users: HashMap<String, String>,
}

impl Users {
    pub fn load() -> Self {
        let content = std::fs::read_to_string("users.json").expect("users.json tidak ditemukan");
        serde_json::from_str(&content).expect("Format JSON salah")
    }

    pub fn is_allowed(&self, user_id: i64) -> bool {
        self.users.contains_key(&user_id.to_string())
    }

    pub fn is_admin(&self, user_id: i64) -> bool {
        self.admins.contains(&user_id)
    }

    pub fn nama(&self, user_id: i64) -> String {
        self.users
            .get(&user_id.to_string())
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    pub fn all_user_ids(&self) -> Vec<i64> {
        self.users.keys().filter_map(|k| k.parse().ok()).collect()
    }
}
