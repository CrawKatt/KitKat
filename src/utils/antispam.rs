use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::anyhow;
use poise::serenity_prelude as serenity;
use serenity::all::*;
use tokio::time::sleep;

use super::Error;

const DEFAULT_WINDOW_SECONDS: i64 = 12;
const DEFAULT_DISTINCT_CHANNELS_THRESHOLD: usize = 3;
const TIMEOUT_SECONDS: i64 = 7 * 24 * 60 * 60;
const ACTION_COOLDOWN_SECONDS: i64 = 60;
const CHANNEL_FETCH_LIMIT: u8 = 100;
const SANCTION_CLEANUP_PER_CHANNEL_LIMIT: usize = 3;
const SANCTION_CLEANUP_DELAY_MS: u64 = 1200;
const SANCTION_CLEANUP_PASSES: usize = 2;
const SANCTION_CLEANUP_RETRY_DELAY_MS: u64 = 1800;

pub struct AntiSpam {
    config: AntiSpamConfig,
    state: Mutex<HashMap<TrackerKey, UserTracker>>,
}

#[derive(Clone, Debug)]
struct AntiSpamConfig {
    alert_channel_id: Option<ChannelId>,
    duplicate_window_seconds: i64,
    distinct_channels_threshold: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TrackerKey {
    guild_id: GuildId,
    user_id: UserId,
}

#[derive(Default)]
struct UserTracker {
    recent_messages: VecDeque<ObservedMessage>,
    last_action_at: Option<i64>,
}

#[derive(Clone)]
struct ObservedMessage {
    channel_id: ChannelId,
    observed_at: i64,
    fingerprint: MessageFingerprint,
}

#[derive(Clone, Debug)]
pub struct SpamIncident {
    guild_id: GuildId,
    user_id: UserId,
    trigger_message_id: MessageId,
    trigger_channel_id: ChannelId,
    repeated_messages: usize,
    channels: Vec<ChannelId>,
    first_detected_at: i64,
    last_detected_at: i64,
    preview: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Observation {
    guild_id: GuildId,
    user_id: UserId,
    channel_id: ChannelId,
    message_id: MessageId,
    observed_at: i64,
    fingerprint: MessageFingerprint,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MessageFingerprint {
    normalized_content: String,
    image_descriptors: Vec<String>,
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

impl AntiSpam {
    pub fn load() -> Result<Self, Error> {
        let config = AntiSpamConfig::load()?;
        println!(
            "AntiSpam initialized. alert_channel_id={:?}, duplicate_window_seconds={}, distinct_channels_threshold={}",
            config.alert_channel_id.map(ChannelId::get),
            config.duplicate_window_seconds,
            config.distinct_channels_threshold
        );

        Ok(Self {
            config,
            state: Mutex::new(HashMap::new()),
        })
    }

    pub fn observe_message(&self, message: &Message) -> Option<SpamIncident> {
        if message.author.bot || message.webhook_id.is_some() {
            return None;
        }

        let guild_id = message.guild_id?;
        let fingerprint = MessageFingerprint::from_message(message)?;

        let observation = Observation {
            guild_id,
            user_id: message.author.id,
            channel_id: message.channel_id,
            message_id: message.id,
            observed_at: message.timestamp.unix_timestamp(),
            fingerprint,
        };

        self.observe(observation)
    }

    pub async fn enforce(
        &self,
        ctx: &Context,
        incident: SpamIncident,
    ) -> Result<(), Error> {
        let timeout_until =
            Timestamp::from_unix_timestamp(Timestamp::now().unix_timestamp() + TIMEOUT_SECONDS)?;
        let mut member = incident.guild_id.member(ctx, incident.user_id).await?;
        let timeout_result = member.disable_communication_until_datetime(ctx, timeout_until).await;
        let timeout_error = timeout_result.as_ref().err().map(|error| error.to_string());

        sleep(Duration::from_millis(SANCTION_CLEANUP_DELAY_MS)).await;
        let deleted_messages = self.delete_detected_messages(ctx, &incident).await;

        self.send_alert(
            ctx,
            &incident,
            timeout_until,
            timeout_error.as_deref(),
            deleted_messages,
        )
        .await?;
        timeout_result?;

        Ok(())
    }

    fn observe(&self, observation: Observation) -> Option<SpamIncident> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let tracker = state
            .entry(TrackerKey {
                guild_id: observation.guild_id,
                user_id: observation.user_id,
            })
            .or_default();

        tracker.prune(observation.observed_at, self.config.duplicate_window_seconds, );

        if let Some(last_action_at) = tracker.last_action_at {
            if elapsed_seconds(observation.observed_at, last_action_at) <= ACTION_COOLDOWN_SECONDS {
                return None;
            }
        }

        tracker.recent_messages.push_back(ObservedMessage {
            channel_id: observation.channel_id,
            observed_at: observation.observed_at,
            fingerprint: observation.fingerprint.clone(),
        });

        let mut repeated_messages = 0;
        let mut channels = Vec::new();
        let mut first_detected_at = observation.observed_at;

        for recent_message in &tracker.recent_messages {
            if recent_message.fingerprint != observation.fingerprint {
                continue;
            }

            repeated_messages += 1;
            first_detected_at = first_detected_at.min(recent_message.observed_at);

            if !channels.contains(&recent_message.channel_id) {
                channels.push(recent_message.channel_id);
            }
        }

        if channels.len() < self.config.distinct_channels_threshold {
            return None;
        }

        tracker.last_action_at = Some(observation.observed_at);
        tracker.recent_messages.clear();

        Some(SpamIncident {
            guild_id: observation.guild_id,
            user_id: observation.user_id,
            trigger_message_id: observation.message_id,
            trigger_channel_id: observation.channel_id,
            repeated_messages,
            channels,
            first_detected_at,
            last_detected_at: observation.observed_at,
            preview: observation.fingerprint.preview(),
        })
    }

    async fn delete_detected_messages(
        &self,
        ctx: &Context,
        incident: &SpamIncident,
    ) -> usize {
        let mut deleted_messages = 0;
        for pass in 0..SANCTION_CLEANUP_PASSES {
            let Ok(channels) = incident.guild_id.channels(&ctx.http).await else {
                return deleted_messages;
            };

            let ordered_channels = self.prioritize_channels(&channels, &incident.channels);

            for channel in ordered_channels {
                deleted_messages += self
                    .delete_user_messages_in_channel(
                        ctx,
                        channel,
                        incident.user_id,
                        SANCTION_CLEANUP_PER_CHANNEL_LIMIT,
                    )
                    .await;
            }

            if pass + 1 < SANCTION_CLEANUP_PASSES {
                sleep(Duration::from_millis(SANCTION_CLEANUP_RETRY_DELAY_MS)).await;
            }
        }

        deleted_messages
    }

    fn prioritize_channels<'a>(
        &self,
        channels: &'a HashMap<ChannelId, GuildChannel>,
        detected_channels: &[ChannelId],
    ) -> Vec<&'a GuildChannel> {
        let mut ordered_channels = Vec::with_capacity(channels.len());

        for channel_id in detected_channels {
            if let Some(channel) = channels.get(channel_id) {
                ordered_channels.push(channel);
            }
        }

        for (channel_id, channel) in channels {
            if detected_channels.contains(channel_id) {
                continue;
            }

            ordered_channels.push(channel);
        }

        ordered_channels
    }

