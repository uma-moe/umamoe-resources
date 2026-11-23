//! Reads the game's asset CDN: file lists (manifests), downloads and bundle decryption.

#![allow(non_snake_case)]

use anyhow::{bail, Context, Result};
use data_encoding::{BASE32_NOPAD, BASE64};
use sha1::Digest;
use std::io::Read;

#[path = "types/game_cdn.rs"]
mod types;
use types::{
    BsvParser, BsvValue, ASSET_BUNDLE_HEADER_SIZE, BSV_FORMAT_ANONYMOUS, BSV_FORMAT_VERSION,
    BSV_MAGIC, BSV_RAW_U64, LZ4_FRAME_MAGIC, ROOT_MANIFEST_NAME, USER_AGENT,
};
pub use types::{CdnSource, ManifestEntry, ANDROID, GLOBAL_CDN, JP_CDN, WINDOWS};

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
            } else if type_byte == BSV_RAW_U64 {
                row.push(BsvValue::Int(parser.readUnum(8)?));
            } else if type_byte == 0x11 || type_byte == 0x12 || base_type == 0x10 {
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

impl ManifestEntry {
    /// A Windows entry as the CDN lists it; its download name derives from checksum, size and path.
    pub fn new(name: &str, size: u64, checksum: u64, key: u64) -> Self {
        Self {
            name: name.to_string(),
            size,
            checksum,
            key,
            dependencies: Vec::new(),
            platform: WINDOWS,
            hname: calcHname(checksum, size, name.as_bytes()),
        }
    }
}

fn parseRootManifest(data: &[u8]) -> Result<Vec<ManifestEntry>> {
    let rows = parseAnonymousBsv(data)?;
    let mut entries = Vec::new();
    for row in &rows {
        if row.len() >= 3 {
            entries.push(ManifestEntry::new(
                row[0].asStr()?,
                row[1].asU64()?,
                row[2].asU64()?,
                0,
            ));
        }
    }
    Ok(entries)
}

fn parseContentManifest(data: &[u8]) -> Result<Vec<ManifestEntry>> {
    let rows = parseAnonymousBsv(data)?;
    let mut entries = Vec::new();
    for row in &rows {
        let entry = if row.len() >= 7 {
            ManifestEntry {
                dependencies: listedDependencies(&row[1]),
                ..ManifestEntry::new(
                    row[0].asStr()?,
                    row[4].asU64()?,
                    row[5].asU64()?,
                    row[6].asU64()?,
                )
            }
        } else if row.len() >= 3 {
            ManifestEntry::new(row[0].asStr()?, row[1].asU64()?, row[2].asU64()?, 0)
        } else {
            continue;
        };
        entries.push(entry);
    }
    Ok(entries)
}

/// The dependency column: bundle names separated by `;`, empty for a bundle that needs none.
fn listedDependencies(column: &BsvValue) -> Vec<String> {
    let Ok(names) = column.asStr() else {
        return Vec::new();
    };
    names
        .split(';')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn decompressLz4(data: &[u8]) -> Result<Vec<u8>> {
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

/// Where a resource version's root manifest is. A Global version is a number that names the
/// folder holding it. A JP version is `number:token`, and the CDN stores the root manifest like
/// any other manifest, under the name its size and checksum in the token hash to.
fn rootManifestUrl(source: &CdnSource, resource_version: &str) -> Result<String> {
    if !source.hashed_root {
        return Ok(format!(
            "{}/{}/manifests/manifestdat/root.manifest.bsv.lz4",
            source.base_url, resource_version
        ));
    }

    let (_, token) = resource_version.split_once(':').with_context(|| {
        format!(
            "{} resource version {resource_version} has no root manifest token",
            source.region
        )
    })?;
    let (size, checksum) = rootManifestIdentity(token)
        .with_context(|| format!("invalid resource version {resource_version}"))?;
    let hname = calcHname(checksum, size, ROOT_MANIFEST_NAME.as_bytes());
    Ok(resourceUrl(source, "Manifest", &hname))
}

/// The root manifest's size and checksum from a version token: base64 of the size, written like
/// a manifest's numbers, then the checksum's eight bytes. Only one-byte sizes have been seen.
fn rootManifestIdentity(token: &str) -> Result<(u64, u64)> {
    let bytes = BASE64
        .decode(token.as_bytes())
        .context("the version token is not base64")?;
    let mut parser = BsvParser::new(&bytes);
    let size = parser.readVlq();
    let checksum = parser.readUnum(8)?;
    if parser.offset != bytes.len() {
        bail!("the version token has bytes after its checksum");
    }
    Ok((size, checksum))
}

fn resourceUrl(source: &CdnSource, folder: &str, hname: &str) -> String {
    format!(
        "{}/resources/{}/{}/{}",
        source.base_url,
        folder,
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

/// The entries a manifest lists, of the same platform as the manifest.
async fn fetchManifest(
    client: &reqwest::Client,
    source: &CdnSource,
    entry: &ManifestEntry,
) -> Result<Vec<ManifestEntry>> {
    let data = download(client, &resourceUrl(source, "Manifest", &entry.hname)).await?;
    let data = if isLz4Compressed(&data) {
        decompressLz4(&data)?
    } else {
        data
    };
    let mut entries =
        parseContentManifest(&data).with_context(|| format!("invalid manifest {}", entry.name))?;
    for listed in &mut entries {
        listed.platform = entry.platform;
    }
    Ok(entries)
}

/// The platform's category manifests for one resource version: master, chara, outgame and so on.
pub async fn fetchCategories(
    client: &reqwest::Client,
    source: &CdnSource,
    platform: &'static str,
    resource_version: &str,
) -> Result<Vec<ManifestEntry>> {
    let root_data = download(client, &rootManifestUrl(source, resource_version)?).await?;
    let root_entries = parseRootManifest(&decompressLz4(&root_data)?)?;
    let platform_entry = root_entries
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(platform))
        .with_context(|| format!("{platform} platform not found in root manifest"))?;

    let platform_entry = ManifestEntry {
        platform,
        ..platform_entry.clone()
    };
    fetchManifest(client, source, &platform_entry).await
}

/// Every file the CDN lists in the named categories.
pub async fn fetchCategoryFiles(
    client: &reqwest::Client,
    source: &CdnSource,
    categories: &[ManifestEntry],
    names: &[&str],
) -> Result<Vec<ManifestEntry>> {
    let mut files = Vec::new();
    for name in names {
        let category = categories
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
            .with_context(|| format!("'{name}' category not found in platform manifest"))?;
        files.extend(fetchManifest(client, source, category).await?);
    }
    Ok(files)
}

/// A file from the `Generic` folder, such as the compressed master database.
pub async fn downloadGeneric(
    client: &reqwest::Client,
    source: &CdnSource,
    entry: &ManifestEntry,
) -> Result<Vec<u8>> {
    download(client, &resourceUrl(source, "Generic", &entry.hname)).await
}

/// An asset bundle exactly as the CDN serves it for its platform, still encrypted.
pub async fn downloadAssetBundle(
    client: &reqwest::Client,
    source: &CdnSource,
    entry: &ManifestEntry,
) -> Result<Vec<u8>> {
    let folder = format!("{}/assetbundles", entry.platform);
    let bundle = download(client, &resourceUrl(source, &folder, &entry.hname)).await?;
    if bundle.len() as u64 != entry.size {
        bail!(
            "{} is {} bytes, the manifest lists {}",
            entry.name,
            bundle.len(),
            entry.size
        );
    }
    Ok(bundle)
}

/// XORs everything after the clear header with the base key spread over the file's own key.
pub fn decryptAssetBundle(bundle: &mut [u8], base_key: &[u8], file_key: u64) {
    if file_key == 0 || base_key.is_empty() || bundle.len() <= ASSET_BUNDLE_HEADER_SIZE {
        return;
    }

    let file_key_bytes = file_key.to_le_bytes();
    let key: Vec<u8> = base_key
        .iter()
        .flat_map(|base| file_key_bytes.iter().map(move |part| base ^ part))
        .collect();
    for (index, byte) in bundle.iter_mut().enumerate().skip(ASSET_BUNDLE_HEADER_SIZE) {
        *byte ^= key[index % key.len()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vlq(value: u8) -> u8 {
        assert!(value < 0x80);
        value
    }

    #[test]
    fn globalVersionsNameTheFolderOfTheirRootManifest() {
        assert_eq!(
            rootManifestUrl(&GLOBAL_CDN, "10008010").unwrap(),
            "https://assets-umamusume-en.akamaized.net/dl/vertical/10008010/manifests/manifestdat/root.manifest.bsv.lz4"
        );
    }

    #[test]
    fn jpVersionsNameTheirRootManifestBySizeAndChecksum() {
        // A live JP version; its root manifest is 77 bytes and was served under this name.
        assert_eq!(
            rootManifestIdentity("TZH99sxhsgYr").unwrap(),
            (77, 0x91FD_F6CC_61B2_062B)
        );
        assert_eq!(
            rootManifestUrl(&JP_CDN, "10021450:TZH99sxhsgYr").unwrap(),
            "https://prd-storage-game-umamusume.akamaized.net/dl/resources/Manifest/7R/7RFTYUPGQLGCONV3YBUA3KTCBCF7T4EB"
        );
    }

    #[test]
    fn malformedVersionTokensAreRejected() {
        // No token, not base64, too short for a checksum, and bytes left over after it.
        for version in [
            "10021450",
            "10021450:%%%",
            "10021450:TZH9",
            "10021450:TZH99sxhsgYrAAAA",
        ] {
            assert!(rootManifestUrl(&JP_CDN, version).is_err(), "{version}");
        }
    }

    #[test]
    fn fileKeyColumnIsReadAsEightRawBytes() {
        // Two rows of (name, key): a VLQ read of the key would leave bytes in front of the second name.
        let mut data = vec![
            BSV_MAGIC,
            0x11,
            0,
            0,
            vlq(2),
            vlq(0),
            vlq(0),
            vlq(2),
            0x40,
            0x13,
        ];
        data.extend(b"chara/a\0");
        data.extend(0x8123_4567_89AB_CDEFu64.to_be_bytes());
        data.extend(b"chara/b\0");
        data.extend(7u64.to_be_bytes());

        let rows = parseAnonymousBsv(&data).unwrap();

        let parsed: Vec<(&str, u64)> = rows
            .iter()
            .map(|row| (row[0].asStr().unwrap(), row[1].asU64().unwrap()))
            .collect();
        assert_eq!(
            parsed,
            vec![("chara/a", 0x8123_4567_89AB_CDEF), ("chara/b", 7)]
        );
    }

    #[test]
    fn contentRowsCarryTheBundlesTheyDependOn() {
        // One row of (name, dependencies, two unused numbers, size, checksum, key).
        let mut data = vec![
            BSV_MAGIC,
            0x11,
            0,
            0,
            vlq(1),
            vlq(0),
            vlq(0),
            vlq(7),
            0x40,
            0x40,
            0x11,
            0x11,
            0x11,
            0x11,
            0x13,
        ];
        data.extend(b"3d/env/race/pfb shader;3d/model ");
        data.extend([vlq(0), vlq(0), vlq(5), vlq(9)]);
        data.extend(7u64.to_be_bytes());

        let entries = parseContentManifest(&data).unwrap();

        assert_eq!(entries[0].name, "3d/env/race/pfb");
        assert_eq!(entries[0].dependencies, vec!["shader", "3d/model"]);
        assert_eq!(
            (entries[0].size, entries[0].checksum, entries[0].key),
            (5, 9, 7)
        );
    }

    #[test]
    fn decryptionSkipsTheHeaderAndUsesTheExpandedKey() {
        let original = vec![0u8; ASSET_BUNDLE_HEADER_SIZE + 16];
        let mut bundle = original.clone();

        decryptAssetBundle(&mut bundle, &[0x01], 0x0102_0304_0506_0708);

        assert!(bundle[..ASSET_BUNDLE_HEADER_SIZE]
            .iter()
            .all(|byte| *byte == 0));
        // Base byte 0x01 XOR the file key's little-endian bytes 08 07 06 05 04 03 02 01.
        assert_eq!(
            &bundle[ASSET_BUNDLE_HEADER_SIZE..ASSET_BUNDLE_HEADER_SIZE + 8],
            &[0x09, 0x06, 0x07, 0x04, 0x05, 0x02, 0x03, 0x00]
        );

        decryptAssetBundle(&mut bundle, &[0x01], 0x0102_0304_0506_0708);
        assert_eq!(bundle, original);
    }

    #[test]
    fn unencryptedFilesStayAsServed() {
        let original: Vec<u8> = (0..=255).cycle().take(600).collect();
        let mut bundle = original.clone();

        decryptAssetBundle(&mut bundle, &[0x5A, 0xA5], 0);

        assert_eq!(bundle, original);
    }
}
