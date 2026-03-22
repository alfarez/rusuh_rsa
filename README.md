# RSA Telegram Bot

Bot Telegram untuk operasional RSA: download file dari server, update RSA manual via file `.txt`, proses AutoRSA berbasis data Excel/HTML table, dan manajemen jadwal piket.

## Ringkasan Fitur

- Download file `History APDown` dan `ManageRSA` dari endpoint server, lalu kirim ke Telegram.
- AutoRSA end-to-end: download data, transform dengan Polars, tentukan status, lalu POST update ke server.
- Input RSA manual via upload file `.txt` (format `witel;lc;dp;po`).
- Jadwal piket per user (`/jadwal`), list jadwal bulanan (`/listjadwal`), generate jadwal (`/generate`).
- Scheduler otomatis jam `21:00` (waktu lokal server) untuk AutoRSA admin dan download agent.
- Kompres file > 1 MB sebelum dikirim (gzip).

## Daftar Command

| Command | Akses | Fungsi |
|---|---|---|
| `/download` | User terjadwal hari ini atau admin | Unduh file RSA dari server lalu kirim ke chat |
| `/rsa` | User terjadwal hari ini atau admin | Masuk mode tunggu file `.txt`, validasi, lalu update RSA concurrent |
| `/autorsa` | Admin | Jalankan pipeline AutoRSA penuh |
| `/jadwal` | Semua user terdaftar | Tampilkan jadwal user untuk bulan berjalan |
| `/listjadwal` | Admin | Tampilkan jadwal seluruh user untuk bulan berjalan |
| `/generate` | Admin | Generate jadwal bulan berjalan dan kirim file hasil |
| `/testscheduler autorsa` | Admin | Tes trigger scheduler AutoRSA (delay 1 menit) |
| `/testscheduler download` | Admin | Tes trigger scheduler download agent (delay 1 menit) |

## Arsitektur Singkat

```text
src/
  main.rs                 # bootstrap env + start bot
  bot/
    instance.rs           # command handler, scheduler, access control
    sender.rs             # kompres file gzip sebelum kirim
  config/
    env.rs                # loader .env sederhana
    users.rs              # parsing users.json + role check
    jadwal.rs             # load/save/generate jadwal
    heade.rs              # header HTTP default
  scraper/
    ambil_data.rs         # download History APDown + ManageRSA
    proses_rsa.rs         # parse/validasi input txt + POST concurrent
    auto_rsa.rs           # transform data + hitung status + kirim update
```

## Konfigurasi

### 1) File .env

Buat `.env` di root project:

```env
TELOXIDE_TOKEN=isi_token_bot_telegram
BASE_URL_RSA=https://domain-anda/path/
URL_EDIT_RSA=https://domain-anda/path/edit-rsa
UNAME=username_anda
```

Keterangan:

- `TELOXIDE_TOKEN`: token bot Telegram (dipakai `Bot::from_env()`).
- `BASE_URL_RSA`: base URL untuk endpoint download (`historyapdown`, `managersa`).
- `URL_EDIT_RSA`: endpoint POST update RSA.
- `UNAME`: dipakai saat request `managersa`.

### 2) File users.json

Struktur harus mengikuti model di kode (`admins` + `users`):

```json
{
  "admins": [123456789],
  "users": {
    "123456789": "Nama Admin",
    "987654321": "Nama Agent"
  }
}
```

Catatan:

- Semua ID di `users` dianggap user yang diizinkan akses bot.
- User dalam `admins` mendapat hak command admin.

### 3) File jadwal.json

Saat awal boleh kosong:

```json
{}
```

Contoh isi setelah generate:

```json
{
  "2026-03-22": 987654321,
  "2026-03-23": 123456789
}
```

## Cara Menjalankan

### Opsi A: Lokal (tanpa Docker)

Prasyarat:

- Rust toolchain (project ini memakai edition `2024`).
- Library OpenSSL dev sesuai OS.

Jalankan:

```bash
cargo run --release
```

