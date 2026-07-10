use crate::utils::Error;
use reqwest::Client;
use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct VoiceVoxClient {
    http: Client,
    base_url: String,
    default_speaker_id: i32,
}

impl VoiceVoxClient {
    pub fn from_env() -> Result<Self, Error> {
        let host = dotenvy::var("VOICEVOX_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = dotenvy::var("VOICEVOX_PORT").unwrap_or_else(|_| "50021".to_string());
        let default_speaker_id = Self::read_speaker_id().unwrap_or(3);

        Self::new(format!("http://{host}:{port}"), default_speaker_id)
    }

    pub fn new(base_url: String, default_speaker_id: i32) -> Result<Self, Error> {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            default_speaker_id,
        })
    }

    pub fn speaker_id(&self) -> i32 {
        Self::read_speaker_id().unwrap_or(self.default_speaker_id)
    }

    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>, Error> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::from("El texto a sintetizar está vacío".to_string()));
        }

        let speaker_id = self.speaker_id();
        let query = self.audio_query(&text, speaker_id).await?;
        self.synthesis(&query, speaker_id).await
    }

    fn read_speaker_id() -> Option<i32> {
        if let Some(id) = Self::speaker_id_from_env_file(Path::new(".env")) {
            return Some(id);
        }

        dotenvy::var("VOICEVOX_SPEAKER_ID")
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    fn speaker_id_from_env_file(path: &Path) -> Option<i32> {
        let content = fs::read_to_string(path).ok()?;

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            if key.trim() != "VOICEVOX_SPEAKER_ID" {
                continue;
            }

            let value = value.trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim();

            return value.parse().ok();
        }

        None
    }

    async fn audio_query(&self, text: &str, speaker_id: i32) -> Result<String, Error> {
        let url = format!("{}/audio_query", self.base_url);
        let response = self
            .http
            .post(&url)
            .query(&[("text", text), ("speaker", &speaker_id.to_string())])
            .query(&[("enable_katakana_english", true)])
            .send()
            .await
            .map_err(|e| Error::from(format!("No se pudo conectar a VOICEVOX en {}: {e}. ¿Está corriendo el engine en ese host/puerto?", self.base_url)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::from(format!("VOICEVOX audio_query falló ({status}): {body}")));
        }

        Ok(response.text().await?)
    }

    async fn synthesis(&self, audio_query_json: &str, speaker_id: i32) -> Result<Vec<u8>, Error> {
        let url = format!("{}/synthesis", self.base_url);
        let response = self
            .http
            .post(&url)
            .query(&[("speaker", speaker_id.to_string())])
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(audio_query_json.to_string())
            .send()
            .await
            .map_err(|e| Error::from(format!("No se pudo sintetizar con VOICEVOX en {}: {e}", self.base_url)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::from(format!(
                "VOICEVOX synthesis falló ({status}): {body}"
            )));
        }

        let bytes = response.bytes().await?.to_vec();
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" {
            return Err(Error::from("VOICEVOX no devolvió un WAV válido".to_string()));
        }

        Ok(bytes)
    }
}