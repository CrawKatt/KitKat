use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::anyhow;
use poise::serenity_prelude as serenity;
use rusqlite::{params, Connection, OptionalExtension};
use serenity::all::{ChannelId, CreateEmbed, CreateEmbedFooter, CreateMessage, GuildId, Message, MessageId, Timestamp};
use serenity::utils::{FormattedTimestamp, FormattedTimestampStyle};
use serde_json::json;

use super::Error;

const DEFAULT_DATABASE_PATH: &str = "kitkat_deleted_messages.sqlite3";

pub struct DeletedMessageLogger {
    config: DeletedMessageConfig,
    database: Mutex<Connection>,
}

#[derive(Clone, Debug)]
struct DeletedMessageConfig {
    log_channels: HashMap<GuildId, ChannelId>,
    database_path: PathBuf,
}

struct StoredMessage {
    guild_id: Option<u64>,
    channel_id: u64,
    message_id: u64,
    author_id: u64,
    author_name: String,
    content: String,
    attachments_json: String,
    attachments_summary: String,
    message_created_at: i64,
}

impl DeletedMessageLogger {
    pub fn load() -> Result<Self, Error> {
        let config = DeletedMessageConfig::load()?;
        let logger = Self {
            database: Mutex::new(Connection::open(&config.database_path)?),
            config,
        };
        logger.initialize()?;

        println!(
            "DeletedMessageLogger initialized. configured_guilds={}, database_path={}",
            logger.config.log_channels.len(),
            logger.config.database_path.display()
        );

        Ok(logger)
    }

    pub fn remember_message(&self, message: &Message) {
        if self.config.log_channel_for(message.guild_id) == Some(message.channel_id) {
            return;
        }

        let stored = StoredMessage::from_message(message);
        if let Err(error) = self.upsert_message(&stored) {
            println!(
                "DeletedMessageLogger: no se pudo guardar mensaje {}: {}",
                stored.message_id,
                error
            );
        }
    }

    pub async fn log_deleted_message(
        &self,
        ctx: &serenity::Context,
        guild_id: Option<GuildId>,
        channel_id: ChannelId,
        message_id: MessageId,
    ) {
        let deleted_at = Timestamp::now().unix_timestamp();
        let message = match self.mark_deleted(message_id, deleted_at) {
            Ok(message) => message,
            Err(error) => {
                println!("DeletedMessageLogger: no se pudo consultar mensaje borrado {}: {}", message_id.get(), error);
                return;
            }
        };

        let (guild_id, embed) = match message {
            Some(message) => (
                message.guild_id.map(GuildId::new).or(guild_id),
                deletion_log_embed(&message, deleted_at),
            ),
            None => (
                guild_id,
                missing_message_log_embed(guild_id, channel_id, message_id, deleted_at),
            ),
        };

        self.send_log(ctx, guild_id, message_id, embed).await;
    }

    pub async fn log_bulk_deleted_messages(
        &self,
        ctx: &serenity::Context,
        guild_id: Option<GuildId>,
        channel_id: ChannelId,
        message_ids: &[MessageId],
    ) {
        for message_id in message_ids {
            self.log_deleted_message(ctx, guild_id, channel_id, *message_id).await;
        }
    }

    fn initialize(&self) -> Result<(), Error> {
        let database = self
            .database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        database.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS observed_messages (
                message_id TEXT PRIMARY KEY,
                guild_id TEXT,
                channel_id TEXT NOT NULL,
                author_id TEXT NOT NULL,
                author_name TEXT NOT NULL,
                content TEXT NOT NULL,
                attachments_json TEXT NOT NULL,
                attachments_summary TEXT NOT NULL,
                message_created_at INTEGER NOT NULL,
                observed_at INTEGER NOT NULL,
                deleted_at INTEGER
            );
            ",
        )?;