    async fn delete_user_messages_in_channel(
        &self,
        ctx: &Context,
        channel: &GuildChannel,
        user_id: UserId,
        remaining: usize,
    ) -> usize {
        if !matches!(
            channel.kind,
            ChannelType::Text
                | ChannelType::News
                | ChannelType::PublicThread
                | ChannelType::PrivateThread
                | ChannelType::NewsThread
        ) {
            return 0;
        }

        let mut deleted_messages = 0;
        let mut before = None;

        loop {
            let mut builder = GetMessages::new().limit(CHANNEL_FETCH_LIMIT);
            if let Some(message_id) = before {
                builder = builder.before(message_id);
            }

            let messages = match channel.id.messages(&ctx.http, builder).await {
                Ok(messages) => messages,
                Err(error) => {
                    println!(
                        "AntiSpam cleanup: no se pudieron leer mensajes en canal {}: {}",
                        channel.id.get(),
                        error
                    );
                    break;
                }
            };

            if messages.is_empty() {
                break;
            }

            before = messages.last().map(|message| message.id);

        for message in messages {
            if message.author.id != user_id {
                continue;
            }

                println!(
                    "AntiSpam cleanup: intentando borrar mensaje {} del usuario {} en canal {}",
                    message.id.get(),
                    user_id.get(),
                    channel.id.get()
                );

                match channel.id.delete_message(&ctx.http, message.id).await {
                    Ok(_) => {
                        println!(
                            "AntiSpam cleanup: mensaje {} borrado en canal {}",
                            message.id.get(),
                            channel.id.get()
                        );
                        deleted_messages += 1;
                    }
                    Err(error) => {
                        println!(
                            "AntiSpam cleanup: no se pudo borrar mensaje {} en canal {}: {}",
                            message.id.get(),
                            channel.id.get(),
                            error
                        );
                    }
                }

                if deleted_messages >= remaining {
                    return deleted_messages;
                }
            }

            if before.is_none() {
                break;
            }
        }

        deleted_messages
    }

