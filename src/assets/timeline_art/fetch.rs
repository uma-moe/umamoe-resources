//! Downloading banner pictures politely: one request at a time, with retries for load.

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::{sleep, Instant};

const USER_AGENT: &str = "umamoe-resources-image-sync/3.0 (+https://uma.moe)";

/// The pause between two requests, as the frontend's sync kept it.
const REQUEST_INTERVAL: Duration = Duration::from_millis(250);

const ATTEMPTS: u32 = 4;

/// How long after its release a missing English header is still expected to appear.
const ENGLISH_RETRY_DAYS: i64 = 7;

pub(in crate::assets) struct SourceFetcher {
    client: reqwest::Client,
    last_request: Option<Instant>,
    /// Campaign headers by the thumbnail banner preferred over them, when one exists.
    preferred_pictures: HashMap<String, String>,
}

impl SourceFetcher {
    pub(in crate::assets) fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            client,
            last_request: None,
            preferred_pictures: HashMap::new(),
        })
    }

    /// The picture's bytes, or `None` when the host answers 403 or 404.
    pub(in crate::assets) async fn download(&mut self, url: &str) -> Result<Option<Vec<u8>>> {
        let Some(response) = self.request(reqwest::Method::GET, url).await? else {
            return Ok(None);
        };
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("failed to read {url}"))?;
        Ok(Some(bytes.to_vec()))
    }

    /// A campaign's thumbnail banner when it exists; its header otherwise.
    pub(super) async fn campaignPicture(&mut self, header: &str) -> Result<String> {
        if let Some(known) = self.preferred_pictures.get(header) {
            return Ok(known.clone());
        }
        let thumbnail = header.replace("/Header/header_", "/Thumbnail/banner_");
        if thumbnail == header {
            return Ok(header.to_string());
        }

        let exists = self
            .request(reqwest::Method::HEAD, &thumbnail)
            .await?
            .is_some();
        let preferred = if exists {
            thumbnail
        } else {
            header.to_string()
        };
        self.preferred_pictures
            .insert(header.to_string(), preferred.clone());
        Ok(preferred)
    }

    async fn request(
        &mut self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<Option<reqwest::Response>> {
        for attempt in 0..ATTEMPTS {
            self.waitForTurn().await;
            let outcome = self.client.request(method.clone(), url).send().await;
            let last_attempt = attempt + 1 == ATTEMPTS;

            match outcome {
                Ok(response) if response.status().is_success() => return Ok(Some(response)),
                Ok(response) if isMissing(response.status()) => return Ok(None),
                Ok(response) if !isRetryable(response.status()) || last_attempt => {
                    bail!("{url} returned HTTP {}", response.status());
                }
                Err(error) if last_attempt => {
                    return Err(error).with_context(|| format!("failed to fetch {url}"));
                }
                _ => sleep(Duration::from_secs(1 << attempt)).await,
            }
        }
        bail!("failed to fetch {url}")
    }

    async fn waitForTurn(&mut self) {
        if let Some(last) = self.last_request {
            sleep(REQUEST_INTERVAL.saturating_sub(last.elapsed())).await;
        }
        self.last_request = Some(Instant::now());
    }
}

fn isMissing(status: StatusCode) -> bool {
    matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND)
}

fn isRetryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Whether a missing English header is past the week after the release its `_L<unix>` names.
/// Pictures without a release in their name are always asked again.
pub(super) fn isPastRetryWindow(url: &str, now_seconds: i64) -> bool {
    releaseSeconds(url).is_some_and(|release| now_seconds > release + ENGLISH_RETRY_DAYS * 86_400)
}

fn releaseSeconds(url: &str) -> Option<i64> {
    url.match_indices("_L").find_map(|(start, _)| {
        let rest = &url[start + 2..];
        let length = rest.bytes().take_while(u8::is_ascii_digit).count();
        let followed_by_extension = rest[length..].starts_with('.');
        (length > 0 && followed_by_extension)
            .then(|| rest[..length].parse().ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "https://en/Header/gacha_banner_30101_L1753135200.png";

    #[test]
    fn aMissingHeaderIsRetiredAWeekAfterItsRelease() {
        let release = 1_753_135_200;

        assert!(!isPastRetryWindow(HEADER, release + 6 * 86_400));
        assert!(isPastRetryWindow(HEADER, release + 8 * 86_400));
    }

    #[test]
    fn aPictureWithoutAReleaseIsNeverRetired() {
        assert!(!isPastRetryWindow(
            "https://en/Thumbnail/banner_1.png",
            i64::MAX / 2
        ));
    }
}