        Ok(())
    }

    fn upsert_message(&self, message: &StoredMessage) -> Result<(), rusqlite::Error> {
        let database = self
            .database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        database.execute(
            "
            INSERT INTO observed_messages (
                message_id,
                guild_id,
                channel_id,
                author_id,
                author_name,
                content,
                attachments_json,
                attachments_summary,
                message_created_at,
                observed_at,
                deleted_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)
            ON CONFLICT(message_id) DO UPDATE SET
                guild_id = excluded.guild_id,
                channel_id = excluded.channel_id,
                author_id = excluded.author_id,
                author_name = excluded.author_name,
                content = excluded.content,
                attachments_json = excluded.attachments_json,
                attachments_summary = excluded.attachments_summary,
                message_created_at = excluded.message_created_at,
                observed_at = excluded.observed_at
            ",
            params![
                message.message_id.to_string(),
                message.guild_id.map(|id| id.to_string()),
                message.channel_id.to_string(),
                message.author_id.to_string(),
                message.author_name,
                message.content,
                message.attachments_json,
                message.attachments_summary,
                message.message_created_at,
                Timestamp::now().unix_timestamp(),
            ],
        )?;

        Ok(())
    }

    fn mark_deleted(
        &self,
        message_id: MessageId,
        deleted_at: i64,
    ) -> Result<Option<StoredMessage>, rusqlite::Error> {
        let database = self
            .database
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        database.execute(
            "UPDATE observed_messages SET deleted_at = ?1 WHERE message_id = ?2",
            params![deleted_at, message_id.get().to_string()],
        )?;

        database
            .query_row(
                "
                SELECT guild_id, channel_id, message_id, author_id, author_name, content, attachments_summary, message_created_at
                FROM observed_messages
                WHERE message_id = ?1
                ",
                params![message_id.get().to_string()],
                |row| {
                    Ok(StoredMessage {
                        guild_id: row
                            .get::<_, Option<String>>(0)?
                            .and_then(|value| value.parse::<u64>().ok()),
                        channel_id: row.get::<_, String>(1)?.parse::<u64>().unwrap_or_default(),
                        message_id: row.get::<_, String>(2)?.parse::<u64>().unwrap_or_default(),
                        author_id: row.get::<_, String>(3)?.parse::<u64>().unwrap_or_default(),
                        author_name: row.get(4)?,
                        content: row.get(5)?,
                        attachments_json: String::new(),
                        attachments_summary: row.get(6)?,
                        message_created_at: row.get(7)?,
                    })
                },
            )
            .optional()
    }

    async fn send_log(
        &self,
        ctx: &serenity::Context,
        guild_id: Option<GuildId>,
        message_id: MessageId,
        embed: CreateEmbed,
    ) {
        let Some(log_channel_id) = self.config.log_channel_for(guild_id) else {
            return;
        };

        if let Err(error) = log_channel_id.send_message(&ctx.http, CreateMessage::new().embed(embed)).await {
            println!("DeletedMessageLogger: no se pudo enviar log del mensaje {} al canal {}: {}", message_id.get(), log_channel_id.get(), error);
        }
    }
}

impl DeletedMessageConfig {
    fn load() -> Result<Self, Error> {
        let path = dotenvy::var("KITKAT_CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
        let path = PathBuf::from(path);

        match fs::read_to_string(&path) {
            Ok(raw_config) => Self::parse(&raw_config),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
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

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                return Err(anyhow!("Linea {line_number}: se esperaba key = value").into());
            };

            let key = raw_key.trim();
            let value = raw_value.trim();

            match current_section.as_str() {
                "deleted_messages" => match key {
                    "database_path" => {
                        config.database_path = PathBuf::from(trim_quotes(value));
                    }
                    "log_channel_id" => {
                        println!(
                            "DeletedMessageLogger: [deleted_messages].log_channel_id es global y sera ignorado. Configura [deleted_messages.log_channels] con guild_id = channel_id."
                        );
                    }
                    _ => {}
                }
                "deleted_messages.log_channels" => {
                    let guild_id = GuildId::new(parse_u64(key, line_number, "guild_id")?);
                    let channel_id = ChannelId::new(parse_u64(value, line_number, key)?);
                    config.log_channels.insert(guild_id, channel_id);
                }
                _ => continue,
            }
        }

        Ok(config)
    }

    fn log_channel_for(&self, guild_id: Option<GuildId>) -> Option<ChannelId> {
        guild_id.and_then(|guild_id| self.log_channels.get(&guild_id).copied())
    }
}

impl Default for DeletedMessageConfig {
    fn default() -> Self {
        Self {
            log_channels: HashMap::new(),
            database_path: PathBuf::from(DEFAULT_DATABASE_PATH),
        }
    }
}

