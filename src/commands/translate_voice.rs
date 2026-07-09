use crate::commands::create_ai_message;
use crate::utils::{CommandResult, Context, Error};
use poise::CreateReply;
use serenity::all::ChannelId;

#[poise::command(
    prefix_command,
    slash_command,
    guild_only,
    category = "Info",
    aliases("trv", "trvoice"),
    guild_cooldown = 8,
)]
pub async fn translate_voice(
    ctx: Context<'_>,
    #[description = "Texto en español a traducir y hablar en japonés"]
    #[rest]
    prompt: String,
) -> CommandResult {
    let loading = ctx.say("Traduciendo y preparando voz...").await?;

    let channel_id = user_voice_channel(ctx)?;
    let japanese = create_ai_message(prompt).await?;

    if japanese.trim().is_empty() {
        loading.edit(ctx, CreateReply::default().content("La traducción llegó vacía.")).await?;
        return Ok(());
    }

    let wav = ctx.data()
        .voicevox
        .synthesize(&japanese)
        .await
        .map_err(|e| Error::from(format!("Error de VOICEVOX: {e}")))?;

    play_wav_in_voice(ctx, channel_id, wav).await?;
    loading.edit(ctx, CreateReply::default().content(format!("🔊 **Traducción (ES→JA):**\n{japanese}"))).await?;

    Ok(())
}

fn user_voice_channel(ctx: Context<'_>) -> Result<ChannelId, Error> {
    let guild = ctx.guild().ok_or("Este comando solo funciona en un servidor")?;
    guild.voice_states
        .get(&ctx.author().id)
        .and_then(|vs| vs.channel_id)
        .ok_or_else(|| Error::from("Debes estar en un canal de voz para usar la traducción por voz".to_string()))
}

async fn play_wav_in_voice(
    ctx: Context<'_>,
    channel_id: ChannelId,
    wav: Vec<u8>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or("No se pudo obtener el guild")?;
    let lavalink = &ctx.data().lavalink;
    if lavalink.get_player_context(guild_id.get()).is_some() {
        let _ = lavalink.delete_player(guild_id.get()).await;
    }

    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("No se pudo obtener el Songbird VoiceManager")?;

    let call = manager.join(guild_id, channel_id).await?;
    {
        let mut handler = call.lock().await;
        handler.play_only_input(wav.into());
    }

    Ok(())
}
