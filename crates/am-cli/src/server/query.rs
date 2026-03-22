use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use am_core::{
    compose::{
        BudgetConfig, RecallCategory, compose_context, compose_context_budgeted, compose_index,
        retrieve_by_ids,
    },
    query::QueryEngine,
    store_trait::AmStore,
    surface::compute_surface,
};

use super::{AmServer, ServerState, check_input_size, flush_orphaned_buffer, persist_manifest};
use crate::jsonrpc::tool_result_text;

#[derive(Debug, Deserialize)]
pub(super) struct QueryRequest {
    /// The text to query the memory system with
    text: String,
    /// Optional maximum token budget for composed context.
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QueryIndexRequest {
    /// The query text to search memory for
    text: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RetrieveByIdsRequest {
    /// Neighborhood UUIDs to retrieve full content for
    ids: Vec<String>,
}

impl<S: AmStore> AmServer<S> {
    pub(super) fn am_query(&self, args: &Value) -> Result<Value, String> {
        let req: QueryRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system,
            store,
            rng,
            session_recalled,
            ..
        } = &mut *state;

        flush_orphaned_buffer(store, system, rng);

        let query_result = QueryEngine::process_query(system, &req.text);
        let surface = compute_surface(system, &query_result);

        let (mut result, new_ids) = if let Some(max_tokens) = req.max_tokens {
            // Budgeted query: Nancy's prompt compiler uses this
            let budget = BudgetConfig {
                max_tokens,
                min_conscious: 1,
                min_subconscious: 1,
                min_novel: 0,
            };
            let composed = compose_context_budgeted(
                system,
                &surface,
                &query_result,
                &budget,
                Some(session_recalled),
            );
            let ids: Vec<Uuid> = composed
                .included
                .iter()
                .map(|f| f.neighborhood_id)
                .collect();
            // Categorize IDs from IncludedFragment for feedback tracking
            let mut con_ids = Vec::new();
            let mut sub_ids = Vec::new();
            let mut nov_ids = Vec::new();
            for f in &composed.included {
                match f.category {
                    RecallCategory::Conscious => con_ids.push(f.neighborhood_id.to_string()),
                    RecallCategory::Subconscious => sub_ids.push(f.neighborhood_id.to_string()),
                    RecallCategory::Novel => nov_ids.push(f.neighborhood_id.to_string()),
                }
            }
            let json = serde_json::json!({
                "context": composed.context,
                "metrics": {
                    "conscious": composed.metrics.conscious,
                    "subconscious": composed.metrics.subconscious,
                    "novel": composed.metrics.novel,
                },
                "recalled_ids": {
                    "conscious": con_ids,
                    "subconscious": sub_ids,
                    "novel": nov_ids,
                },
                "token_estimate": {
                    "conscious": composed.token_estimate.conscious,
                    "subconscious": composed.token_estimate.subconscious,
                    "novel": composed.token_estimate.novel,
                    "total": composed.token_estimate.total,
                },
                "budget": {
                    "tokens_used": composed.tokens_used,
                    "tokens_budget": composed.tokens_budget,
                    "included_count": composed.included.len(),
                    "excluded_count": composed.excluded_count,
                },
                "stats": Self::stats_json(system),
            });
            (json, ids)
        } else {
            // Default: fixed-size composition
            let composed = compose_context(system, &surface, &query_result, Some(session_recalled));
            let ids = composed.included_ids.clone();
            let recalled = &composed.recalled_ids;
            let json = serde_json::json!({
                "context": composed.context,
                "metrics": {
                    "conscious": composed.metrics.conscious,
                    "subconscious": composed.metrics.subconscious,
                    "novel": composed.metrics.novel,
                },
                "recalled_ids": {
                    "conscious": recalled.conscious.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                    "subconscious": recalled.subconscious.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                    "novel": recalled.novel.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                },
                "token_estimate": {
                    "conscious": composed.token_estimate.conscious,
                    "subconscious": composed.token_estimate.subconscious,
                    "novel": composed.token_estimate.novel,
                    "total": composed.token_estimate.total,
                },
                "stats": Self::stats_json(system),
            });
            (json, ids)
        };

        // Compose compact index summary (top 10 entries, most recent first)
        let index = compose_index(system, &surface, &query_result, Some(session_recalled));
        let mut sorted_entries = index.entries;
        sorted_entries.sort_by(|a, b| b.epoch.cmp(&a.epoch));
        let index_entries: Vec<serde_json::Value> = sorted_entries
            .iter()
            .take(10)
            .map(|e| {
                serde_json::json!({
                    "id": e.neighborhood_id.to_string(),
                    "category": format!("{:?}", e.category),
                    "type": format!("{:?}", e.neighborhood_type),
                    "score": (e.score * 100.0).round() / 100.0,
                    "epoch": e.epoch,
                    "summary": e.summary,
                    "token_estimate": e.token_estimate,
                })
            })
            .collect();
        result["index"] = serde_json::json!(index_entries);

        persist_manifest(store, system, &query_result.manifest, "query");

        // Increment recall count for returned neighborhood IDs (diminishing returns)
        for id in new_ids {
            *session_recalled.entry(id).or_insert(0) += 1;
        }

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    pub(super) fn am_query_index(&self, args: &Value) -> Result<Value, String> {
        let req: QueryIndexRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system,
            store,
            rng,
            session_recalled,
            ..
        } = &mut *state;

        flush_orphaned_buffer(store, system, rng);

        let query_result = QueryEngine::process_query(system, &req.text);
        let surface = compute_surface(system, &query_result);

        let index = compose_index(system, &surface, &query_result, Some(session_recalled));

        persist_manifest(store, system, &query_result.manifest, "query_index");

        let entries_json: Vec<serde_json::Value> = index
            .entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.neighborhood_id.to_string(),
                    "category": format!("{:?}", e.category),
                    "type": format!("{:?}", e.neighborhood_type),
                    "score": (e.score * 100.0).round() / 100.0,
                    "epoch": e.epoch,
                    "summary": e.summary,
                    "token_estimate": e.token_estimate,
                })
            })
            .collect();

        let result = serde_json::json!({
            "entries": entries_json,
            "total_candidates": index.total_candidates(),
            "total_tokens_if_fetched": index.total_tokens_if_fetched(),
            "stats": Self::stats_json(system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    pub(super) fn am_retrieve(&self, args: &Value) -> Result<Value, String> {
        let req: RetrieveByIdsRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState { system, .. } = &mut *state;

        let ids: Vec<Uuid> = req
            .ids
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();

        let fragments = retrieve_by_ids(system, &ids);

        // Track these as recalled for diminishing returns
        for f in &fragments {
            *state.session_recalled.entry(f.neighborhood_id).or_insert(0) += 1;
        }

        let entries_json: Vec<serde_json::Value> = fragments
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.neighborhood_id.to_string(),
                    "category": format!("{:?}", f.category),
                    "type": format!("{:?}", f.neighborhood_type),
                    "episode": f.episode_name,
                    "tokens": f.tokens,
                    "text": f.text,
                })
            })
            .collect();

        let result = serde_json::json!({
            "entries": entries_json,
            "count": fragments.len(),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }
}