impl StoredMessage {
    fn from_message(message: &Message) -> Self {
        let attachments = message
            .attachments
            .iter()
            .map(|attachment| {
                json!({
                    "id": attachment.id.get().to_string(),
                    "filename": attachment.filename,
                    "url": attachment.url,
                    "proxy_url": attachment.proxy_url,
                    "content_type": attachment.content_type,
                    "size": attachment.size,
                    "width": attachment.width,
                    "height": attachment.height,
                })
            })
            .collect::<Vec<_>>();

        Self {
            guild_id: message.guild_id.map(GuildId::get),
            channel_id: message.channel_id.get(),
            message_id: message.id.get(),
            author_id: message.author.id.get(),
            author_name: message.author.name.clone(),
            content: message.content.clone(),
            attachments_json: serde_json::to_string(&attachments).unwrap_or_else(|_| "[]".to_string()),
            attachments_summary: summarize_attachments(&attachments),
            message_created_at: message.timestamp.unix_timestamp(),
        }
    }
}

fn deletion_log_embed(message: &StoredMessage, deleted_at: i64) -> CreateEmbed {
    CreateEmbed::new()
        .title("Mensaje borrado")
        .color(0xff3b30)
        .field(
            "Usuario",
            format!("<@{}> ({})", message.author_id, message.author_name),
            false,
        )
        .field("Canal", format!("<#{}>", message.channel_id), true)
        .field(
            "Servidor",
            message
                .guild_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "(desconocido)".to_string()),
            true,
        )
        .field("Mensaje ID", message.message_id.to_string(), false)
        .field("Creado", discord_timestamp(message.message_created_at), true)
        .field("Borrado", discord_timestamp(deleted_at), true)
        .field("Texto", truncate_embed_field(&visible_text(&message.content)), false)
        .field(
            "Adjuntos",
            truncate_embed_field(&visible_text(&message.attachments_summary)),
            false,
        )
        .footer(CreateEmbedFooter::new("KitKat deleted message log"))
}

fn missing_message_log_embed(
    guild_id: Option<GuildId>,
    channel_id: ChannelId,
    message_id: MessageId,
    deleted_at: i64,
) -> CreateEmbed {
    CreateEmbed::new()
        .title("Mensaje borrado")
        .color(0xff9500)
        .field("Canal", format!("<#{}>", channel_id.get()), true)
        .field(
            "Servidor",
            guild_id
                .map(|id| id.get().to_string())
                .unwrap_or_else(|| "(desconocido)".to_string()),
            true,
        )
        .field("Mensaje ID", message_id.get().to_string(), false)
        .field("Borrado", discord_timestamp(deleted_at), true)
        .field(
            "Contenido",
            "No disponible; el bot no habia registrado este mensaje antes del borrado.",
            false,
        )
        .footer(CreateEmbedFooter::new("KitKat deleted message log"))
}

fn summarize_attachments(attachments: &[serde_json::Value]) -> String {
    if attachments.is_empty() {
        return "(sin adjuntos)".to_string();
    }

    attachments
        .iter()
        .take(5)
        .map(|attachment| {
            let filename = attachment
                .get("filename")
                .and_then(|value| value.as_str())
                .unwrap_or("archivo");
            let url = attachment
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("(sin url)");

            format!("{filename}: {url}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn visible_text(value: &str) -> String {
    if value.trim().is_empty() {
        return "(vacio)".to_string();
    }

    value.to_string()
}

fn discord_timestamp(unix_timestamp: i64) -> String {
    Timestamp::from_unix_timestamp(unix_timestamp)
        .map(|timestamp| {
            FormattedTimestamp::new(timestamp, Some(FormattedTimestampStyle::LongDateTime))
                .to_string()
        })
        .unwrap_or_else(|_| unix_timestamp.to_string())
}

fn truncate_embed_field(content: &str) -> String {
    const MAX_EMBED_FIELD_CHARS: usize = 1000;

    if content.chars().count() <= MAX_EMBED_FIELD_CHARS {
        return content.to_string();
    }

    let mut truncated = content
        .chars()
        .take(MAX_EMBED_FIELD_CHARS)
        .collect::<String>();
    truncated.push_str("\n...");
    truncated
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

fn trim_quotes(raw_value: &str) -> &str {
    raw_value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(raw_value)
        .trim()
}
