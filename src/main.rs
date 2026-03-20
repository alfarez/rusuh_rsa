mod bot;
mod config;
mod scraper;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    config::env::load_env();
    // Jalankan bot Telegram
    bot::instance::jalankan_bot().await;
    Ok(())
}
