use std::sync::Arc;
use std::time::Duration;
use poise::{serenity_prelude as serenity, Framework, FrameworkContext, FrameworkError};
use poise::{Command, EditTracker};
use serenity::all::FullEvent;
use crate::commands::translate;

pub struct Data;

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
            event_handler: |ctx, event, framework, _data| Box::pin(event_handler(ctx, event, framework)),
            allowed_mentions: Some(serenity::CreateAllowedMentions::default()
                .all_users(true)
                .all_roles(true)
            ),
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data)
            })
        }).build()
}

async fn event_handler(
    _ctx: &serenity::Context,
    event: &FullEvent,
    _framework: FrameworkContext<'_, Data, Error>
) -> CommandResult {
    match event {
        FullEvent::Ready { data_about_bot } => println!("Logged in as {}", data_about_bot.user.name),
        _ => { println!("Unhandled event: {:?}", event.snake_case_name()) }
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
        translate()
    ]
}