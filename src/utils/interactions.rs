use poise::{serenity_prelude as serenity, FrameworkContext};
use serenity::all::{CacheHttp, Context, Interaction};

use crate::utils::{handle_action, handle_and_update, ButtonAction, CommandResult, Data, Error};

/// # Esta función maneja las interacciones de botones
/// - `mc`: La interacción de componente
/// - `ButtonAction`: Enumeración de acciones de botones
pub async fn handler(
    ctx: &Context,
    interaction: &Interaction,
    _framework: &FrameworkContext<'_, Data, Error>
) -> CommandResult {
    let Some(mc) = interaction.as_message_component() else { return Ok(()) };
    let guild_id = mc.guild_id.ok_or("No se pudo obtener el ID del servidor")?;
    let custom_id = mc.data.custom_id.parse::<ButtonAction>().unwrap_or(ButtonAction::Unknown);

    match ButtonAction::from(custom_id) {
        ButtonAction::Close => ctx.http().delete_message(mc.channel_id, mc.message.id, None).await?,
        ButtonAction::Skip => handle_action(ctx, guild_id, mc, "Se ha saltado la canción", |queue| queue.skip()).await?,
        ButtonAction::Pause => handle_and_update(ctx, guild_id, mc, "Se ha pausado la canción", |queue| queue.pause(),true).await?,
        ButtonAction::Resume => handle_and_update(ctx, guild_id, mc, "Se ha reanudado la canción", |queue| queue.resume(), false).await?,
        ButtonAction::Stop => handle_action(ctx, guild_id, mc, "Se ha detenido la canción", |queue| { queue.stop(); Ok(()) }).await?,
        ButtonAction::Unknown => ()
    }

    Ok(())
}