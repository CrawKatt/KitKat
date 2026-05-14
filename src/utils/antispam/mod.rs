mod cleanup;
mod config;
mod detector;
mod model;

use std::collections::HashMap;
use std::sync::Mutex;

use poise::serenity_prelude as serenity;
use serenity::all::{CreateMessage, Message, Timestamp};

use super::Error;
use cleanup::delete_user_messages;
use config::AntiSpamConfig;
use detector::detect_incident;
use model::{Observation, SpamIncident, TrackerKey, UserTracker};

const TIMEOUT_SECONDS: i64 = 7 * 24 * 60 * 60;

pub struct AntiSpam {
    config: AntiSpamConfig,
    state: Mutex<HashMap<TrackerKey, UserTracker>>,
}

impl AntiSpam {
    pub fn load() -> Result<Self, Error> {
        let config = AntiSpamConfig::load()?;
        println!(
            "AntiSpam initialized. alert_channel_id={:?}, duplicate_window_seconds={}, distinct_channels_threshold={}",
            config.alert_channel_id.map(serenity::all::ChannelId::get),
            config.duplicate_window_seconds,
            config.distinct_channels_threshold
        );

        Ok(Self {
            config,
            state: Mutex::new(HashMap::new()),
        })
    }

    pub fn observe_message(&self, message: &Message) -> Option<SpamIncident> {
        Observation::from_message(message).and_then(|observation| self.observe(observation))
    }

    pub async fn enforce(
        &self,
        ctx: &serenity::Context,
        incident: SpamIncident,
    ) -> Result<(), Error> {
        let timeout_until =
            Timestamp::from_unix_timestamp(Timestamp::now().unix_timestamp() + TIMEOUT_SECONDS)?;
        let mut member = incident.guild_id.member(ctx, incident.user_id).await?;
        let timeout_result = member.disable_communication_until_datetime(ctx, timeout_until).await;
        let timeout_error = timeout_result.as_ref().err().map(|error| error.to_string());
        let deleted_messages =
            delete_user_messages(ctx, incident.guild_id, incident.user_id, &incident.channels).await;

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

        tracker.prune(observation.observed_at, self.config.duplicate_window_seconds);
        if tracker.is_on_cooldown(observation.observed_at) {
            return None;
        }

        tracker.record(&observation);

        let incident = detect_incident(
            tracker,
            &observation,
            self.config.distinct_channels_threshold,
        )?;

        tracker.mark_action(observation.observed_at);
        Some(incident)
    }

    async fn send_alert(
        &self,
        ctx: &serenity::Context,
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

        let duration_seconds = incident
            .last_detected_at
            .saturating_sub(incident.first_detected_at);
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
        let deletion_status = format!("mensajes borrados tras sancion: {}", deleted_messages);

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

        alert_channel_id
            .send_message(&ctx.http, CreateMessage::new().content(content))
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::MessageFingerprint;
    use poise::serenity_prelude::all::{ChannelId, GuildId, MessageId, UserId};

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
            normalized_content: content
                .split_whitespace()
                .map(|part| part.to_lowercase())
                .collect::<Vec<_>>()
                .join(" "),
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
