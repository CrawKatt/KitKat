mod antispam;
mod buttons;
mod interactions;

pub use buttons::*;

use crate::audio::ready_event;
use crate::commands::*;
use antispam::AntiSpam;
use lavalink_rs::model::UserId;
use lavalink_rs::model::events::Events;
use lavalink_rs::prelude::{LavalinkClient, NodeBuilder, NodeDistributionStrategy};
use poise::{
    Command, EditTracker, Framework, FrameworkContext, FrameworkError, serenity_prelude as serenity,
};
use reqwest::Client as HttpClient;
use serenity::all::{FullEvent, Ready};
use serenity::prelude::TypeMapKey;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
pub struct HttpKey;

impl TypeMapKey for HttpKey {
    type Value = HttpClient;
}

pub struct Data {
    pub lavalink: LavalinkClient,
    pub antispam: AntiSpam,
}

pub type CommandResult = Result<(), Error>;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;

pub fn create_framework() -> Framework<Data, Error> {
    Framework::builder()
        .options(poise::FrameworkOptions {
            commands: load_commands(),
            prefix_options: poise::PrefixFrameworkOptions {
                prefix: Some("kitkat".to_lowercase()),
                additional_prefixes: vec![poise::Prefix::Literal("KitKat"), poise::Prefix::Literal(">>")],
                edit_tracker: Some(Arc::from(EditTracker::for_timespan(Duration::from_hours(1)))),
                ..Default::default()
            },
            on_error: |error| Box::pin(error_handler(error)),
            event_handler: |ctx, event, framework, data| Box::pin(event_handler(ctx, event, framework, data)),
            allowed_mentions: Some(
                serenity::CreateAllowedMentions::default()
                    .all_users(true)
                    .all_roles(true),
            ),
            ..Default::default()
        })
        .setup(|ctx, ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                lavalink_handler(ready).await
            })
        }).build()
}

async fn lavalink_handler(ready: &Ready) -> Result<Data, Error> {
    let lavalink_host = dotenvy::var("LAVALINK_HOST").expect("missing LAVALINK_HOST");
    let lavalink_port = dotenvy::var("LAVALINK_PORT").expect("missing LAVALINK_PORT");
    let lavalink_password = dotenvy::var("LAVALINK_PASSWORD").expect("missing LAVALINK_PASSWORD");
    let antispam = AntiSpam::load()?;

    let node = NodeBuilder {
        hostname: format!("{lavalink_host}:{lavalink_port}"),
        is_ssl: true,
        events: Events::default(),
        password: lavalink_password,
        user_id: UserId(ready.user.id.get()),
        session_id: None,
    };

    let events = Events {
        ready: Some(ready_event),
        ..Default::default()
    };

    println!("Lavalink connecting to {lavalink_host}:{lavalink_port} (SSL: true)...");

    let lavalink: LavalinkClient = LavalinkClient::new(
        events,
        vec![node],
        NodeDistributionStrategy::round_robin()
    ).await;

    let data = Data {
        lavalink,
        antispam
    };

    println!("Lavalink client initialized.");

    Ok(data)
}

async fn event_handler(
    ctx: &serenity::Context,
    event: &FullEvent,
    framework: FrameworkContext<'_, Data, Error>,
    data: &Data,
) -> CommandResult {
    match event {
        FullEvent::Ready { data_about_bot } => {
            println!("Logged in as {}", data_about_bot.user.name)
        }
        FullEvent::Message { new_message } => {
            if let Some(incident) = data.antispam.observe_message(new_message) {
                data.antispam.enforce(ctx, incident).await?;
            }
        }
        FullEvent::InteractionCreate { interaction } => {
            interactions::handler(ctx, interaction, &framework).await?
        }
        _ => ()
    }

    Ok(())
}

pub async fn error_handler(error: FrameworkError<'_, Data, Error>) {
    match error {
        FrameworkError::Setup { error, .. } => {
            println!("Error al iniciar el Bot: {error:?}");
            panic!("Error al iniciar el Bot:")
        },
        FrameworkError::Command { error, ctx, ..} => eprintln!("Error en comando `{}` : {:?}", ctx.command().name, error),
        FrameworkError::EventHandler { error, event, .. } => eprintln!("Error en el evento: {:?} Causa del error: {error:?}", event.snake_case_name()),
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                eprintln!("Error al manejar el error: {e}");
            }
        }
    }
}

pub fn load_commands() -> Vec<Command<Data, Error>> {
    vec![
        translate(),
        join(),
        leave(),
        play(),
        pause(),
        resume(),
        skip(),
        stop(),
        queue()
    ]
}