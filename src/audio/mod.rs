mod voicevox;

pub use voicevox::VoiceVoxClient;

use lavalink_rs::hook;
use lavalink_rs::model::events;
use lavalink_rs::prelude::LavalinkClient;

#[hook]
pub async fn ready_event(client: LavalinkClient, session_id: String, event: &events::Ready) {
    client.delete_all_player_contexts().await.unwrap();
    println!("{session_id:?} -> {event:?}");
}