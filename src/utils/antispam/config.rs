use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::anyhow;
use serenity::all::ChannelId;

use super::Error;

pub const DEFAULT_WINDOW_SECONDS: i64 = 12;
pub const DEFAULT_DISTINCT_CHANNELS_THRESHOLD: usize = 3;

#[derive(Clone, Debug)]
pub struct AntiSpamConfig {
    pub alert_channel_id: Option<ChannelId>,
    pub duplicate_window_seconds: i64,
    pub distinct_channels_threshold: usize,
}

impl Default for AntiSpamConfig {
    fn default() -> Self {
        Self {
            alert_channel_id: None,
            duplicate_window_seconds: DEFAULT_WINDOW_SECONDS,
            distinct_channels_threshold: DEFAULT_DISTINCT_CHANNELS_THRESHOLD,
        }
    }
}

impl AntiSpamConfig {
    pub fn load() -> Result<Self, Error> {
        let path = dotenvy::var("KITKAT_CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
        let path = PathBuf::from(path);

        match fs::read_to_string(&path) {
            Ok(raw_config) => Self::parse(&raw_config),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                println!(
                    "No se encontro {}. AntiSpam usara defaults y no enviara avisos hasta configurar [antispam].alert_channel_id.",
                    path.display()
                );
                Ok(Self::default())
            }
            Err(error) => Err(anyhow!("No se pudo leer {}: {error}", path.display()).into()),
        }
    }

    fn parse(raw_config: &str) -> Result<Self, Error> {
        let mut config = Self::default();
        let mut current_section = String::new();

        for (index, raw_line) in raw_config.lines().enumerate() {
            let line_number = index + 1;
            let line = strip_comment(raw_line).trim();

            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                current_section = line[1..line.len() - 1].trim().to_string();
                continue;
            }

            if current_section != "antispam" {
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                return Err(anyhow!("Linea {line_number}: se esperaba key = value").into());
            };

            let key = raw_key.trim();
            let value = raw_value.trim();

            match key {
                "alert_channel_id" => {
                    config.alert_channel_id = Some(ChannelId::new(parse_u64(value, line_number, key)?));
                }
                "duplicate_window_seconds" => {
                    let seconds = parse_i64(value, line_number, key)?;
                    if seconds <= 0 {
                        return Err(anyhow!("Linea {line_number}: `{key}` debe ser mayor que 0").into());
                    }
                    config.duplicate_window_seconds = seconds;
                }
                "distinct_channels_threshold" => {
                    let threshold = parse_usize(value, line_number, key)?;
                    if threshold < 2 {
                        return Err(anyhow!("Linea {line_number}: `{key}` debe ser al menos 2").into());
                    }
                    config.distinct_channels_threshold = threshold;
                }
                _ => {}
            }
        }

        Ok(config)
    }
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim_end()
}

fn parse_u64(raw_value: &str, line_number: usize, key: &str) -> Result<u64, Error> {
    let cleaned = trim_quotes(raw_value);
    cleaned.parse::<u64>().map_err(|error| {
        anyhow!("Linea {line_number}: `{key}` invalido (`{raw_value}`): {error}").into()
    })
}

fn parse_i64(raw_value: &str, line_number: usize, key: &str) -> Result<i64, Error> {
    let cleaned = trim_quotes(raw_value);
    cleaned.parse::<i64>().map_err(|error| {
        anyhow!("Linea {line_number}: `{key}` invalido (`{raw_value}`): {error}").into()
    })
}

fn parse_usize(raw_value: &str, line_number: usize, key: &str) -> Result<usize, Error> {
    let cleaned = trim_quotes(raw_value);
    cleaned.parse::<usize>().map_err(|error| {
        anyhow!("Linea {line_number}: `{key}` invalido (`{raw_value}`): {error}").into()
    })
}

fn trim_quotes(raw_value: &str) -> &str {
    raw_value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(raw_value)
        .trim()
}
