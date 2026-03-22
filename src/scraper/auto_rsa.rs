use crate::config::heade::header;
use calamine::{Reader, open_workbook_auto};
use polars::prelude::*;
use scraper::{Html, Selector};
use std::ops::Not;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

const CONCURRENT_POST_LIMIT: usize = 10;
const SUCCESS_MESSAGE: &str = "Data Berhasil di update..";

type BoxError = Box<dyn std::error::Error + Send + Sync>;

fn html_table_to_dataframe(bytes: &[u8], path: &str) -> Result<DataFrame, BoxError> {
    let html = String::from_utf8_lossy(bytes);
    let doc = Html::parse_document(&html);
    let row_sel = Selector::parse("table tr").map_err(|e| format!("Selector error: {}", e))?;
    let cell_sel = Selector::parse("th, td").map_err(|e| format!("Selector error: {}", e))?;

    let mut rows: Vec<Vec<String>> = Vec::new();
    for tr in doc.select(&row_sel) {
        let row: Vec<String> = tr
            .select(&cell_sel)
            .map(|cell| cell.text().collect::<String>().trim().to_string())
            .collect();
        if !row.is_empty() {
            rows.push(row);
        }
    }

    if rows.is_empty() {
        return Err(format!(
            "Xls error: Tidak ada tabel HTML yang bisa dibaca di {}",
            path
        )
        .into());
    }

    let headers = rows.remove(0);
    if headers.is_empty() {
        return Err(format!("Xls error: Header tabel kosong di {}", path).into());
    }

    let mut columns: Vec<Vec<String>> = vec![Vec::new(); headers.len()];
    for row in rows {
        for (i, col_vals) in columns.iter_mut().enumerate() {
            let val = row.get(i).cloned().unwrap_or_default();
            col_vals.push(val);
        }
    }

    let height = columns.first().map(|c| c.len()).unwrap_or(0);
    let cols: Vec<Column> = headers
        .iter()
        .zip(columns.iter())
        .map(|(name, vals)| Column::new(name.as_str().into(), vals.clone()))
        .collect();

    Ok(DataFrame::new(height, cols)?)
}

//  Load XLS ke DataFrame
fn xls_to_dataframe(path: &str) -> Result<DataFrame, BoxError> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 8 {
        return Err(format!("Xls error: File terlalu kecil/korup: {}", path).into());
    }

    let is_ole = bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    let is_zip = bytes.starts_with(b"PK\x03\x04"); // xlsx
    let is_html = bytes.starts_with(b"<!DOCTYPE html")
        || bytes.starts_with(b"<html")
        || bytes.starts_with(b"<HTML")
        || bytes.starts_with(b"<table")
        || bytes.starts_with(b"<TABLE");

    if is_html {
        return html_table_to_dataframe(&bytes, path);
    }

    if !is_ole && !is_zip {
        return Err(format!(
            "Xls error: Signature file '{}' bukan format Excel valid",
            path
        )
        .into());
    }

    let mut workbook = open_workbook_auto(path)?;

    let sheet = workbook.worksheet_range_at(0).ok_or("Sheet kosong")??;

    let mut rows = sheet.rows();

    let headers: Vec<String> = match rows.next() {
        Some(row) => row.iter().map(|c| c.to_string()).collect(),
        None => return Err("File kosong".into()),
    };

    let mut columns: Vec<Vec<String>> = vec![Vec::new(); headers.len()];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < columns.len() {
                let cell_str = cell.to_string();
                columns[i].push(cell_str);
            }
        }
    }

    let height = columns.first().map(|c| c.len()).unwrap_or(0);

    // Polars 0.53: DataFrame::new(height, Vec<Column>)
    let cols: Vec<Column> = headers
        .iter()
        .zip(columns.iter())
        .map(|(name, vals)| Column::new(name.as_str().into(), vals.clone()))
        .collect();

    Ok(DataFrame::new(height, cols)?)
}

