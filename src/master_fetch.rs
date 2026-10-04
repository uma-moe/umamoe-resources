#![allow(non_snake_case)]

use anyhow::{bail, Context, Result};
use data_encoding::BASE32_NOPAD;
use sha1::Digest;
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::info;

#[path = "types/master_fetch.rs"]
mod types;
use types::{
    BsvParser, BsvValue, GameVersion, ManifestEntry, BASE_URL, BSV_FORMAT_ANONYMOUS,
    BSV_FORMAT_VERSION, BSV_MAGIC, LZ4_FRAME_MAGIC, PLATFORM, USER_AGENT, VERSION_URL,
};

impl BsvValue {
    fn asStr(&self) -> Result<&str> {
        match self {
            BsvValue::Text(s) => Ok(s.as_str()),
            _ => bail!("Expected string BSV value"),
        }
    }

    fn asU64(&self) -> Result<u64> {
        match self {
            BsvValue::Int(v) => Ok(*v),
            _ => bail!("Expected integer BSV value"),
        }
    }
}

impl<'a> BsvParser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn readVlq(&mut self) -> u64 {
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

    fn readUnum(&mut self, num_bytes: usize) -> Result<u64> {
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

    fn readText(&mut self) -> String {
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

    fn readByte(&mut self) -> Result<u8> {
        if self.offset >= self.data.len() {
            bail!("Unexpected end of BSV data");
        }
        let byte = self.data[self.offset];
        self.offset += 1;
        Ok(byte)
    }
}

fn parseAnonymousBsv(data: &[u8]) -> Result<Vec<Vec<BsvValue>>> {
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

    parser.readUnum(2)?; // header_size
    let row_count = parser.readVlq() as usize;
    parser.readVlq(); // max_row_size
    parser.readVlq(); // schema_version
    let schema_count = parser.readVlq() as usize;

    let mut schemas: Vec<(u8, Option<usize>)> = Vec::with_capacity(schema_count);
    for _ in 0..schema_count {
        let type_byte = parser.readByte()?;
        let fixed_size = if (type_byte.wrapping_sub(0x21) & 0xCF) == 0 && type_byte != 0x51 {
            Some(parser.readVlq() as usize)
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
                row.push(BsvValue::Text(parser.readText()));
            } else if type_byte == 0x11
                || type_byte == 0x12
                || type_byte == 0x13
                || base_type == 0x10
            {
                row.push(BsvValue::Int(parser.readVlq()));
            } else if let Some(size) = fixed_size {
                row.push(BsvValue::Int(parser.readUnum(size)?));
            } else {
                bail!("Unknown BSV type: 0x{:02X}", type_byte);
            }
        }
        rows.push(row);
    }

    Ok(rows)
}

fn calcHname(checksum: u64, size: u64, name: &[u8]) -> String {
    let mut header = [0u8; 16];
    header[0..8].copy_from_slice(&checksum.to_be_bytes());
    header[8..16].copy_from_slice(&size.to_be_bytes());

    let mut hasher = sha1::Sha1::new();
    hasher.update(header);
    hasher.update(name);
    let hash = hasher.finalize();

    BASE32_NOPAD.encode(&hash)
}

