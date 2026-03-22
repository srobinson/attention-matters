use serde::Deserialize;
use serde_json::Value;

use am_core::{
    batch::{BatchQueryEngine, BatchQueryRequest},
    compose::RecallCategory,
    store_trait::AmStore,
    tokenizer::ingest_text,
};

use super::{
    AmServer, BUFFER_THRESHOLD, MAX_TOOL_INPUT_BYTES, ServerState, check_input_size,
    flush_orphaned_buffer, persist_manifest, store_err_to_string,
};
use crate::jsonrpc::tool_result_text;

#[derive(Debug, Deserialize)]
pub(super) struct BufferRequest {
    /// User's message text
    user: String,
    /// Assistant's response text
    assistant: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct IngestRequest {
    /// Document text to ingest
    text: String,
    /// Optional name for the episode
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BatchQueryItem {
    /// The query text
    query: String,
    /// Optional token budget for this query's context
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct McpBatchQueryRequest {
    /// List of queries to process in a single batch.
    queries: Vec<BatchQueryItem>,
}

impl<S: AmStore> AmServer<S> {
    pub(super) fn am_buffer(&self, args: &Value) -> Result<Value, String> {
        let req: BufferRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let total_len = req.user.len() + req.assistant.len();
        if total_len > MAX_TOOL_INPUT_BYTES {
            return Err(format!(
                "combined input exceeds {} byte limit",
                MAX_TOOL_INPUT_BYTES
            ));
        }

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system,
            store,
            rng,
            dedup_window,
            ..
        } = &mut *state;

        // Dedup check: hash the exchange and check against recent hashes
        let hash = Self::content_hash(&req.user, &req.assistant);
        Self::clean_dedup_window(dedup_window);

        if dedup_window.contains_key(&hash) {
            let result = serde_json::json!({
                "deduplicated": true,
                "buffer_size": store.buffer_count().unwrap_or(0),
            });
            return Ok(tool_result_text(
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            ));
        }
        dedup_window.insert(hash, std::time::Instant::now());

        let buffer_size = store
            .append_buffer(&req.user, &req.assistant)
            .map_err(store_err_to_string)?;

        let mut episode_created: Option<String> = None;

        if buffer_size >= BUFFER_THRESHOLD {
            let exchanges = store.drain_buffer().map_err(store_err_to_string)?;

            let combined: String = exchanges
                .iter()
                .map(|(u, a)| format!("{u}\n{a}"))
                .collect::<Vec<_>>()
                .join("\n\n");

            let episode = ingest_text(&combined, Some("conversation"), rng);
            let name = episode.name.clone();
            system.add_episode(episode);

            if let Err(e) = store.save_episode(system.episodes.last().unwrap()) {
                tracing::error!("failed to persist after buffer episode: {e}");
            }

            episode_created = Some(name);
        }

        let result = serde_json::json!({
            "buffer_size": buffer_size,
            "episode_created": episode_created,
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    pub(super) fn am_ingest(&self, args: &Value) -> Result<Value, String> {
        let req: IngestRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        let episode = ingest_text(&req.text, req.name.as_deref(), rng);
        let ep_name = episode.name.clone();
        let neighborhoods = episode.neighborhoods.len();
        let occurrences: usize = episode
            .neighborhoods
            .iter()
            .map(|n| n.occurrences.len())
            .sum();

        system.add_episode(episode);

        if let Err(e) = store.save_episode(system.episodes.last().unwrap()) {
            tracing::error!("failed to persist after ingest: {e}");
        }

        let result = serde_json::json!({
            "episode": ep_name,
            "neighborhoods": neighborhoods,
            "occurrences": occurrences,
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    pub(super) fn am_batch_query(&self, args: &Value) -> Result<Value, String> {
        let req: McpBatchQueryRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let total_len: usize = req.queries.iter().map(|q| q.query.len()).sum();
        if total_len > MAX_TOOL_INPUT_BYTES {
            return Err(format!(
                "aggregate query text ({total_len} bytes) exceeds {} byte limit",
                MAX_TOOL_INPUT_BYTES
            ));
        }

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        flush_orphaned_buffer(store, system, rng);

        let requests: Vec<BatchQueryRequest> = req
            .queries
            .iter()
            .map(|q| BatchQueryRequest {
                query: q.query.clone(),
                max_tokens: q.max_tokens,
            })
            .collect();

        let batch_output = BatchQueryEngine::batch_query(system, &requests);

        persist_manifest(store, system, &batch_output.manifest, "batch_query");

        let results_json: Vec<serde_json::Value> = batch_output
            .results
            .iter()
            .map(|r| {
                let mut con_ids = Vec::new();
                let mut sub_ids = Vec::new();
                let mut nov_ids = Vec::new();
                for f in &r.context.included {
                    match f.category {
                        RecallCategory::Conscious => con_ids.push(f.neighborhood_id.to_string()),
                        RecallCategory::Subconscious => sub_ids.push(f.neighborhood_id.to_string()),
                        RecallCategory::Novel => nov_ids.push(f.neighborhood_id.to_string()),
                    }
                }

                serde_json::json!({
                    "query": r.query,
                    "context": r.context.context,
                    "metrics": {
                        "conscious": r.context.metrics.conscious,
                        "subconscious": r.context.metrics.subconscious,
                        "novel": r.context.metrics.novel,
                    },
                    "recalled_ids": {
                        "conscious": con_ids,
                        "subconscious": sub_ids,
                        "novel": nov_ids,
                    },
                    "token_estimate": {
                        "conscious": r.context.token_estimate.conscious,
                        "subconscious": r.context.token_estimate.subconscious,
                        "novel": r.context.token_estimate.novel,
                        "total": r.context.token_estimate.total,
                    },
                    "budget": {
                        "tokens_used": r.context.tokens_used,
                        "tokens_budget": r.context.tokens_budget,
                        "included_count": r.context.included.len(),
                        "excluded_count": r.context.excluded_count,
                    },
                    "activated_count": r.activated_count,
                })
            })
            .collect();

        let result = serde_json::json!({
            "results": results_json,
            "batch_size": results_json.len(),
            "stats": Self::stats_json(system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }
}
