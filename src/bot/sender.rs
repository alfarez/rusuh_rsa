use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub fn compress_file(input: &Path) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let output_path = input.with_extension(format!(
        "{}.gz",
        input.extension().unwrap_or_default().to_str().unwrap_or("")
    ));

    let mut input_file = File::open(input)?;
    let output_file = File::create(&output_path)?;
    let mut encoder = GzEncoder::new(output_file, Compression::new(9));

    let mut buffer = [0u8; 65536];
    loop {
        let n = input_file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        encoder.write_all(&buffer[..n])?;
    }

    encoder.finish()?;

    Ok(output_path)
}
