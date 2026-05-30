use hom_shared::RpcError;
use serde_json::{Value, json};

pub fn model_catalog(params: Value) -> Result<Value, RpcError> {
    let provider_id = string_param(&params, &["provider_id", "providerId"])
        .unwrap_or_else(|| "unknown-provider".to_string());
    let model_id = string_param(&params, &["model_id", "modelId", "model"])
        .unwrap_or_else(|| "unknown-model".to_string());
    let catalog = params
        .get("catalog")
        .or_else(|| params.get("model_catalog"));
    let catalog_verified = catalog
        .and_then(|value| {
            value
                .get("verified")
                .or_else(|| value.get("catalog_verified"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let context_window_tokens = catalog
        .and_then(|value| {
            value
                .get("context_window_tokens")
                .or_else(|| value.get("contextWindowTokens"))
                .or_else(|| value.get("max_context_tokens"))
                .or_else(|| value.get("maxContextTokens"))
        })
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(0);
    let max_output_tokens = catalog
        .and_then(|value| {
            value
                .get("max_output_tokens")
                .or_else(|| value.get("maxOutputTokens"))
        })
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(0);
    let catalog_source = catalog
        .and_then(|value| value.get("source").or_else(|| value.get("catalog_source")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("missing_or_unverified_catalog")
        .to_string();
    let fetched_at_s = catalog
        .and_then(|value| {
            value
                .get("fetched_at_s")
                .or_else(|| value.get("fetchedAtS"))
        })
        .and_then(Value::as_i64);
    let deterministic_context_window = catalog_verified && context_window_tokens > 0;

    Ok(json!({
        "ok": true,
        "catalog_kind": "provider_model_catalog_v1",
        "provider_id": provider_id,
        "model_id": model_id,
        "catalog_source": catalog_source,
        "catalog_verified": catalog_verified,
        "catalog_missing_or_unverified": !catalog_verified || context_window_tokens == 0,
        "deterministic_context_window": deterministic_context_window,
        "context_window_tokens": context_window_tokens,
        "max_output_tokens": max_output_tokens,
        "fetched_at_s": fetched_at_s,
        "hom_local_acceptance": {
            "accepted": true,
            "blocking_calibration": false,
            "acceptance_boundary": "catalog_metadata_intake_without_compaction_gate"
        },
        "compaction_decision": {
            "made_here": false,
            "reason": "provider_catalog_surface_only"
        },
        "brain_calibration": {
            "deferred": true,
            "calibration_scope": "nightly_and_reasoning",
            "calibration_boundary": "post_catalog_intake_brain_layer"
        }
    }))
}

pub fn context_pressure(params: Value) -> Result<Value, RpcError> {
    let provider_id = string_param(&params, &["provider_id", "providerId"])
        .unwrap_or_else(|| "unknown-provider".to_string());
    let model_id = string_param(&params, &["model_id", "modelId", "model"])
        .unwrap_or_else(|| "unknown-model".to_string());
    let catalog = params
        .get("model_catalog")
        .or_else(|| params.get("catalog"));
    let catalog_verified = bool_from_catalog(catalog, &["verified", "catalog_verified"]);
    let context_window_tokens = i64_from_catalog(
        catalog,
        &[
            "context_window_tokens",
            "contextWindowTokens",
            "max_context_tokens",
            "maxContextTokens",
        ],
    )
    .unwrap_or(0);
    let max_output_tokens =
        i64_from_catalog(catalog, &["max_output_tokens", "maxOutputTokens"]).unwrap_or(0);
    let input_tokens_estimated = i64_param(
        &params,
        &[
            "input_tokens_estimated",
            "inputTokensEstimated",
            "input_tokens",
        ],
    )
    .unwrap_or(0)
    .max(0);
    let reserved_output_tokens = i64_param(
        &params,
        &[
            "reserved_output_tokens",
            "reservedOutputTokens",
            "output_tokens_reserved",
        ],
    )
    .unwrap_or(0)
    .max(0);
    let deterministic_context_window = catalog_verified && context_window_tokens > 0;
    let available_input_tokens = if deterministic_context_window {
        (context_window_tokens - reserved_output_tokens).max(0)
    } else {
        0
    };
    let tokens_used_for_pressure = input_tokens_estimated + reserved_output_tokens;
    let ratio_bps = if deterministic_context_window {
        Some(((tokens_used_for_pressure * 10_000) / context_window_tokens).max(0))
    } else {
        None
    };
    let band = ratio_bps.map(pressure_band).unwrap_or("unknown");

    Ok(json!({
        "ok": true,
        "diagnostic_kind": "context_pressure_diagnostics_v1",
        "diagnostics_only": true,
        "provider": {
            "provider_id": provider_id,
            "model_id": model_id,
            "catalog_verified": catalog_verified,
            "deterministic_context_window": deterministic_context_window,
            "context_window_tokens": context_window_tokens,
            "max_output_tokens": max_output_tokens
        },
        "token_budget": {
            "context_window_tokens": context_window_tokens,
            "input_tokens_estimated": input_tokens_estimated,
            "reserved_output_tokens": reserved_output_tokens,
            "available_input_tokens": available_input_tokens,
            "max_output_tokens": max_output_tokens
        },
        "pressure": {
            "ratio_basis": "input_plus_reserved_output_over_context_window",
            "tokens_used_for_pressure": tokens_used_for_pressure,
            "ratio_bps": ratio_bps,
            "band": band,
            "deterministic": deterministic_context_window
        },
        "compaction_decision": {
            "made_here": false,
            "triggered": false,
            "reason": "diagnostics_only_pressure_surface"
        },
        "hom_local_acceptance": {
            "accepted": true,
            "blocking_calibration": false,
            "acceptance_boundary": "context_pressure_diagnostics_without_compaction_gate"
        },
        "brain_calibration": {
            "deferred": true,
            "calibration_scope": "nightly_and_reasoning",
            "calibration_boundary": "post_diagnostic_brain_layer"
        }
    }))
}

fn pressure_band(ratio_bps: i64) -> &'static str {
    match ratio_bps {
        value if value >= 9000 => "critical",
        value if value >= 7500 => "high",
        value if value >= 5000 => "medium",
        _ => "low",
    }
}

fn i64_param(params: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .filter_map(|key| params.get(*key).and_then(Value::as_i64))
        .find(|value| *value >= 0)
}

fn bool_from_catalog(catalog: Option<&Value>, keys: &[&str]) -> bool {
    catalog
        .and_then(|value| {
            keys.iter()
                .filter_map(|key| value.get(*key).and_then(Value::as_bool))
                .next()
        })
        .unwrap_or(false)
}

fn i64_from_catalog(catalog: Option<&Value>, keys: &[&str]) -> Option<i64> {
    catalog.and_then(|value| {
        keys.iter()
            .filter_map(|key| value.get(*key).and_then(Value::as_i64))
            .find(|item| *item > 0)
    })
}

fn string_param(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| params.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}