    async fn send_alert(
        &self,
        ctx: &Context,
        incident: &SpamIncident,
        timeout_until: Timestamp,
        timeout_error: Option<&str>,
        deleted_messages: usize,
    ) -> Result<(), Error> {
        let Some(alert_channel_id) = self.config.alert_channel_id else {
            return Ok(());
        };

        let channel_mentions = incident
            .channels
            .iter()
            .map(|channel_id| format!("<#{}>", channel_id.get()))
            .collect::<Vec<_>>()
            .join(", ");

        let duration_seconds = elapsed_seconds(incident.last_detected_at, incident.first_detected_at);
        let message_link = format!(
            "https://discord.com/channels/{}/{}/{}",
            incident.guild_id.get(),
            incident.trigger_channel_id.get(),
            incident.trigger_message_id.get()
        );

        let timeout_status = match timeout_error {
            Some(error) => format!("fallo al aplicar timeout: {error}"),
            None => format!("timeout aplicado hasta {}", timeout_until),
        };
        let deletion_status = format!(
            "mensajes borrados tras sancion: {}",
            deleted_messages,
        );

        let content = format!(
            "[AntiSpam]\nUsuario: <@{}>\nEstado: {}\nBorrado: {}\nCanales detectados: {}\nRepeticiones: {}\nVentana: {} segundos\nContenido: {}\nMensaje detonante: {}",
            incident.user_id.get(),
            timeout_status,
            deletion_status,
            channel_mentions,
            incident.repeated_messages,
            duration_seconds,
            incident.preview,
            message_link
        );

        alert_channel_id.send_message(&ctx.http, CreateMessage::new().content(content)).await?;

        Ok(())
    }
}

impl AntiSpamConfig {
    fn load() -> Result<Self, Error> {
        let path = dotenvy::var("KITKAT_CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
        let path = PathBuf::from(path);

        match fs::read_to_string(&path) {
            Ok(raw_config) => Self::parse(&raw_config),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                println!("No se encontro {}. AntiSpam usara defaults y no enviara avisos hasta configurar [antispam].alert_channel_id.", path.display());
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
                    let channel_id = parse_u64(value, line_number, key)?;
                    config.alert_channel_id = Some(ChannelId::new(channel_id));
                }
                "duplicate_window_seconds" => {
                    let seconds = parse_i64(value, line_number, key)?;
                    if seconds <= 0 {
                        return Err(
                            anyhow!("Linea {line_number}: `{key}` debe ser mayor que 0").into()
                        );
                    }
                    config.duplicate_window_seconds = seconds;
                }
                "distinct_channels_threshold" => {
                    let threshold = parse_usize(value, line_number, key)?;
                    if threshold < 2 {
                        return Err(
                            anyhow!("Linea {line_number}: `{key}` debe ser al menos 2").into()
                        );
                    }
                    config.distinct_channels_threshold = threshold;
                }
                _ => {}
            }
        }

        Ok(config)
    }
}

