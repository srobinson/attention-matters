use serde_json::Value;
use uuid::Uuid;

use am_core::store_trait::AmStore;

use super::AmServer;
use crate::jsonrpc::tool_result_text;

impl<S: AmStore> AmServer<S> {
    pub(super) fn am_episodes(&self) -> Result<Value, String> {
        let state = self.state.lock().expect("poisoned mutex");

        let episodes: Vec<Value> = state
            .system
            .episodes
            .iter()
            .map(|ep| {
                let total_occurrences: usize =
                    ep.neighborhoods.iter().map(|n| n.occurrences.len()).sum();

                serde_json::json!({
                    "id": ep.id.to_string(),
                    "name": ep.name,
                    "created": ep.timestamp,
                    "neighborhood_count": ep.neighborhoods.len(),
                    "total_occurrences": total_occurrences,
                    "is_conscious": ep.is_conscious,
                })
            })
            .collect();

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&episodes).unwrap_or_default(),
        ))
    }

    pub(super) fn am_episode_neighborhoods(&self, args: &Value) -> Result<Value, String> {
        let episode_id = args
            .get("episode_id")
            .and_then(|v| v.as_str())
            .ok_or("missing episode_id")?;

        let target_id = Uuid::parse_str(episode_id).map_err(|e| format!("invalid UUID: {e}"))?;

        let state = self.state.lock().expect("poisoned mutex");

        let episode = state
            .system
            .episodes
            .iter()
            .find(|ep| ep.id == target_id)
            .ok_or_else(|| format!("episode {episode_id} not found"))?;

        let neighborhoods: Vec<Value> = episode
            .neighborhoods
            .iter()
            .map(|nbhd| {
                let text = &nbhd.source_text;
                let tokens = nbhd.occurrences.len();

                serde_json::json!({
                    "id": nbhd.id.to_string(),
                    "type": format!("{:?}", nbhd.neighborhood_type),
                    "epoch": nbhd.epoch,
                    "tokens": tokens,
                    "text": text,
                    "episode": episode.name,
                    "is_conscious": episode.is_conscious,
                    "superseded_by": nbhd.superseded_by.map(|id| id.to_string()),
                })
            })
            .collect();

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&neighborhoods).unwrap_or_default(),
        ))
    }
}
