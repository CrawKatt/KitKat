use std::fs;
use once_cell::sync::Lazy;
use crate::utils::{CommandResult, Context, Error};
use openai_api_rs::v1::api::Client;
use openai_api_rs::v1::chat_completion;
use openai_api_rs::v1::chat_completion::{ChatCompletionRequest, ChatCompletionResponse};
use poise::CreateReply;
use regex::Regex;

const SYSTEM_PROMPT: Lazy<String> = Lazy::new(|| {
    fs::read_to_string("translate.md").expect("Failed to read system prompt")
});

#[poise::command(
    prefix_command,
    slash_command,
    guild_only,
    category = "Info",
    aliases("tr"),
    guild_cooldown = 5,
)]
pub async fn translate(
    ctx: Context<'_>,
    #[description = "Texto a enviar al modelo de IA"]
    #[rest]
    prompt: String
) -> CommandResult {
    let loading = ctx.say("Cargando...").await?;
    let message = create_ai_message(prompt).await?;
    let reply = CreateReply::default().content(message);

    loading.edit(ctx, reply).await?;

    Ok(())
}

fn create_request(prompt: String) -> Result<ChatCompletionResponse, Error> {
    let url = dotenvy::var("OPENAI_API_BASE").expect("OPENAI_API_BASE not found");
    let api_key = dotenvy::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not found");
    let model = dotenvy::var("AI_MODEL").expect("AI_MODEL not found");
    let client = Client::new_with_endpoint(url, api_key);

    let req = ChatCompletionRequest::new(
        model,
        vec![
            chat_completion::ChatCompletionMessage {
                role: chat_completion::MessageRole::system,
                content: chat_completion::Content::Text(SYSTEM_PROMPT.clone()),
                name: None,
            },
            chat_completion::ChatCompletionMessage {
                role: chat_completion::MessageRole::user,
                content: chat_completion::Content::Text(prompt),
                name: None,
            }
        ],
    ).max_tokens(1024);

    let request = client.chat_completion(req)?;

    Ok(request)
}

pub async fn create_ai_message(prompt: String) -> Result<String, Error> {
    let response = create_request(prompt)?;
    let message = response
        .choices
        .into_iter()
        .next()
        .and_then(|char| char.message.content);

    let Some(mut message) = message else {
        return Err(Error::from("No se recibió una respuesta válida del modelo de IA".to_string()));
    };

    let re = Regex::new(r"(?s)<think>.*?</think>")?;
    message = re.replace_all(&message, "").trim().to_string();

    Ok(message)
}