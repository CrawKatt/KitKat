use crate::utils::Error;
use reqwest::Client;
use std::fs;
use std::path::Path;
use std::time::Duration;
use wana_kana::ConvertJapanese;

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

        let text = prepare_japanese_for_tts(text);
        if text.trim().is_empty() {
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

pub fn prepare_japanese_for_tts(text: &str) -> String {
    let mut result = String::with_capacity(text.len() * 2);
    let mut latin = String::new();

    for ch in text.chars() {
        if is_latin_token_char(ch) {
            latin.push(ch);
            continue;
        }

        if !latin.is_empty() {
            result.push_str(&convert_latin_token(&latin));
            latin.clear();
        }
        result.push(ch);
    }

    if !latin.is_empty() {
        result.push_str(&convert_latin_token(&latin));
    }

    result
}

fn is_latin_token_char(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '\''
}

fn convert_latin_token(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }

    let lower = token.to_lowercase();
    let as_romaji = lower.to_katakana();
    if as_romaji.chars().all(|c| !c.is_ascii_alphabetic()) {
        return as_romaji;
    }

    let forced = englishish_to_katakana(&lower);
    if !forced.is_empty() && forced.chars().all(|c| !c.is_ascii_alphabetic()) {
        return forced;
    }

    token.to_string()
}

fn englishish_to_katakana(word: &str) -> String {
    let pseudo = english_to_pseudo_romaji(word);
    let kata = pseudo.to_katakana();
    if kata.chars().all(|c| !c.is_ascii_alphabetic()) {
        kata
    } else {
        String::new()
    }
}

fn english_to_pseudo_romaji(input: &str) -> String {
    let mut s = input.to_lowercase();

    let replacements: &[(&str, &str)] = &[
        ("tion", "shon"),
        ("sion", "zhon"),
        ("ture", "chaa"),
        ("ough", "oo"),
        ("augh", "oo"),
        ("eigh", "ei"),
        ("wh", "w"),
        ("ph", "f"),
        ("ck", "k"),
        ("qu", "kw"),
        ("th", "s"),
        ("ch", "ch"),
        ("sh", "sh"),
        ("kn", "n"),
        ("wr", "r"),
        ("dg", "j"),
        ("x", "kkusu"),
    ];

    for (from, to) in replacements {
        s = s.replace(from, to);
    }

    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() * 2);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == 'c' {
            let next = chars.get(i + 1).copied();
            if matches!(next, Some('e') | Some('i') | Some('y')) {
                out.push('s');
            } else {
                out.push('k');
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    s = out;

    insert_u_for_bare_consonants(&s)
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'i' | 'u' | 'e' | 'o')
}

fn is_consonant(c: char) -> bool {
    c.is_ascii_alphabetic() && !is_vowel(c)
}

fn insert_u_for_bare_consonants(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(chars.len() * 2);

    for i in 0..chars.len() {
        let c = chars[i];
        out.push(c);

        if !is_consonant(c) || c == 'n' {
            continue;
        }

        let next = chars.get(i + 1).copied();
        let needs_vowel = match next {
            None => true,
            Some(n) if is_consonant(n) => true,
            Some(n) if is_vowel(n) => false,
            Some(_) => true,
        };

        if needs_vowel {
            if c == 't' && next == Some('s') {
                continue;
            }
            out.push('u');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::prepare_japanese_for_tts;

    #[test]
    fn converts_romaji_to_katakana() {
        assert_eq!(prepare_japanese_for_tts("konnichiwa"), "コンニチワ");
        assert_eq!(prepare_japanese_for_tts("arigatou gozaimasu"), "アリガトウ ゴザイマス");
        assert_eq!(prepare_japanese_for_tts("こんにちは konnichiwa"), "こんにちは コンニチワ");
    }

    #[test]
    fn converts_english_loanwords_to_katakana() {
        let out = prepare_japanese_for_tts("Discord");
        assert!(out.chars().all(|c| !c.is_ascii_alphabetic()), "expected no latin in {out}");
        assert!(!out.is_empty());

        let mixed = prepare_japanese_for_tts("Discord で話そう");
        assert!(mixed.contains("で話そう"), "got: {mixed}");
        assert_eq!(mixed.chars().filter(|c| c.is_ascii_alphabetic()).count(), 0, "got: {mixed}");
    }

    #[test]
    fn keeps_japanese_script() {
        assert_eq!(
            prepare_japanese_for_tts("ディスコードで話そう"),
            "ディスコードで話そう"
        );
    }
}