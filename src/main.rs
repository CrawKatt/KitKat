use crate::utils::{create_framework, CommandResult, HttpKey};
use poise::serenity_prelude as serenity;
use songbird::SerenityInit;
use reqwest::Client as HttpClient;

mod commands;
mod utils;
mod audio;

#[tokio::main]
async fn main() -> CommandResult {
    let token = dotenvy::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set");
    let intents = serenity::GatewayIntents::all() | serenity::GatewayIntents::MESSAGE_CONTENT;
    let framework = create_framework();
    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .register_songbird()
        .type_map_insert::<HttpKey>(HttpClient::new())
        .await?;

    client.start().await?;

    Ok(())
}
