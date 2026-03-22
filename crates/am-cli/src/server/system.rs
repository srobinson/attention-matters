use serde::Deserialize;
use serde_json::Value;

use am_core::{
    serde_compat::{export_json, import_json},
    store_trait::AmStore,
};

use super::AmServer;
use crate::jsonrpc::tool_result_text;

#[derive(Debug, Deserialize)]
pub(super) struct ImportRequest {
    /// Full state JSON to import
    state: serde_json::Value,
}

impl<S: AmStore> AmServer<S> {
    pub(super) fn am_stats(&self) -> Result<Value, String> {
        let state = self.state.lock().expect("poisoned mutex");
        let mut stats = Self::stats_json(&state.system);

        // Add store-level stats (DB size, activation distribution)
        let db_size = state.store.db_size();
        stats["db_size_bytes"] = serde_json::json!(db_size);
        if let Ok(activation) = state.store.activation_distribution() {
            stats["activation"] = serde_json::json!({
                "mean": activation.mean_activation,
                "max": activation.max_activation,
                "zero_count": activation.zero_activation,
            });
        }

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&stats).unwrap_or_default(),
        ))
    }

    pub(super) fn am_export(&self) -> Result<Value, String> {
        let state = self.state.lock().expect("poisoned mutex");
        let json = export_json(&state.system).map_err(|e| format!("[serde] {e}"))?;
        Ok(tool_result_text(&json))
    }

    pub(super) fn am_import(&self, args: &Value) -> Result<Value, String> {
        let req: ImportRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let json_str = serde_json::to_string(&req.state).map_err(|e| format!("[serde] {e}"))?;

        let imported = import_json(&json_str).map_err(|e| format!("[serde] {e}"))?;

        state.system = imported;

        // Intentional save_system: import replaces the entire DAE state,
        // so a full rewrite is the only correct persistence strategy.
        if let Err(e) = state.store.save_system(&state.system) {
            tracing::error!("failed to persist after import: {e}");
        }

        let result = serde_json::json!({
            "imported": true,
            "stats": Self::stats_json(&state.system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }
}
