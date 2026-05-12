use anyhow::{bail, Context, Result};
use data_encoding::BASE32_NOPAD;
use sha1::Digest;
use sqlx::PgPool;
use std::io::Read;
use std::path::Path;
use tracing::info;

const BASE_URL: &str = "https://assets-umamusume-en.akamaized.net";
const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];
const BSV_MAGIC: u8 = 0xBF;
const BSV_FORMAT_VERSION: u8 = 1;
const BSV_FORMAT_ANONYMOUS: u8 = 1;
const USER_AGENT: &str = "UnityPlayer/2022.3.46f1 (UnityWebRequest/1.0, libcurl/8.5.0-DEV)";
const PLATFORM: &str = "Windows";

// ---------------------------------------------------------------------------
// BSV binary format parser
// ---------------------------------------------------------------------------

enum BsvValue {
    Text(String),
    Int(u64),
}

impl BsvValue {
    fn as_str(&self) -> Result<&str> {
        match self {
            BsvValue::Text(s) => Ok(s.as_str()),
            _ => bail!("Expected string BSV value"),
        }
    }

    fn as_u64(&self) -> Result<u64> {
        match self {
            BsvValue::Int(v) => Ok(*v),
            _ => bail!("Expected integer BSV value"),
        }
    }
}

struct BsvParser<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> BsvParser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn read_vlq(&mut self) -> u64 {
        let mut value: u64 = 0;
        let mut bytes_read = 0;
        while bytes_read < 8 && self.offset < self.data.len() {
            let byte = self.data[self.offset];
            self.offset += 1;
            bytes_read += 1;
            value = (value << 7) | (byte & 0x7F) as u64;
            if byte & 0x80 == 0 {
                break;
            }
        }
        value
    }

    fn read_unum(&mut self, num_bytes: usize) -> Result<u64> {
        if self.offset + num_bytes > self.data.len() {
            bail!(
                "Unexpected end of BSV data reading {} bytes at offset {}",
                num_bytes,
                self.offset
            );
        }
        let mut value: u64 = 0;
        for i in 0..num_bytes {
            value = (value << 8) | self.data[self.offset + i] as u64;
        }
        self.offset += num_bytes;
        Ok(value)
    }

    fn read_text(&mut self) -> String {
        let start = self.offset;
        while self.offset < self.data.len() && self.data[self.offset] != 0 {
            self.offset += 1;
        }
        let text = String::from_utf8_lossy(&self.data[start..self.offset]).to_string();
        if self.offset < self.data.len() {
            self.offset += 1; // skip null terminator
        }
        text
    }

    fn read_byte(&mut self) -> Result<u8> {
        if self.offset >= self.data.len() {
            bail!("Unexpected end of BSV data");
        }
        let byte = self.data[self.offset];
        self.offset += 1;
        Ok(byte)
    }
}

fn parse_anonymous_bsv(data: &[u8]) -> Result<Vec<Vec<BsvValue>>> {
    if data.len() < 2 {
        bail!("BSV data too short");
    }
    if data[0] != BSV_MAGIC {
        bail!(
            "Invalid BSV magic: expected 0x{:02X}, got 0x{:02X}",
            BSV_MAGIC,
            data[0]
        );
    }

    let format_byte = data[1];
    let version = (format_byte >> 4) & 0x0F;
    let format_type = format_byte & 0x0F;

    if version != BSV_FORMAT_VERSION {
        bail!("Unsupported BSV version: {}", version);
    }
    if format_type != BSV_FORMAT_ANONYMOUS {
        bail!("Expected ANONYMOUS BSV format, got {}", format_type);
    }

    let mut parser = BsvParser::new(data);
    parser.offset = 2;

    parser.read_unum(2)?; // header_size
    let row_count = parser.read_vlq() as usize;
    parser.read_vlq(); // max_row_size
    parser.read_vlq(); // schema_version
    let schema_count = parser.read_vlq() as usize;

    let mut schemas: Vec<(u8, Option<usize>)> = Vec::with_capacity(schema_count);
    for _ in 0..schema_count {
        let type_byte = parser.read_byte()?;
        let fixed_size = if (type_byte.wrapping_sub(0x21) & 0xCF) == 0 && type_byte != 0x51 {
            Some(parser.read_vlq() as usize)
        } else {
            None
        };
        schemas.push((type_byte, fixed_size));
    }

    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let mut row = Vec::with_capacity(schemas.len());
        for &(type_byte, fixed_size) in &schemas {
            let base_type = type_byte & 0xF0;
            if type_byte == 0x40 || base_type == 0x40 {
                row.push(BsvValue::Text(parser.read_text()));
            } else if type_byte == 0x11
                || type_byte == 0x12
                || type_byte == 0x13
                || base_type == 0x10
            {
                row.push(BsvValue::Int(parser.read_vlq()));
            } else if let Some(size) = fixed_size {
                row.push(BsvValue::Int(parser.read_unum(size)?));
            } else {
                bail!("Unknown BSV type: 0x{:02X}", type_byte);
            }
        }
        rows.push(row);
    }

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Manifest parsing
// ---------------------------------------------------------------------------

