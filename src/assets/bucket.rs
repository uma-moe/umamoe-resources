use super::types::{
    Bucket, ListedObject, SignedRequest, ASSET_BUCKET, ASSET_CACHE_CONTROL, PUT_ATTEMPTS,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, ETAG};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::warn;

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
            folder: String::new(),
        })
    }

    /// The same bucket seen from `folder`: every key lies below it. A tree published there keeps
    /// the root tree's key layout without touching the root's objects.
    pub fn inFolder(&self, folder: &str) -> Self {
        Self {
            folder: format!("{}/", folder.trim_matches('/')),
            ..self.clone()
        }
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

    /// Writes the object with the artwork cache lifetime.
    pub(super) async fn putObject(
        &self,
        client: &reqwest::Client,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<()> {
        self.putObjectWithCache(client, key, body, content_type, ASSET_CACHE_CONTROL)
            .await
    }

    /// Writes the object, waiting 1, 2, 4 and 8 seconds before retrying a write the bucket
    /// refused for load.
    pub(super) async fn putObjectWithCache(
        &self,
        client: &reqwest::Client,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
        cache_control: &str,
    ) -> Result<()> {
        for attempt in 1..=PUT_ATTEMPTS {
            let request = self.signedRequest("PUT", key, &body)?;
            let response = client
                .put(&request.url)
                .headers(request.headers)
                .header("content-type", content_type)
                .header("cache-control", cache_control)
                .body(body.clone())
                .send()
                .await
                .with_context(|| format!("PUT {key} failed"))?;

            let status = response.status();
            if status.is_success() {
                return Ok(());
            }

            let detail = response.text().await.unwrap_or_default();
            if !isRetryable(status) || attempt == PUT_ATTEMPTS {
                bail!("PUT {key} returned HTTP {status}: {detail}");
            }
            warn!(key, %status, attempt, "the bucket refused a write, retrying");
            tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
        }
        unreachable!("the last attempt returns or fails")
    }

    /// Copies `source`'s object at `key` to the same key below this bucket's folder without
    /// downloading it. The copy keeps the object's content type and cache lifetime.
    pub(super) async fn copyObjectFrom(
        &self,
        client: &reqwest::Client,
        source: &Bucket,
        key: &str,
    ) -> Result<()> {
        let source_key = format!("{}{key}", source.folder);
        let copy_source = format!("/{}/{}", source.name, encodeKey(&source_key));
        for attempt in 1..=PUT_ATTEMPTS {
            let request = self.signedRequestWith(
                "PUT",
                key,
                &[],
                &[("x-amz-copy-source", copy_source.clone())],
            )?;
            let response = client
                .put(&request.url)
                .headers(request.headers)
                .body(Vec::new())
                .send()
                .await
                .with_context(|| format!("copying {key} failed"))?;

            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            // S3 may report a failed copy in the body of a 200 answer.
            if status.is_success() && !detail.contains("<Error>") {
                return Ok(());
            }
            if !isRetryable(status) || attempt == PUT_ATTEMPTS {
                bail!("copying {key} returned HTTP {status}: {detail}");
            }
            warn!(key, %status, attempt, "the bucket refused a copy, retrying");
            tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
        }
        unreachable!("the last attempt returns or fails")
    }

    /// Every object whose key starts with `prefix`, a page of up to 1000 at a time. Errors leave
    /// out the request URL, which names the account.
    pub(super) async fn listObjects(
        &self,
        client: &reqwest::Client,
        prefix: &str,
    ) -> Result<Vec<ListedObject>> {
        let mut objects = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let mut query = vec![
                ("list-type", "2".to_string()),
                ("prefix", format!("{}{prefix}", self.folder)),
            ];
            if let Some(token) = &continuation {
                query.push(("continuation-token", token.clone()));
            }
            let request = self.signedPath("GET", &format!("/{}", self.name), &query, &[])?;
            let response = client
                .get(&request.url)
                .headers(request.headers)
                .send()
                .await
                .map_err(|error| anyhow!("listing the bucket failed: {}", error.without_url()))?;

            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if !status.is_success() {
                bail!("listing the bucket returned HTTP {status}");
            }
            let page = listingPage(&body)?;
            objects.extend(
                page.objects
                    .into_iter()
                    .filter_map(|object| self.relativeObject(object)),
            );
            match page.continuation {
                Some(token) => continuation = Some(token),
                None => return Ok(objects),
            }
        }
    }

    fn signedRequest(&self, method: &str, key: &str, body: &[u8]) -> Result<SignedRequest> {
        self.signedRequestWith(method, key, body, &[])
    }

    fn signedRequestWith(
        &self,
        method: &str,
        key: &str,
        body: &[u8],
        amz_headers: &[(&'static str, String)],
    ) -> Result<SignedRequest> {
        let key = format!("{}{key}", self.folder);
        self.signedPathWith(
            method,
            &format!("/{}/{}", self.name, encodeKey(&key)),
            &[],
            body,
            amz_headers,
        )
    }

    /// A listed object with its key as it is used from this bucket's folder.
    fn relativeObject(&self, object: ListedObject) -> Option<ListedObject> {
        let key = object.key.strip_prefix(&self.folder)?.to_string();
        Some(ListedObject {
            key,
            size: object.size,
        })
    }

    fn signedPath(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, String)],
        body: &[u8],
    ) -> Result<SignedRequest> {
        self.signedPathWith(method, path, query, body, &[])
    }

    /// Signs the minimum set S3 requires, `host`, `x-amz-content-sha256` and `x-amz-date`, plus
    /// any other `x-amz-` headers the request sends. Content type and cache control are not
    /// `x-amz-` headers, so they stay unsigned.
    fn signedPathWith(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, String)],
        body: &[u8],
        amz_headers: &[(&'static str, String)],
    ) -> Result<SignedRequest> {
        let endpoint = reqwest::Url::parse(&self.endpoint)
            .with_context(|| format!("invalid ASSETS_S3_ENDPOINT: {}", self.endpoint))?;
        let host = endpoint
            .host_str()
            .context("ASSETS_S3_ENDPOINT has no host")?;
        let host = match endpoint.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };

        let mut query_pairs: Vec<String> = query
            .iter()
            .map(|(name, value)| format!("{}={}", encodeQueryPart(name), encodeQueryPart(value)))
            .collect();
        query_pairs.sort_unstable();
        let canonical_query = query_pairs.join("&");
        let payload_hash = hex::encode(Sha256::digest(body));
        let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let date = &amz_date[..8];

        let mut headers = vec![
            ("host", host),
            ("x-amz-content-sha256", payload_hash.clone()),
            ("x-amz-date", amz_date.clone()),
        ];
        headers.extend(amz_headers.iter().cloned());
        // SigV4 lists the signed headers sorted by name.
        headers.sort_unstable_by_key(|(name, _)| *name);
        let canonical_headers = headers
            .iter()
            .map(|(name, value)| format!("{name}:{value}\n"))
            .collect::<String>();
        let signed_headers = headers
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(";");
        let canonical_request =
            format!("{method}\n{path}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

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

        let url = match canonical_query.is_empty() {
            true => format!("{}{path}", self.endpoint),
            false => format!("{}{path}?{canonical_query}", self.endpoint),
        };
        Ok(SignedRequest {
            url,
            headers: header_map,
        })
    }
}