//  Hitung jam mati
fn hitung_jam_mati(df: &mut DataFrame) -> Result<Vec<String>, PolarsError> {
    let candidate_jam_cols: Vec<String> = df
        .get_column_names()
        .iter()
        .filter(|name| {
            let s = name.as_str();
            s.len() == 3 && s.starts_with('J') && s[1..].parse::<u32>().is_ok()
        })
        .map(|s| s.to_string())
        .collect();

    let mut jam_cols = Vec::new();
    for col_name in candidate_jam_cols {
        let series = match df.column(&col_name) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let s = match series.cast(&polars::datatypes::DataType::Float64) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let s = match s.f64() {
            Ok(v) => v,
            Err(_) => continue,
        };

        if s.into_iter().flatten().any(|v| (1.0..=4.0).contains(&v)) {
            jam_cols.push(col_name);
        }
    }

    if jam_cols.is_empty() {
        return Err(PolarsError::ComputeError(
            "Tidak ada kolom jam valid (Jxx) dengan nilai 1..4".into(),
        ));
    }

    let len = jam_cols.len() as f64;
    let sum_expr = jam_cols
        .iter()
        .map(|c| {
            col(c.as_str())
                .cast(polars::datatypes::DataType::Float64)
                .fill_null(lit(0.0))
        })
        .reduce(|a, b| a + b)
        .unwrap_or(lit(0.0));

    let result = df
        .clone()
        .lazy()
        .with_column((sum_expr / lit(len)).alias("TOTAL_JAM_MATI"))
        .collect()?;

    *df = result;
    Ok(jam_cols)
}

//  Merge + bersihkan
fn gabung_dan_bersihkan(
    df_rsa: &DataFrame,
    df_manage: &DataFrame,
    jam_cols: &[String],
) -> Result<DataFrame, PolarsError> {
    let mut rsa_cols = vec![
        "LOC_ID".to_string(),
        "WITEL".to_string(),
        "TOTAL_JAM_MATI".to_string(),
        "MAC_ADDRESS".to_string(),
    ];
    rsa_cols.extend_from_slice(jam_cols);

    let df_rsa_sel = df_rsa.select(&rsa_cols)?;
    let df_manage_sel = df_manage.select(["LOC_ID", "RSA_TYPE", "MINIMUM_AP"])?;

    let df_merge = df_rsa_sel.join(
        &df_manage_sel,
        ["LOC_ID"],
        ["LOC_ID"],
        JoinArgs::new(JoinType::Left),
        None,
    )?;

    // Samakan perilaku dengan Python: drop_duplicates(subset="MAC_ADDRESS").
    let df_merge = df_merge
        .lazy()
        .unique_stable(Some(cols(["MAC_ADDRESS"])), UniqueKeepStrategy::First)
        .with_column(
            col("MINIMUM_AP")
                .cast(polars::datatypes::DataType::Int64)
                .fill_null(lit(0))
                .alias("MINIMUM_AP"),
        )
        .collect()?;

    // Samakan dengan Python: buang LOC_ID yang mengandung '_' (termasuk null).
    let mask_no_underscore = df_merge
        .column("LOC_ID")?
        .str()?
        .contains("_", false)?
        .not();

    let df_merge = df_merge.filter(&mask_no_underscore)?;

    // Buang LOC_ID kosong/null supaya output tidak jadi "WITEL;;...".
    let loc_series = df_merge
        .column("LOC_ID")?
        .cast(&polars::datatypes::DataType::String)?;
    let loc_utf8 = loc_series.str()?;
    let non_empty_mask: BooleanChunked = loc_utf8
        .into_iter()
        .map(|opt| opt.map(|s| !s.trim().is_empty()))
        .collect();

    Ok(df_merge.filter(&non_empty_mask)?)
}

//  Hitung status AP ─
fn hitung_status_ap(df_merge: &DataFrame) -> Result<DataFrame, PolarsError> {
    let mask_mati = df_merge
        .column("TOTAL_JAM_MATI")?
        .cast(&polars::datatypes::DataType::Float64)?
        .f64()?
        .gt_eq(3.0);

    let df_mati = df_merge.filter(&mask_mati)?;

    let total = df_merge
        .clone()
        .lazy()
        .group_by([col("LOC_ID")])
        .agg([col("LOC_ID").count().alias("TOTAL_AP")])
        .collect()?;

    let mati = df_mati
        .clone()
        .lazy()
        .group_by([col("LOC_ID")])
        .agg([col("LOC_ID").count().alias("AP_MATI")])
        .collect()?;

    let stats = total
        .join(
            &mati,
            ["LOC_ID"],
            ["LOC_ID"],
            JoinArgs::new(JoinType::Left),
            None,
        )?
        .lazy()
        .with_column(col("AP_MATI").fill_null(lit(0)).alias("AP_MATI"))
        .with_column((col("TOTAL_AP") - col("AP_MATI")).alias("JUMLAH"))
        .with_column(
            (col("JUMLAH").cast(polars::datatypes::DataType::Float64)
                / col("TOTAL_AP").cast(polars::datatypes::DataType::Float64)
                * lit(100.0))
            .alias("PERSENTASE"),
        )
        .collect()?;

    Ok(stats)
}