struct ManifestEntry {
    name: String,
    #[allow(dead_code)]
    size: u64,
    #[allow(dead_code)]
    checksum: u64,
    hname: String,
}

fn calc_hname(checksum: u64, size: u64, name: &[u8]) -> String {
    let mut header = [0u8; 16];
    header[0..8].copy_from_slice(&checksum.to_be_bytes());
    header[8..16].copy_from_slice(&size.to_be_bytes());

    let mut hasher = sha1::Sha1::new();
    hasher.update(header);
    hasher.update(name);
    let hash = hasher.finalize();

    BASE32_NOPAD.encode(&hash)
}

fn parse_root_manifest(data: &[u8]) -> Result<Vec<ManifestEntry>> {
    let rows = parse_anonymous_bsv(data)?;
    let mut entries = Vec::new();
    for row in &rows {
        if row.len() >= 3 {
            let name = row[0].as_str()?.to_string();
            let size = row[1].as_u64()?;
            let checksum = row[2].as_u64()?;
            let hname = calc_hname(checksum, size, name.as_bytes());
            entries.push(ManifestEntry {
                name,
                size,
                checksum,
                hname,
            });
        }
    }
    Ok(entries)
}

fn parse_content_manifest(data: &[u8]) -> Result<Vec<ManifestEntry>> {
    let rows = parse_anonymous_bsv(data)?;
    let mut entries = Vec::new();
    for row in &rows {
        let (name, size, checksum) = if row.len() >= 7 {
            (row[0].as_str()?, row[4].as_u64()?, row[5].as_u64()?)
        } else if row.len() >= 3 {
            (row[0].as_str()?, row[1].as_u64()?, row[2].as_u64()?)
        } else {
            continue;
        };
        let name = name.to_string();
        let hname = calc_hname(checksum, size, name.as_bytes());
        entries.push(ManifestEntry {
            name,
            size,
            checksum,
            hname,
        });
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// LZ4 decompression
// ---------------------------------------------------------------------------

fn decompress_lz4(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 4 {
        bail!("Data too short for LZ4");
    }
    if data[..4] == LZ4_FRAME_MAGIC {
        let mut decoder = lz4_flex::frame::FrameDecoder::new(data);
        let mut output = Vec::new();
        decoder
            .read_to_end(&mut output)
            .context("LZ4 frame decompression failed")?;
        Ok(output)
    } else {
        let uncompressed_size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        lz4_flex::decompress(&data[4..], uncompressed_size)
            .map_err(|e| anyhow::anyhow!("LZ4 block decompression failed: {}", e))
    }
}

fn is_lz4_compressed(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == LZ4_FRAME_MAGIC
}

// ---------------------------------------------------------------------------
// HTTP download + manifest chain
// ---------------------------------------------------------------------------

fn root_manifest_url(app_ver: &str) -> String {
    format!(
        "{}/dl/vertical/{}/manifests/manifestdat/root.manifest.bsv.lz4",
        BASE_URL, app_ver
    )
}

fn manifest_url(hname: &str) -> String {
    format!(
        "{}/dl/vertical/resources/Manifest/{}/{}",
        BASE_URL,
        &hname[..2],
        hname
    )
}

fn generic_url(hname: &str) -> String {
    format!(
        "{}/dl/vertical/resources/Generic/{}/{}",
        BASE_URL,
        &hname[..2],
        hname
    )
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "*/*")
        .header("Accept-Encoding", "identity")
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .with_context(|| format!("HTTP request failed for {}", url))?;

    if !resp.status().is_success() {
        bail!("HTTP {}: {}", resp.status(), url);
    }

    Ok(resp.bytes().await?.to_vec())
}

async fn fetch_master_mdb(resource_version: &str, output_path: &str) -> Result<()> {
    let client = reqwest::Client::new();

    // Step 1: Root manifest
    info!("Fetching root manifest for version {}...", resource_version);
    let root_data = download(&client, &root_manifest_url(resource_version)).await?;
    let root_data = decompress_lz4(&root_data)?;
    let root_entries = parse_root_manifest(&root_data)?;

    let platform_entry = root_entries
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(PLATFORM))
        .context("Windows platform not found in root manifest")?;

    // Step 2: Platform manifest
    info!("Fetching {} platform manifest...", PLATFORM);
    let platform_data = download(&client, &manifest_url(&platform_entry.hname)).await?;
    let platform_data = if is_lz4_compressed(&platform_data) {
        decompress_lz4(&platform_data)?
    } else {
        platform_data
    };
    let platform_entries = parse_content_manifest(&platform_data)?;

    let master_entry = platform_entries
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case("master"))
        .context("'master' entry not found in platform manifest")?;

    // Step 3: Master manifest
    info!("Fetching master manifest...");
    let master_data = download(&client, &manifest_url(&master_entry.hname)).await?;
    let master_data = if is_lz4_compressed(&master_data) {
        decompress_lz4(&master_data)?
    } else {
        master_data
    };
    let master_entries = parse_content_manifest(&master_data)?;

    let mdb_entry = master_entries
        .iter()
        .find(|e| e.name.to_lowercase().contains("master.mdb"))
        .context("master.mdb entry not found in master manifest")?;

    // Step 4: Download and decompress master.mdb
    info!("Downloading master.mdb...");
    let mdb_compressed = download(&client, &generic_url(&mdb_entry.hname)).await?;
    let mdb_data = decompress_lz4(&mdb_compressed)?;

    if let Some(parent) = Path::new(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_path, &mdb_data)?;
    info!(
        "Saved master.mdb: {} bytes ({:.2} MB)",
        mdb_data.len(),
        mdb_data.len() as f64 / (1024.0 * 1024.0)
    );

    Ok(())
}

