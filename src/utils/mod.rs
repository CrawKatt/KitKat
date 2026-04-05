use std::sync::Arc;
use std::time::Duration;
use poise::{serenity_prelude as serenity, Framework};
use poise::{Command, EditTracker};
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

pub fn load_commands() -> Vec<Command<Data, Error>> {
    vec![
        translate()
    ]
}