/// R2 answers 429 to more than about one write per second to the same object, and 5xx when
/// it is briefly overloaded.
fn isRetryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn hmacSha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encodes everything but the unreserved set, keeping `/` so keys stay path-shaped.
pub(super) fn encodeKey(key: &str) -> String {
    percentEncode(key, |byte| byte == b'/')
}

/// A query name or value as SigV4 wants it: everything but the unreserved set encoded.
fn encodeQueryPart(text: &str) -> String {
    percentEncode(text, |_| false)
}

fn percentEncode(text: &str, keeps: impl Fn(u8) -> bool) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if is_unreserved || keeps(byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// One ListObjectsV2 answer: its objects, and the token for the next page when it is cut short.
#[derive(Debug, PartialEq)]
struct ListingPage {
    objects: Vec<ListedObject>,
    continuation: Option<String>,
}

fn listingPage(xml: &str) -> Result<ListingPage> {
    let mut objects = Vec::new();
    for entry in xml.split("<Contents>").skip(1) {
        let key = xmlElement(entry, "Key").context("a listed object has no key")?;
        let size = xmlElement(entry, "Size")
            .and_then(|size| size.parse().ok())
            .context("a listed object has no size")?;
        objects.push(ListedObject {
            key: xmlText(key),
            size,
        });
    }

    let is_truncated = xmlElement(xml, "IsTruncated") == Some("true");
    let continuation = match is_truncated {
        true => Some(xmlText(
            xmlElement(xml, "NextContinuationToken").context("a cut-short listing has no token")?,
        )),
        false => None,
    };
    Ok(ListingPage {
        objects,
        continuation,
    })
}

fn xmlElement<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{name}>"))? + name.len() + 2;
    let length = xml[start..].find(&format!("</{name}>"))?;
    Some(&xml[start..start + length])
}