impl UserTracker {
    fn prune(&mut self, now: i64, window_seconds: i64) {
        let mut retained = VecDeque::with_capacity(self.recent_messages.len());

        while let Some(entry) = self.recent_messages.pop_front() {
            if elapsed_seconds(now, entry.observed_at) <= window_seconds {
                retained.push_back(entry);
            }
        }

        self.recent_messages = retained;

        if let Some(last_action_at) = self.last_action_at {
            if elapsed_seconds(now, last_action_at) > ACTION_COOLDOWN_SECONDS {
                self.last_action_at = None;
            }
        }
    }
}

impl MessageFingerprint {
    fn from_message(message: &Message) -> Option<Self> {
        let mut image_descriptors = message
            .attachments
            .iter()
            .filter(|attachment| is_image_attachment(attachment))
            .map(|attachment| {
                format!(
                    "{}:{}:{}:{}:{}",
                    attachment.filename.to_lowercase(),
                    attachment.size,
                    attachment.width.unwrap_or_default(),
                    attachment.height.unwrap_or_default(),
                    attachment.content_type.as_deref()
                        .unwrap_or("unknown")
                        .to_lowercase()
                )
            })
            .collect::<Vec<_>>();

        if image_descriptors.is_empty() {
            return None;
        }

        image_descriptors.sort_unstable();

        Some(Self {
            normalized_content: normalize_content(&message.content),
            image_descriptors,
        })
    }

    fn preview(&self) -> String {
        if self.normalized_content.is_empty() {
            return "(sin texto)".to_string();
        }

        truncate_text(&self.normalized_content, 120)
    }
}

fn normalize_content(content: &str) -> String {
    content.split_whitespace()
        .map(|part| part.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
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
    raw_value.strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(raw_value)
        .trim()
}

fn is_image_attachment(attachment: &Attachment) -> bool {
    attachment.content_type
        .as_deref()
        .map(|content_type| content_type.starts_with("image/"))
        .unwrap_or_else(|| attachment.width.is_some() && attachment.height.is_some())
}

fn elapsed_seconds(now: i64, past: i64) -> i64 {
    now.saturating_sub(past)
}

fn truncate_text(content: &str, max_chars: usize) -> String {
    let total_chars = content.chars().count();
    if total_chars <= max_chars {
        return content.to_string();
    }

    let mut truncated = content.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anti_spam() -> AntiSpam {
        AntiSpam {
            config: AntiSpamConfig {
                alert_channel_id: None,
                duplicate_window_seconds: 12,
                distinct_channels_threshold: 3,
            },
            state: Mutex::new(HashMap::new()),
        }
    }

    fn fingerprint(content: &str) -> MessageFingerprint {
        MessageFingerprint {
            normalized_content: normalize_content(content),
            image_descriptors: vec!["banner.png:1024:640:480:image/png".to_string()],
        }
    }

    fn observation(channel_id: u64, observed_at: i64) -> Observation {
        Observation {
            guild_id: GuildId::new(1),
            user_id: UserId::new(2),
            channel_id: ChannelId::new(channel_id),
            message_id: MessageId::new(channel_id * 10),
            observed_at,
            fingerprint: fingerprint("Promo Nitro"),
        }
    }

    #[test]
    fn detects_same_message_across_three_channels() {
        let anti_spam = anti_spam();

        assert!(anti_spam.observe(observation(10, 100)).is_none());
        assert!(anti_spam.observe(observation(11, 104)).is_none());

        let incident = anti_spam
            .observe(observation(12, 108))
            .expect("incident should be detected");

        assert_eq!(incident.repeated_messages, 3);
        assert_eq!(incident.channels.len(), 3);
        assert_eq!(incident.first_detected_at, 100);
        assert_eq!(incident.last_detected_at, 108);
    }

    #[test]
    fn ignores_duplicates_inside_same_channel_only() {
        let anti_spam = anti_spam();

        assert!(anti_spam.observe(observation(10, 100)).is_none());
        assert!(anti_spam.observe(observation(10, 103)).is_none());
        assert!(anti_spam.observe(observation(11, 106)).is_none());
    }

    #[test]
    fn ignores_old_messages_outside_window() {
        let anti_spam = anti_spam();

        assert!(anti_spam.observe(observation(10, 100)).is_none());
        assert!(anti_spam.observe(observation(11, 120)).is_none());
        assert!(anti_spam.observe(observation(12, 121)).is_none());
    }
}
