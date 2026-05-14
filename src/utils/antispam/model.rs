use std::collections::VecDeque;

use serenity::all::{Attachment, ChannelId, GuildId, Message, MessageId, UserId};

pub const ACTION_COOLDOWN_SECONDS: i64 = 60;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TrackerKey {
    pub guild_id: GuildId,
    pub user_id: UserId,
}

#[derive(Default)]
pub struct UserTracker {
    recent_messages: VecDeque<ObservedMessage>,
    last_action_at: Option<i64>,
}

#[derive(Clone)]
pub struct ObservedMessage {
    channel_id: ChannelId,
    observed_at: i64,
    fingerprint: MessageFingerprint,
}

#[derive(Clone, Debug)]
pub struct SpamIncident {
    pub guild_id: GuildId,
    pub user_id: UserId,
    pub trigger_message_id: MessageId,
    pub trigger_channel_id: ChannelId,
    pub repeated_messages: usize,
    pub channels: Vec<ChannelId>,
    pub first_detected_at: i64,
    pub last_detected_at: i64,
    pub preview: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    pub guild_id: GuildId,
    pub user_id: UserId,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub observed_at: i64,
    pub fingerprint: MessageFingerprint,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MessageFingerprint {
    pub normalized_content: String,
    pub image_descriptors: Vec<String>,
}

impl UserTracker {
    pub fn prune(&mut self, now: i64, window_seconds: i64) {
        self.recent_messages = self
            .recent_messages
            .drain(..)
            .filter(|entry| elapsed_seconds(now, entry.observed_at) <= window_seconds)
            .collect();

        if self
            .last_action_at
            .is_some_and(|last_action_at| elapsed_seconds(now, last_action_at) > ACTION_COOLDOWN_SECONDS)
        {
            self.last_action_at = None;
        }
    }

    pub fn is_on_cooldown(&self, observed_at: i64) -> bool {
        self.last_action_at.is_some_and(|last_action_at| elapsed_seconds(observed_at, last_action_at) <= ACTION_COOLDOWN_SECONDS)
    }

    pub fn record(&mut self, observation: &Observation) {
        self.recent_messages.push_back(ObservedMessage {
            channel_id: observation.channel_id,
            observed_at: observation.observed_at,
            fingerprint: observation.fingerprint.clone(),
        });
    }

    pub fn matching_messages(
        &self,
        fingerprint: &MessageFingerprint,
    ) -> impl Iterator<Item = &ObservedMessage> {
        self.recent_messages
            .iter()
            .filter(move |message| &message.fingerprint == fingerprint)
    }

    pub fn mark_action(&mut self, observed_at: i64) {
        self.last_action_at = Some(observed_at);
        self.recent_messages.clear();
    }
}

impl Observation {
    pub fn from_message(message: &Message) -> Option<Self> {
        if message.author.bot || message.webhook_id.is_some() {
            return None;
        }

        Some(Self {
            guild_id: message.guild_id?,
            user_id: message.author.id,
            channel_id: message.channel_id,
            message_id: message.id,
            observed_at: message.timestamp.unix_timestamp(),
            fingerprint: MessageFingerprint::from_message(message)?,
        })
    }
}

impl MessageFingerprint {
    pub fn from_message(message: &Message) -> Option<Self> {
        let mut image_descriptors = message
            .attachments
            .iter()
            .filter(|attachment| is_image_attachment(attachment))
            .map(image_descriptor)
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

    pub fn preview(&self) -> String {
        if self.normalized_content.is_empty() {
            return "(sin texto)".to_string();
        }

        truncate_text(&self.normalized_content, 120)
    }
}

impl ObservedMessage {
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    pub fn observed_at(&self) -> i64 {
        self.observed_at
    }
}

fn image_descriptor(attachment: &Attachment) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        attachment.filename.to_lowercase(),
        attachment.size,
        attachment.width.unwrap_or_default(),
        attachment.height.unwrap_or_default(),
        attachment.content_type
            .as_deref()
            .unwrap_or("unknown")
            .to_lowercase()
    )
}

fn normalize_content(content: &str) -> String {
    content
        .split_whitespace()
        .map(|part| part.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
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