//  Build data akhir ─
fn bangun_data_akhir(
    df_merge: &DataFrame,
    stats: &DataFrame,
) -> Result<Vec<(String, String, String, String)>, PolarsError> {
    let df = df_merge
        .join(
            stats,
            ["LOC_ID"],
            ["LOC_ID"],
            JoinArgs::new(JoinType::Left),
            None,
        )?
        .lazy();

    let df_partially = df
        .clone()
        .filter(
            col("TOTAL_AP")
                .neq(col("JUMLAH"))
                .and(col("JUMLAH").neq(lit(0)))
                .and(
                    col("MINIMUM_AP")
                        .cast(polars::datatypes::DataType::Int64)
                        .neq(col("JUMLAH").cast(polars::datatypes::DataType::Int64)),
                ),
        )
        .with_column(lit("Partially On").alias("RSA"))
        .collect()?;
    println!("[AutoRSA] Kandidat Partially On: {}", df_partially.height());

    let df_ocass = df
        .filter(
            col("TOTAL_JAM_MATI")
                .cast(polars::datatypes::DataType::Float64)
                .gt_eq(lit(3.0))
                .and(
                    col("RSA_TYPE")
                        .cast(polars::datatypes::DataType::String)
                        .fill_null(lit(""))
                        .neq(lit("Ocassionally")),
                )
                .and(col("JUMLAH").eq(lit(0))),
        )
        .with_column(lit("Ocassionally").alias("RSA"))
        .collect()?;
    println!("[AutoRSA] Kandidat Ocassionally: {}", df_ocass.height());

    // Samakan dengan Python: drop_duplicates(subset="LOC_ID").
    let df_akhir = concat([df_ocass.lazy(), df_partially.lazy()], UnionArgs::default())?
        .filter(
            col("LOC_ID")
                .cast(polars::datatypes::DataType::String)
                .neq(lit("")),
        )
        .group_by([col("LOC_ID")])
        .agg([
            col("WITEL").first().alias("WITEL"),
            col("RSA").first().alias("RSA"),
            col("JUMLAH").first().alias("JUMLAH"),
        ])
        .select([col("WITEL"), col("LOC_ID"), col("RSA"), col("JUMLAH")])
        .collect()?;
    println!(
        "[AutoRSA] Kandidat akhir unik LOC_ID: {}",
        df_akhir.height()
    );

    let witel = df_akhir.column("WITEL")?.str()?;
    let loc_id = df_akhir.column("LOC_ID")?.str()?;
    let rsa = df_akhir.column("RSA")?.str()?;
    let jumlah = df_akhir.column("JUMLAH")?;

    let mut result = Vec::new();
    for i in 0..df_akhir.height() {
        result.push((
            witel.get(i).unwrap_or("").to_string(),
            loc_id.get(i).unwrap_or("").to_string(),
            rsa.get(i).unwrap_or("").to_string(),
            jumlah.get(i).map(|v| v.to_string()).unwrap_or_default(),
        ));
    }

    Ok(result)
}