/// Check `master_versions` table for a newer app_version or resource_version and re-fetch
/// master.mdb if needed. Returns true if the file was updated.
pub async fn maybe_update_master_mdb(pool: &PgPool, mdb_path: &str) -> Result<bool> {
    let row: Option<(String, String)> = match sqlx::query_as(
        "SELECT app_version, resource_version FROM master_versions ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            info!(
                "Could not query master_versions (table may not exist yet): {}",
                e
            );
            return Ok(false);
        }
    };

    let Some((app_version, resource_version)) = row else {
        info!("No version in master_versions table, skipping master.mdb update");
        return Ok(false);
    };

    let combined = format!("{}:{}", app_version.trim(), resource_version.trim());

    // Compare with version marker file next to master.mdb
    let marker_path = format!("{}.version", mdb_path);
    let current = std::fs::read_to_string(&marker_path).unwrap_or_default();
    if current.trim() == combined {
        info!(
            "master.mdb is up to date (app_version: {}, resource_version: {})",
            app_version, resource_version
        );
        return Ok(false);
    }

    info!(
        "Updating master.mdb: '{}' -> '{}'",
        current.trim(),
        combined
    );
    fetch_master_mdb(&resource_version, mdb_path).await?;
    std::fs::write(&marker_path, &combined)?;

    Ok(true)
}