### Opsi B: Docker

Build image:

```bash
docker build -t rsa-bot .
```

Run container:

```bash
docker run -d \
  --name rsa-bot \
  --env-file .env \
  -v $(pwd)/users.json:/app/users.json \
  -v $(pwd)/jadwal.json:/app/jadwal.json \
  rsa-bot
```

## Alur Kerja Fitur

### A) /download

1. Bot memanggil dua endpoint secara paralel: `historyapdown` dan `managersa`.
2. File disimpan ke folder `PRABAK_CACHE/`.
3. Jika ukuran file > 1 MB, file dikompres ke `.gz` sebelum dikirim.
4. Semua file dikirim ke chat.
5. Folder `PRABAK_CACHE/` dibersihkan setelah proses selesai.

### B) /rsa (manual upload)

1. Kirim command `/rsa`.
2. Bot masuk mode menunggu file `.txt`.
3. Format per baris wajib: `witel;lc;dp;po`.
4. Bot validasi field kosong dan `po` harus angka.
5. Data valid diposting concurrent ke `URL_EDIT_RSA` (semaphore 10).
6. Bot kirim ringkasan sukses/gagal + file log `logs/hasil_rsa.txt`.

Contoh isi file:

```text
WITEL_A;LOC001;Partially On;3
WITEL_B;LOC002;Ocassionally;0
```

### C) /autorsa

1. Bot download data terbaru (`History APDown` + `ManageRSA`).
2. Auto pilih dua file Excel/HTML-table dari `PRABAK_CACHE/`.
3. Transform data dengan Polars:
   - hitung `TOTAL_JAM_MATI` dari kolom jam `Jxx` valid,
   - join ke data ManageRSA,
   - hitung statistik AP mati/total,
   - tentukan status `Partially On` / `Ocassionally`.
4. Simpan output ke file `RSA_YYYYMMDD.txt`.
5. Kirim update ke server concurrent (limit 10).
6. Bot kirim hasil dan file output ke Telegram, lalu bersihkan cache.

## Scheduler Otomatis

Bot menjalankan dua scheduler, cek tiap 30 detik, trigger saat jam lokal server tepat `21:00`:

- Scheduler 1: jika petugas hari itu admin, bot otomatis jalankan AutoRSA ke admin tersebut.
- Scheduler 2: jika petugas hari itu bukan admin (agent), bot otomatis download dan kirim file ke agent tersebut.

Sumber jadwal diambil dari `jadwal.json` dengan key tanggal format `YYYY-MM-DD`.

## Catatan Perilaku Penting

- Jika user tidak ada di `users.json`, bot balas: tidak diizinkan.
- `/download` dan `/rsa` ditolak jika bukan admin dan bukan jadwal user di hari ini.
- `format_periode()` di download data:
  - sebelum jam 06:00 memakai tanggal hari ini,
  - setelah itu memakai tanggal kemarin.
- Timeout total proses AutoRSA adalah 600 detik.

## Troubleshooting

- Error `users.json tidak ditemukan`: pastikan file ada di working directory saat binary dijalankan.
- Error `.env` gagal dibaca: cek file `.env` dan format `KEY=value` (tanpa spasi berlebih).
- AutoRSA gagal karena data kosong: cek hasil download, validitas sheet, dan isi kolom yang dipakai (`LOC_ID`, `RSA_TYPE`, `MINIMUM_AP`, kolom `Jxx`).
- File upload `/rsa` ditolak: pastikan ekstensi `.txt`, encoding UTF-8, dan delimiter `;` per baris 4 kolom.

## Dependensi Utama

- `teloxide` untuk Telegram bot.
- `tokio` untuk async runtime.
- `reqwest` untuk HTTP client.
- `polars` untuk transform data tabular.
- `calamine` untuk baca Excel (`.xls/.xlsx`).
- `scraper` untuk fallback parsing HTML table.
- `flate2` untuk kompresi gzip.

- Jalankan pakai file env 
- docker run --env-file .env rsa-app