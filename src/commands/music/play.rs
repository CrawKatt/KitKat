use crate::commands::music::try_join;
use crate::utils::{CommandResult, Context};
use lavalink_rs::prelude::{SearchEngines, TrackInQueue, TrackLoadData};
use serenity::all::{CreateEmbed, CreateEmbedAuthor, CreateMessage};

#[poise::command(
    prefix_command,
    slash_command,
    guild_only,
    user_cooldown = 10,
    category = "Audio",
    aliases("p"),
)]
pub async fn play(
    ctx: Context<'_>,
    #[rest]
    query: String
) -> CommandResult {
    let guild = ctx.guild().unwrap().clone();
    let guild_id = guild.id;

    try_join(ctx, guild).await?;

    let author_name = ctx.author_member().await
        .ok_or("Failed to get author member")?
        .display_name()
        .to_string();

    let author_face = ctx.author_member().await
        .ok_or("Failed to get author member")?
        .face();

    let message = ctx.say("Buscando...").await?;

    let lavalink = &ctx.data().lavalink;

    let query = if query.starts_with("http") {
        query
    } else {
        SearchEngines::YouTube.to_query(&query)?
    };

    let loaded_tracks = lavalink.load_tracks(guild_id.get(), &query).await?;
    let mut tracks = match loaded_tracks.data {
        Some(TrackLoadData::Track(track)) => vec![track],
        Some(TrackLoadData::Search(result)) => {
            if result.is_empty() {
                ctx.say("No se encontraron resultados").await?;
                return Ok(());
            }
            vec![result[0].clone()]
        }
        Some(TrackLoadData::Playlist(playlist)) => {
            if playlist.tracks.is_empty() {
                ctx.say("La lista de reproducción está vacía").await?;
                return Ok(());
            }
            playlist.tracks
        },
        _ => {
            ctx.say("No se encontraron resultados").await?;
            return Ok(());
        }
    };

    let player_ctx = lavalink.get_player_context(guild_id.get()).ok_or("No se pudo obtener el contexto del reproductor")?;
    let player = player_ctx.get_player().await?;
    let is_playing = player.track.is_some();

    for track in &mut tracks {
        track.user_data = Some(serde_json::json!({"requester_id": ctx.author().id.get()}))
    }

    let track_to_show = tracks[0].clone();
    let tracks_to_queue: Vec<TrackInQueue> = tracks.into_iter().map(Into::into).collect();

    for track in tracks_to_queue {
        player_ctx.queue(track)?;
    }

    if !is_playing {
        player_ctx.skip()?;
    }

    let song_name = if is_playing {
        format!("{} Añadido a la cola", track_to_show.info.title)
    } else {
        format!("Reproduciendo {}", track_to_show.info.title)
    };

    let thumbnail = track_to_show.info.artwork_url.clone().unwrap_or_default();

    message.delete(ctx).await?;
    let desc = format!("**Solicitado por:** {author_name}");
    let embed = CreateEmbed::new()
        .title(song_name)
        .author(CreateEmbedAuthor::new(author_name)
            .icon_url(author_face))
        .description(desc)
        .thumbnail(thumbnail)
        .color(0x00ff_0000);

    let builder = CreateMessage::new().embed(embed);
    ctx.channel_id().send_message(ctx.http(), builder).await?;

    Ok(())
}