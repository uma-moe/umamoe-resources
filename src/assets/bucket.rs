use super::types::{Bucket, SignedRequest, ASSET_BUCKET, ASSET_CACHE_CONTROL};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, ETAG};
use sha2::{Digest, Sha256};

impl Bucket {
    /// Reads the endpoint and write key for the asset bucket from the environment.
    pub fn fromEnv() -> Result<Self> {
        let endpoint = std::env::var("ASSETS_S3_ENDPOINT").context("set ASSETS_S3_ENDPOINT")?;

        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            // R2 wants "auto"; anything else is passed through.
            region: std::env::var("ASSETS_S3_REGION").unwrap_or_else(|_| "auto".to_string()),
            name: ASSET_BUCKET.to_string(),
            access_key: std::env::var("ASSETS_S3_ACCESS_KEY_ID")
                .context("set ASSETS_S3_ACCESS_KEY_ID")?,
            secret_key: std::env::var("ASSETS_S3_SECRET_ACCESS_KEY")
                .context("set ASSETS_S3_SECRET_ACCESS_KEY")?,
        })
    }

    /// The MD5 the bucket reports for `key`, or `None` when it holds no object there. Objects are
    /// written in a single PUT, so their ETag is the MD5 of their bytes.
    pub(super) async fn objectEtag(
        &self,
        client: &reqwest::Client,
        key: &str,
    ) -> Result<Option<String>> {
        let request = self.signedRequest("HEAD", key, &[])?;
        let response = client
            .head(&request.url)
            .headers(request.headers)
            .send()
            .await
            .with_context(|| format!("HEAD {key} failed"))?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("HEAD {key} returned HTTP {status}");
        }

        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .with_context(|| format!("HEAD {key} returned no ETag"))?;
        Ok(Some(etag.trim_matches('"').to_ascii_lowercase()))
    }

    /// The object's bytes, or `None` when the bucket holds nothing at `key`.
    pub(super) async fn getObject(
        &self,
        client: &reqwest::Client,
        key: &str,
    ) -> Result<Option<Vec<u8>>> {
        let request = self.signedRequest("GET", key, &[])?;
        let response = client
            .get(&request.url)
            .headers(request.headers)
            .send()
            .await
            .with_context(|| format!("GET {key} failed"))?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("GET {key} returned HTTP {status}");
        }

        Ok(Some(response.bytes().await?.to_vec()))
    }

    /// Removes the object at `key`; a key that holds nothing is not an error.
    pub(super) async fn deleteObject(&self, client: &reqwest::Client, key: &str) -> Result<()> {
        let request = self.signedRequest("DELETE", key, &[])?;
        let response = client
            .delete(&request.url)
            .headers(request.headers)
            .send()
            .await
            .with_context(|| format!("DELETE {key} failed"))?;

        let status = response.status();
        if !status.is_success() && status.as_u16() != 404 {
            let detail = response.text().await.unwrap_or_default();
            bail!("DELETE {key} returned HTTP {status}: {detail}");
        }
        Ok(())
    }

    pub(super) async fn putObject(
        &self,
        client: &reqwest::Client,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<()> {
        let request = self.signedRequest("PUT", key, &body)?;
        let response = client
            .put(&request.url)
            .headers(request.headers)
            .header("content-type", content_type)
            .header("cache-control", ASSET_CACHE_CONTROL)
            .body(body)
            .send()
            .await
            .with_context(|| format!("PUT {key} failed"))?;

        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("PUT {key} returned HTTP {status}: {detail}");
        }
        Ok(())
    }

    /// Signs the minimum set S3 requires: `host`, `x-amz-content-sha256` and `x-amz-date`. Content
    /// type and cache control are not `x-amz-` headers, so they stay unsigned.
    fn signedRequest(&self, method: &str, key: &str, body: &[u8]) -> Result<SignedRequest> {
        let endpoint = reqwest::Url::parse(&self.endpoint)
            .with_context(|| format!("invalid ASSETS_S3_ENDPOINT: {}", self.endpoint))?;
        let host = endpoint
            .host_str()
            .context("ASSETS_S3_ENDPOINT has no host")?;
        let host = match endpoint.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };

        let path = format!("/{}/{}", self.name, encodeKey(key));
        let payload_hash = hex::encode(Sha256::digest(body));
        let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let date = &amz_date[..8];

        // Already in the sorted order SigV4 requires.
        let headers = [
            ("host", host),
            ("x-amz-content-sha256", payload_hash.clone()),
            ("x-amz-date", amz_date.clone()),
        ];
        let canonical_headers = headers
            .iter()
            .map(|(name, value)| format!("{name}:{value}\n"))
            .collect::<String>();
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request =
            format!("{method}\n{path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );

        let mut signing_key = hmacSha256(
            format!("AWS4{}", self.secret_key).as_bytes(),
            date.as_bytes(),
        );
        signing_key = hmacSha256(&signing_key, self.region.as_bytes());
        signing_key = hmacSha256(&signing_key, b"s3");
        signing_key = hmacSha256(&signing_key, b"aws4_request");
        let signature = hex::encode(hmacSha256(&signing_key, string_to_sign.as_bytes()));

        let mut header_map = HeaderMap::new();
        for (name, value) in headers {
            header_map.insert(
                HeaderName::from_static(name),
                HeaderValue::from_str(&value).context("invalid signed header value")?,
            );
        }
        header_map.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!(
                "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
                self.access_key
            ))
            .context("invalid authorization header")?,
        );

        Ok(SignedRequest {
            url: format!("{}{path}", self.endpoint),
            headers: header_map,
        })
    }
}

fn hmacSha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encodes everything but the unreserved set, keeping `/` so keys stay path-shaped.
pub(super) fn encodeKey(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    for byte in key.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}
