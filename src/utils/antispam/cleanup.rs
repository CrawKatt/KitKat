use std::collections::HashMap;
use std::time::Duration;

use poise::serenity_prelude as serenity;
use serenity::all::{ChannelId, ChannelType, GetMessages, GuildChannel, GuildId, UserId};
use tokio::time::sleep;

const CHANNEL_FETCH_LIMIT: u8 = 100;
const SANCTION_CLEANUP_PER_CHANNEL_LIMIT: usize = 3;
const SANCTION_CLEANUP_DELAY_MS: u64 = 1200;
const SANCTION_CLEANUP_PASSES: usize = 2;
const SANCTION_CLEANUP_RETRY_DELAY_MS: u64 = 1800;

pub async fn delete_user_messages(
    ctx: &serenity::Context,
    guild_id: GuildId,
    user_id: UserId,
    detected_channels: &[ChannelId],
) -> usize {
    sleep(Duration::from_millis(SANCTION_CLEANUP_DELAY_MS)).await;

    let mut deleted_messages = 0;
    for pass in 0..SANCTION_CLEANUP_PASSES {
        let Ok(channels) = guild_id.channels(&ctx.http).await else {
            return deleted_messages;
        };

        let ordered_channels = prioritize_channels(&channels, detected_channels);
        for channel in ordered_channels {
            deleted_messages += delete_user_messages_in_channel(ctx, channel, user_id, SANCTION_CLEANUP_PER_CHANNEL_LIMIT).await;
        }

        if pass + 1 < SANCTION_CLEANUP_PASSES {
            sleep(Duration::from_millis(SANCTION_CLEANUP_RETRY_DELAY_MS)).await;
        }
    }

    deleted_messages
}

fn prioritize_channels<'a>(
    channels: &'a HashMap<ChannelId, GuildChannel>,
    detected_channels: &[ChannelId],
) -> Vec<&'a GuildChannel> {
    let prioritized = detected_channels
        .iter()
        .filter_map(|channel_id| channels.get(channel_id));

    let remaining = channels.iter()
        .filter(|(channel_id, _)| !detected_channels.contains(channel_id))
        .map(|(_, channel)| channel);

    prioritized.chain(remaining).collect()
}

async fn delete_user_messages_in_channel(
    ctx: &serenity::Context,
    channel: &GuildChannel,
    user_id: UserId,
    per_channel_limit: usize,
) -> usize {
    if !is_supported_channel(channel.kind) {
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
            Err(_) => break
        };

        if messages.is_empty() {
            break;
        }

        before = messages.last().map(|message| message.id);

        for message in messages {
            if message.author.id != user_id {
                continue;
            }
            
            if channel.id.delete_message(&ctx.http, message.id).await.is_ok() {
                deleted_messages += 1;
            }

            if deleted_messages >= per_channel_limit {
                return deleted_messages;
            }
        }

        if before.is_none() {
            break;
        }
    }

    deleted_messages
}

fn is_supported_channel(kind: ChannelType) -> bool {
    matches!(
        kind,
        ChannelType::Text
            | ChannelType::News
            | ChannelType::PublicThread
            | ChannelType::PrivateThread
            | ChannelType::NewsThread
    )
}
