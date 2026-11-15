pub(super) const VERSION_URL: &str = "https://uma.moe/api/ver";

#[derive(serde::Deserialize)]
pub(super) struct GameVersion {
    pub(super) app_version: String,
    pub(super) resource_version: String,
}