fn parseRootManifest(data: &[u8]) -> Result<Vec<ManifestEntry>> {
    let rows = parseAnonymousBsv(data)?;
    let mut entries = Vec::new();
    for row in &rows {
        if row.len() >= 3 {
            let name = row[0].asStr()?.to_string();
            let size = row[1].asU64()?;
            let checksum = row[2].asU64()?;
            let hname = calcHname(checksum, size, name.as_bytes());
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

fn parseContentManifest(data: &[u8]) -> Result<Vec<ManifestEntry>> {
    let rows = parseAnonymousBsv(data)?;
    let mut entries = Vec::new();
    for row in &rows {
        let (name, size, checksum) = if row.len() >= 7 {
            (row[0].asStr()?, row[4].asU64()?, row[5].asU64()?)
        } else if row.len() >= 3 {
            (row[0].asStr()?, row[1].asU64()?, row[2].asU64()?)
        } else {
            continue;
        };
        let name = name.to_string();
        let hname = calcHname(checksum, size, name.as_bytes());
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

fn decompressLz4(data: &[u8]) -> Result<Vec<u8>> {
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

fn isLz4Compressed(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == LZ4_FRAME_MAGIC
}

// ---------------------------------------------------------------------------
// HTTP download + manifest chain
// ---------------------------------------------------------------------------

fn rootManifestUrl(app_ver: &str) -> String {
    format!(
        "{}/dl/vertical/{}/manifests/manifestdat/root.manifest.bsv.lz4",
        BASE_URL, app_ver
    )
}

fn manifestUrl(hname: &str) -> String {
    format!(
        "{}/dl/vertical/resources/Manifest/{}/{}",
        BASE_URL,
        &hname[..2],
        hname
    )
}

fn genericUrl(hname: &str) -> String {
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

async fn fetchMasterMdb(resource_version: &str, output_path: &Path) -> Result<()> {
    let client = reqwest::Client::new();

    // Step 1: Root manifest
    info!("Fetching root manifest for version {}...", resource_version);
    let root_data = download(&client, &rootManifestUrl(resource_version)).await?;
    let root_data = decompressLz4(&root_data)?;
    let root_entries = parseRootManifest(&root_data)?;

    let platform_entry = root_entries
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(PLATFORM))
        .context("Windows platform not found in root manifest")?;

    // Step 2: Platform manifest
    info!("Fetching {} platform manifest...", PLATFORM);
    let platform_data = download(&client, &manifestUrl(&platform_entry.hname)).await?;
    let platform_data = if isLz4Compressed(&platform_data) {
        decompressLz4(&platform_data)?
    } else {
        platform_data
    };
    let platform_entries = parseContentManifest(&platform_data)?;

    let master_entry = platform_entries
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case("master"))
        .context("'master' entry not found in platform manifest")?;

    // Step 3: Master manifest
    info!("Fetching master manifest...");
    let master_data = download(&client, &manifestUrl(&master_entry.hname)).await?;
    let master_data = if isLz4Compressed(&master_data) {
        decompressLz4(&master_data)?
    } else {
        master_data
    };
    let master_entries = parseContentManifest(&master_data)?;

    let mdb_entry = master_entries
        .iter()
        .find(|e| e.name.to_lowercase().contains("master.mdb"))
        .context("master.mdb entry not found in master manifest")?;

    // Step 4: Download and decompress master.mdb
    info!("Downloading master.mdb...");
    let mdb_compressed = download(&client, &genericUrl(&mdb_entry.hname)).await?;
    let mdb_data = decompressLz4(&mdb_compressed)?;

    anyhow::ensure!(
        mdb_data.starts_with(b"SQLite format 3\0"),
        "downloaded master is not a SQLite database"
    );
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for {}",
                output_path.display()
            )
        })?;
    }
    let pending_path = output_path.with_extension("mdb.download");
    std::fs::write(&pending_path, &mdb_data)
        .with_context(|| format!("failed to write master.mdb to {}", pending_path.display()))?;
    {
        let connection = rusqlite::Connection::open_with_flags(
            &pending_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let course_count: u64 =
            connection.query_row("SELECT COUNT(*) FROM race_course_set", [], |row| row.get(0))?;
        anyhow::ensure!(course_count > 0, "downloaded master has no race courses");
    }
    std::fs::rename(&pending_path, output_path)
        .with_context(|| format!("failed to replace master at {}", output_path.display()))?;
    info!(
        "Saved master.mdb: {} bytes ({:.2} MB)",
        mdb_data.len(),
        mdb_data.len() as f64 / (1024.0 * 1024.0)
    );

    Ok(())
}

/// Check the current Global game version on every call. Failed checks are
/// errors, so an old local marker cannot be mistaken for the current version.
pub async fn maybeUpdateMasterMdb(mdb_path: &Path) -> Result<bool> {
    let version = fetchGameVersion(VERSION_URL).await?;
    let combined = versionMarker(&version)?;

    // Compare with version marker file next to master.mdb.
    let mut marker_name = mdb_path.as_os_str().to_os_string();
    marker_name.push(".version");
    let marker_path = PathBuf::from(marker_name);
    let current = std::fs::read_to_string(&marker_path).unwrap_or_default();
    if current.trim() == combined && mdb_path.is_file() {
        info!(
            "master.mdb is up to date (app_version: {}, resource_version: {})",
            version.app_version, version.resource_version
        );
        return Ok(false);
    }

    info!(
        "Updating master.mdb: '{}' -> '{}'",
        current.trim(),
        combined
    );
    fetchMasterMdb(version.resource_version.trim(), mdb_path).await?;
    std::fs::write(&marker_path, &combined).with_context(|| {
        format!(
            "failed to write master version marker to {}",
            marker_path.display()
        )
    })?;
    Ok(true)
}

async fn fetchGameVersion(url: &str) -> Result<GameVersion> {
    let version: GameVersion = reqwest::Client::new()
        .get(url)
        .header("Cache-Control", "no-cache")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .context("failed to check current Global game version")?
        .error_for_status()
        .context("Global game version endpoint returned an error")?
        .json()
        .await
        .context("invalid Global game version response")?;
    versionMarker(&version)?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn checksVersionEachTimeAndRejectsFailedOrInvalidResponses() {
        use axum::{routing::get, Json, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let app = Router::new()
            .route("/version", get(move || {
                let observed = observed.clone();
                async move {
                    let call = observed.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({"app_version": "1.35.2", "resource_version": (10008010 + call).to_string()}))
                }
            }))
            .route("/invalid", get(|| async { Json(serde_json::json!({"app_version": "", "resource_version": "../old"})) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://{address}/version");
        assert_eq!(
            versionMarker(&fetchGameVersion(&url).await.unwrap()).unwrap(),
            "1.35.2:10008010"
        );
        assert_eq!(
            versionMarker(&fetchGameVersion(&url).await.unwrap()).unwrap(),
            "1.35.2:10008011"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(fetchGameVersion(&format!("http://{address}/invalid"))
            .await
            .is_err());
        assert!(fetchGameVersion(&format!("http://{address}/missing"))
            .await
            .is_err());
        server.abort();
    }
}

fn versionMarker(version: &GameVersion) -> Result<String> {
    let app_version = version.app_version.trim();
    let resource_version = version.resource_version.trim();
    anyhow::ensure!(
        app_version.split('.').count() == 3
            && app_version
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            && !resource_version.is_empty()
            && resource_version.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid current Global game version"
    );
    Ok(format!("{app_version}:{resource_version}"))
}
