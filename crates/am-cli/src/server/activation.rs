use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use am_core::{
    feedback::{FeedbackSignal, apply_feedback},
    query::{QueryEngine, QueryManifest},
    salient::{extract_salient, mark_salient_typed},
    store_trait::AmStore,
};

use super::{AmServer, ServerState, check_input_size, persist_manifest};
use crate::jsonrpc::tool_result_text;

#[derive(Debug, Deserialize)]
pub(super) struct ActivateResponseRequest {
    /// Response text to strengthen connections for
    text: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SalientRequest {
    /// Text to mark as conscious memory (may contain salient tags)
    text: String,
    /// Optional list of neighborhood UUIDs that this new memory supersedes.
    #[serde(default)]
    supersedes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FeedbackRequest {
    /// The original query text that produced the recall
    query: String,
    /// UUIDs of the neighborhoods that were recalled and shown to the user
    neighborhood_ids: Vec<String>,
    /// Feedback signal: "boost" if the recall was helpful, "demote" if not
    signal: String,
}

impl<S: AmStore> AmServer<S> {
    pub(super) fn am_activate_response(&self, args: &Value) -> Result<Value, String> {
        let req: ActivateResponseRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState { system, store, .. } = &mut *state;

        let (activation, activated_ids) = QueryEngine::activate(system, &req.text);
        let all_refs: Vec<_> = activation
            .subconscious
            .iter()
            .chain(activation.conscious.iter())
            .copied()
            .collect();
        let mut drifted = QueryEngine::drift_and_consolidate(system, &all_refs);
        drifted.extend(QueryEngine::couple_phases(
            system,
            &activation.subconscious,
            &activation.conscious,
        ));

        let manifest = QueryManifest {
            drifted,
            activated: activated_ids,
            demoted_activations: Vec::new(),
        };
        persist_manifest(store, system, &manifest, "activate_response");

        let result = serde_json::json!({
            "activated": all_refs.len(),
            "stats": Self::stats_json(system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    pub(super) fn am_salient(&self, args: &Value) -> Result<Value, String> {
        let req: SalientRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        // Track how many neighborhoods exist before adding new ones
        let nbhd_before = system.conscious_episode.neighborhoods.len();

        let stored = extract_salient(system, &req.text, rng);
        let new_id = if stored == 0 {
            // No <salient> tags found - mark the whole text as salient
            // with automatic type detection from DECISION:/PREFERENCE: prefix
            let id = mark_salient_typed(system, &req.text, rng);
            Some(id)
        } else {
            None
        };

        // Persist only the newly added neighborhoods
        for nbhd in &system.conscious_episode.neighborhoods[nbhd_before..] {
            if let Err(e) = store.save_neighborhood(&system.conscious_episode, nbhd) {
                tracing::error!("failed to persist conscious neighborhood: {e}");
            }
        }
        let stored = if stored == 0 { 1u32 } else { stored };

        // Process supersedes: mark old neighborhoods as superseded by the new one
        let mut superseded_count = 0u32;
        if let Some(new_id) = new_id {
            for old_id_str in &req.supersedes {
                if let Ok(old_id) = Uuid::parse_str(old_id_str) {
                    // Update in-memory
                    if system.mark_superseded(old_id, new_id) {
                        // Persist targeted update to SQLite
                        if let Err(e) = store.mark_superseded(old_id, new_id) {
                            tracing::error!("failed to persist supersession: {e}");
                        }
                        superseded_count += 1;
                    } else {
                        tracing::warn!("supersedes target not found: {old_id_str}");
                    }
                } else {
                    tracing::warn!("invalid UUID in supersedes: {old_id_str}");
                }
            }
        } else if !req.supersedes.is_empty() {
            tracing::warn!(
                "supersedes ignored: multiple salient tags produce multiple neighborhoods, \
                 supersession only applies to single-neighborhood salient calls"
            );
        }

        let mut result = serde_json::json!({
            "stored": stored,
            "stats": Self::stats_json(system),
        });
        if superseded_count > 0 {
            result["superseded"] = serde_json::json!(superseded_count);
        }

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    pub(super) fn am_feedback(&self, args: &Value) -> Result<Value, String> {
        let req: FeedbackRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.query, "query")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState { system, store, .. } = &mut *state;

        let signal = match req.signal.to_lowercase().as_str() {
            "boost" => FeedbackSignal::Boost,
            "demote" => FeedbackSignal::Demote,
            other => {
                return Err(format!("signal must be 'boost' or 'demote', got '{other}'"));
            }
        };

        let neighborhood_ids: Vec<Uuid> = req
            .neighborhood_ids
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();

        if neighborhood_ids.is_empty() {
            return Err("no valid neighborhood UUIDs provided".to_owned());
        }

        let feedback = apply_feedback(system, &req.query, &neighborhood_ids, signal);

        persist_manifest(store, system, &feedback.manifest, "feedback");

        let result = serde_json::json!({
            "boosted": feedback.boosted,
            "demoted": feedback.demoted,
            "centroid": feedback.centroid.map(|c| serde_json::json!({
                "w": c.w, "x": c.x, "y": c.y, "z": c.z
            })),
            "stats": Self::stats_json(system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }
}
