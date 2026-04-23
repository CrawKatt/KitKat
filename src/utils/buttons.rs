use poise::serenity_prelude as serenity;
use songbird::error::TrackResult;
use serenity::all::{ComponentInteraction, Context, CreateActionRow, CreateButton, CreateInteractionResponse, CreateInteractionResponseMessage, GuildId};
use strum_macros::EnumString;
use crate::utils::CommandResult;

#[derive(PartialEq, Eq, EnumString)]
pub enum ButtonAction {
    #[strum(serialize = "skip")]
    Skip,
    #[strum(serialize = "stop")]
    Stop,
    #[strum(serialize = "pause")]
    Pause,
    #[strum(serialize = "resume")]
    Resume,
    #[strum(serialize = "close")]
    Close,
    Unknown,
}

pub async fn update_button(ctx: &Context, mc: &ComponentInteraction, is_paused: bool) -> CommandResult {
    let buttons = generate_row(is_paused);
    let components = vec![buttons];

    let response = CreateInteractionResponseMessage::new().components(components);
    mc.create_response(&ctx, CreateInteractionResponse::UpdateMessage(response)).await?;

    Ok(())
}

pub async fn handle_action<F>(
    ctx: &Context,
    guild_id: GuildId,
    mc: &ComponentInteraction,
    message: &str,
    action: F
) -> CommandResult
where
    F: FnOnce(&mut songbird::tracks::TrackQueue) -> TrackResult<()> + Send,
{
    let songbird = songbird::get(ctx).await.ok_or("No se pudo obtener el Songbird VoiceManager")?;

    let Some(call) = songbird.get(guild_id) else {
        let response = CreateInteractionResponseMessage::new().content("No estás en un canal de voz");
        mc.create_response(&ctx, CreateInteractionResponse::Message(response)).await?;

        return Ok(());
    };

    let custom_id = ButtonAction::from(mc.data.custom_id.parse::<ButtonAction>().unwrap_or(ButtonAction::Unknown));
    let queue = call.lock().await.queue().current_queue();
    if (custom_id == ButtonAction::Skip && queue.is_empty()) || (custom_id == ButtonAction::Stop && queue.len() <= 1) {
        let error_message = match custom_id {
            ButtonAction::Skip => "No hay canciones en la cola",
            ButtonAction::Stop => "No hay más canciones en la cola",
            _ => "Error desconocido",
        };

        let response = CreateInteractionResponseMessage::new().content(error_message);
        mc.create_response(&ctx, CreateInteractionResponse::Message(response)).await?;

        return Ok(());
    }

    let caller = call.lock().await;
    let mut queue = caller.queue().clone();
    action(&mut queue)?;

    if let ButtonAction::Pause | ButtonAction::Resume = ButtonAction::from(mc.data.custom_id.parse::<ButtonAction>().unwrap_or(ButtonAction::Unknown)) {
        drop(caller);
        return Ok(())
    }

    let response = CreateInteractionResponseMessage::new().content(message);
    mc.create_response(&ctx, CreateInteractionResponse::Message(response)).await?;

    drop(caller);

    Ok(())
}

pub async fn handle_and_update<F>(
    ctx: &Context,
    guild_id: GuildId,
    mc: &ComponentInteraction,
    message: &str,
    action: F,
    is_paused: bool
) -> CommandResult
where
    F: FnOnce(&mut songbird::tracks::TrackQueue) -> TrackResult<()> + Send,
{
    handle_action(ctx, guild_id, mc, message, action).await?;
    update_button(ctx, mc, is_paused).await?;

    Ok(())
}

pub fn generate_row(is_paused: bool) -> CreateActionRow {
    let skip = CreateButton::new("skip")
        .label("Saltar")
        .emoji('⏭');

    let stop = CreateButton::new("stop")
        .label("Detener")
        .emoji('⏹');

    let play_pause = if is_paused {
        CreateButton::new("resume")
            .label("Reanudar")
            .emoji('▶')
    } else {
        CreateButton::new("pause")
            .label("Pausar")
            .emoji('⏸')
    };

    CreateActionRow::Buttons(vec![stop, play_pause, skip])
}