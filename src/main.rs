use crate::utils::{create_framework, CommandResult};
use poise::serenity_prelude as serenity;

mod commands;
mod utils;

#[tokio::main]
async fn main() -> CommandResult {
    let token = dotenvy::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set");
    let intents = serenity::GatewayIntents::all() | serenity::GatewayIntents::MESSAGE_CONTENT;
    let framework = create_framework();
    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;

    client.start().await?;

    Ok(())
}
