mod play;
mod pause;
mod resume;
mod stop;
mod skip;
mod queue;

use lavalink_rs::model::player::ConnectionInfo;
use lavalink_rs::model::ChannelId as LavalinkChannelId;
pub use play::*;
pub use pause::*;
pub use resume::*;
pub use stop::*;
pub use skip::*;
pub use queue::*;

use serenity::all::{ChannelId, GetMessages, Guild, GuildId};
use crate::utils::{CommandResult, Context, Error};

#[poise::command(
    prefix_command,
    slash_command,
    category = "Audio",
    user_cooldown = 10,
    guild_only,
)]
pub async fn join(ctx: Context<'_>) -> CommandResult {
    let (guild_id, channel_id) = get_guild_id_and_channel_id(ctx).await?;

    let Some(connect_to) = channel_id else {
        ctx.say("No estás en un canal de voz para unirme").await?;
        return Ok(())
    };

    let maybe_manager = songbird::get(ctx.serenity_context()).await;
    let Some(manager) = maybe_manager else {
        ctx.say("No pude unirme a un canal de voz").await?;
        return Ok(())
    };

    manager.join(guild_id, connect_to).await?;

    Ok(())
}

#[poise::command(
    prefix_command,
    slash_command,
    category = "Audio",
    user_cooldown = 10,
    guild_only,
)]
pub async fn leave(ctx: Context<'_>) -> CommandResult {
    let (guild_id, channel_id) = get_guild_id_and_channel_id(ctx).await?;

    if channel_id.is_none() {
        ctx.say("No estoy en un canal de voz para salir").await?;
        return Ok(())
    }

    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("No pude obtener el Songbird VoiceManager")?;

    manager.remove(guild_id).await?;

    let lavalink = &ctx.data().lavalink;
    let _ = lavalink.delete_player(guild_id.get()).await;

    Ok(())
}

pub async fn try_join(ctx: Context<'_>, guild: Guild) -> CommandResult {
    let channel_id = guild
        .voice_states
        .get(&ctx.author().id)
        .and_then(|voice_state| voice_state.channel_id)
        .ok_or("User is not in a voice channel")?;

    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("No se pudo obtener el Songbird VoiceManager")?;

    let lavalink = &ctx.data().lavalink;
    if lavalink.get_player_context(guild.id.get()).is_none() {
        let (connection_info, _) = manager.join_gateway(guild.id, channel_id).await?;
        let lava_info = ConnectionInfo {
            session_id: connection_info.session_id,
            token: connection_info.token,
            endpoint: connection_info.endpoint,
            channel_id: Some(LavalinkChannelId::from(channel_id.get())),
        };

        lavalink.create_player_context(guild.id.get(), lava_info).await?;
    }

    Ok(())
}

pub async fn get_guild_id_and_channel_id(ctx: Context<'_>) -> Result<(GuildId, Option<ChannelId>), Error> {
    let messages = ctx.channel_id().messages(&ctx.http(), GetMessages::default()).await?;
    let msg = messages.first().ok_or("Could not find the message that triggered the command")?;

    let guild = ctx.guild().unwrap();
    let channel_id = guild
        .voice_states
        .get(&msg.author.id)
        .and_then(|voice_state| voice_state.channel_id);

    Ok((guild.id, channel_id))
}