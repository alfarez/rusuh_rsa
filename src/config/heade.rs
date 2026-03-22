use std::collections::HashMap;
use std::env;

pub async fn header() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    headers.insert("User-Agent".to_string(), "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36".to_string());
    headers.insert("Connection".to_string(), "keep-alive".to_string());

    if let Ok(cookie) = env::var("RSA_COOKIE") {
        if !cookie.trim().is_empty() {
            headers.insert("Cookie".to_string(), cookie);
        }
    }

    headers
}