/// Undoes the escapes XML text uses; `&amp;` last, so an escaped escape stays as written.
fn xmlText(escaped: &str) -> String {
    escaped
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket() -> Bucket {
        Bucket {
            endpoint: "https://bucket.example".to_string(),
            region: "auto".to_string(),
            name: "assets".to_string(),
            access_key: "key".to_string(),
            secret_key: "secret".to_string(),
            folder: String::new(),
        }
    }

    #[test]
    fn aFolderedBucketReadsAndWritesBelowItsFolder() {
        let root = bucket();
        let english = root.inFolder("/en/");

        let root_url = root.signedRequest("PUT", "art/umas/100101/icon.webp", &[]);
        let english_url = english.signedRequest("PUT", "art/umas/100101/icon.webp", &[]);

        assert_eq!(
            root_url.unwrap().url,
            "https://bucket.example/assets/art/umas/100101/icon.webp"
        );
        assert_eq!(
            english_url.unwrap().url,
            "https://bucket.example/assets/en/art/umas/100101/icon.webp"
        );
    }

    #[test]
    fn aFolderedBucketListsItsOwnObjectsByTheirKeysInside() {
        let english = bucket().inFolder("en");
        let listed = |key: &str| ListedObject {
            key: key.to_string(),
            size: 1,
        };

        assert_eq!(
            english.relativeObject(listed("en/art/a.webp")),
            Some(listed("art/a.webp"))
        );
        assert_eq!(english.relativeObject(listed("art/a.webp")), None);
        assert_eq!(
            bucket().relativeObject(listed("en/art/a.webp")),
            Some(listed("en/art/a.webp"))
        );
    }

    #[test]
    fn aCopySignsItsSourceAmongTheHeaders() {
        let copy_source = ("x-amz-copy-source", "/assets/art/a.webp".to_string());

        let request = bucket()
            .inFolder("en")
            .signedRequestWith("PUT", "art/a.webp", &[], &[copy_source])
            .unwrap();

        let authorization = request.headers[AUTHORIZATION].to_str().unwrap();
        assert_eq!(request.url, "https://bucket.example/assets/en/art/a.webp");
        assert!(authorization
            .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-copy-source;x-amz-date,"));
        assert_eq!(request.headers["x-amz-copy-source"], "/assets/art/a.webp");
    }

    #[test]
    fn aListingPageGivesItsObjectsAndTheNextToken() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated>            <Contents><Key>art/a&amp;b.json</Key><Size>12</Size></Contents>            <Contents><Key>art/c.glb</Key><Size>3</Size></Contents>            <NextContinuationToken>next/page=</NextContinuationToken></ListBucketResult>";

        let page = listingPage(xml).unwrap();

        assert_eq!(
            page,
            ListingPage {
                objects: vec![
                    ListedObject {
                        key: "art/a&b.json".to_string(),
                        size: 12
                    },
                    ListedObject {
                        key: "art/c.glb".to_string(),
                        size: 3
                    },
                ],
                continuation: Some("next/page=".to_string()),
            }
        );
    }

    #[test]
    fn theLastListingPageHasNoToken() {
        let xml = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";

        let page = listingPage(xml).unwrap();

        assert_eq!(page.continuation, None);
        assert!(page.objects.is_empty());
    }

    #[test]
    fn aQueryIsSortedAndEncodedInTheUrlItSigns() {
        let query = [
            ("prefix", "art/".to_string()),
            ("list-type", "2".to_string()),
            ("continuation-token", "a+b=".to_string()),
        ];

        let request = bucket().signedPath("GET", "/assets", &query, &[]).unwrap();

        assert_eq!(
            request.url,
            "https://bucket.example/assets?continuation-token=a%2Bb%3D&list-type=2&prefix=art%2F"
        );
    }
}
