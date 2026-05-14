use serenity::all::ChannelId;

use super::model::{Observation, SpamIncident, UserTracker};

pub fn detect_incident(
    tracker: &UserTracker,
    observation: &Observation,
    distinct_channels_threshold: usize,
) -> Option<SpamIncident> {
    let matching_messages = tracker
        .matching_messages(&observation.fingerprint)
        .collect::<Vec<_>>();

    let channels = distinct_channels(&matching_messages);
    if channels.len() < distinct_channels_threshold {
        return None;
    }

    let repeated_messages = matching_messages.len();
    let first_detected_at = matching_messages
        .iter()
        .map(|message| message.observed_at())
        .min()
        .unwrap_or(observation.observed_at);

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

fn distinct_channels(messages: &[&super::model::ObservedMessage]) -> Vec<ChannelId> {
    messages.iter().fold(Vec::new(), |mut channels, message| {
        let channel_id = message.channel_id();
        if !channels.contains(&channel_id) {
            channels.push(channel_id);
        }
        channels
    })
}
