/// The site's version endpoint. It answers for Global, and for another region as `?region=`.
pub(super) const VERSION_URL: &str = "https://uma.moe/api/ver";

/// Points the version check at another backend, such as a local one.
pub(super) const VERSION_URL_VARIABLE: &str = "GAME_VERSION_URL";

#[derive(serde::Deserialize)]
pub(super) struct GameVersion {
    pub(super) app_version: String,
    pub(super) resource_version: String,
}