//  Kirim ke server
async fn kirim_semua_rsa(data_list: Vec<(String, String, String, String)>) -> (usize, usize, f64) {
    let total = data_list.len();
    println!("[AutoRSA] Mulai kirim {} request...", total);

    let semaphore = Arc::new(Semaphore::new(CONCURRENT_POST_LIMIT));
    let client = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap(),
    );
    let url = std::env::var("URL_EDIT_RSA").expect("URL_EDIT_RSA tidak ada di .env");
    let start = std::time::Instant::now();

    let tasks: Vec<_> = data_list
        .into_iter()
        .map(|(witel, lc, dp, po)| {
            let client = Arc::clone(&client);
            let semaphore = Arc::clone(&semaphore);
            let url = url.clone();
            tokio::spawn(async move {
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
                for (k, v) in &headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                match req.form(&params).send().await {
                    Ok(resp) => resp.text().await.unwrap_or_default(),
                    Err(e) => e.to_string(),
                }
            })
        })
        .collect();

    let mut sukses = 0;
    let mut gagal = 0;
    for (i, task) in tasks.into_iter().enumerate() {
        match task.await {
            Ok(text) if text.contains(SUCCESS_MESSAGE) => sukses += 1,
            _ => gagal += 1,
        }

        let done = i + 1;
        if done % 25 == 0 || done == total {
            println!(
                "[AutoRSA] Progress kirim: {}/{} | sukses={} gagal={}",
                done, total, sukses, gagal
            );
        }
    }

    (sukses, gagal, start.elapsed().as_secs_f64())
}

//  Entry point
pub struct AutoRsaResult {
    pub sukses: usize,
    pub gagal: usize,
    pub total_waktu: f64,
    pub output_path: PathBuf,
}

pub async fn process_auto_rsa(
    path_hapdown: &str,
    path_managersa: &str,
) -> Result<AutoRsaResult, BoxError> {
    let total_start = std::time::Instant::now();
    println!(
        "[AutoRSA] Start. input_hap='{}', input_mgr='{}'",
        path_hapdown, path_managersa
    );

    let path_hap = path_hapdown.to_string();
    let path_mgr = path_managersa.to_string();

    let stage_load = std::time::Instant::now();
    let (mut df_rsa, df_manage) = tokio::task::spawn_blocking(move || {
        let df_rsa = xls_to_dataframe(&path_hap)?;
        let df_manage = xls_to_dataframe(&path_mgr)?;
        Ok::<_, BoxError>((df_rsa, df_manage))
    })
    .await??;
    println!(
        "[AutoRSA] Load file selesai dalam {:.2}s",
        stage_load.elapsed().as_secs_f64()
    );

    let stage_transform = std::time::Instant::now();
    println!("[AutoRSA] Baris awal RSA: {}", df_rsa.height());
    println!("[AutoRSA] Baris awal ManageRSA: {}", df_manage.height());
    let jam_cols = hitung_jam_mati(&mut df_rsa)?;
    println!("[AutoRSA] Kolom jam valid: {}", jam_cols.len());
    let df_merge = gabung_dan_bersihkan(&df_rsa, &df_manage, &jam_cols)?;
    println!(
        "[AutoRSA] Baris setelah merge + bersih: {}",
        df_merge.height()
    );
    let stats = hitung_status_ap(&df_merge)?;
    println!("[AutoRSA] Baris stats LOC_ID: {}", stats.height());
    let data_list = bangun_data_akhir(&df_merge, &stats)?;
    println!(
        "[AutoRSA] Transform selesai dalam {:.2}s, data siap kirim={} baris",
        stage_transform.elapsed().as_secs_f64(),
        data_list.len()
    );

    if data_list.is_empty() {
        return Err("AutoRSA: hasil transform kosong, tidak ada data yang memenuhi kriteria Partially On/Ocassionally".into());
    }

    let tanggal = chrono::Local::now().format("%Y%m%d").to_string();
    let output_path = PathBuf::from(format!("RSA_{}.txt", tanggal));
    let isi: Vec<String> = data_list
        .iter()
        .map(|(w, l, r, j)| format!("{};{};{};{}", w, l, r, j))
        .collect();
    std::fs::write(&output_path, isi.join("\n"))?;
    println!("[AutoRSA] Output disimpan: {:?}", output_path);

    let stage_kirim = std::time::Instant::now();
    let (sukses, gagal, total_waktu) = kirim_semua_rsa(data_list).await;
    println!(
        "[AutoRSA] Kirim selesai dalam {:.2}s | sukses={} gagal={}",
        stage_kirim.elapsed().as_secs_f64(),
        sukses,
        gagal
    );
    println!(
        "[AutoRSA] Total proses selesai dalam {:.2}s",
        total_start.elapsed().as_secs_f64()
    );

    Ok(AutoRsaResult {
        sukses,
        gagal,
        total_waktu,
        output_path,
    })
}
