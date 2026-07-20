fn push_metadata(
    output: &mut Vec<TelemetryEvent>,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    model_override: Option<&str>,
) {
    let cwd = first_string_at(
        value,
        &[
            &["cwd"],
            &["workspaceRoot"],
            &["workspace_root"],
            &["params", "cwd"],
        ],
    )
    .map(str::to_owned);
    let model = model_override
        .or_else(|| {
            first_string_at(
                value,
                &[
                    &["modelId"],
                    &["model_id"],
                    &["model"],
                    &["result", "modelId"],
                    &["result", "model"],
                ],
            )
        })
        .map(str::to_owned);
    output.push(event(
        ToolId::GrokBuild,
        session,
        None,
        value,
        context,
        Confidence::Exact,
        EventKind::Metadata {
            cwd,
            model,
            provider: Some("xai".to_owned()),
            pid: None,
        },
    ));
}

fn push_usage(
    output: &mut Vec<TelemetryEvent>,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &ParserState,
) {
    let usage = grok_usage(value);
    if usage.is_empty() {
        return;
    }
    let turn = state.current_turn(session).or_else(|| turn_id(value));
    output.push(event(
        ToolId::GrokBuild,
        session,
        turn.as_deref(),
        value,
        context,
        Confidence::Exact,
        usage.into_event(false),
    ));
}

fn grok_usage(value: &Value) -> UsageFields {
    let mut usage = UsageFields {
        input_tokens: first_u64_at(
            value,
            &[
                &["usage", "input_tokens"],
                &["usage", "inputTokens"],
                &["result", "usage", "inputTokens"],
                &["result", "_meta", "usage", "inputTokens"],
                &["params", "update", "usage", "inputTokens"],
            ],
        ),
        output_tokens: first_u64_at(
            value,
            &[
                &["usage", "output_tokens"],
                &["usage", "outputTokens"],
                &["result", "usage", "outputTokens"],
                &["result", "_meta", "usage", "outputTokens"],
                &["params", "update", "usage", "outputTokens"],
            ],
        ),
        cached_input_tokens: first_u64_at(
            value,
            &[
                &["usage", "cache_read_input_tokens"],
                &["usage", "cacheReadInputTokens"],
                &["usage", "cachedReadTokens"],
                &["result", "usage", "cachedReadTokens"],
                &["result", "_meta", "usage", "cachedReadTokens"],
                &["params", "update", "usage", "cachedReadTokens"],
            ],
        ),
        reasoning_tokens: first_u64_at(
            value,
            &[
                &["usage", "reasoning_tokens"],
                &["usage", "reasoningTokens"],
                &["result", "usage", "reasoningTokens"],
                &["result", "_meta", "usage", "reasoningTokens"],
                &["params", "update", "usage", "reasoningTokens"],
            ],
        ),
        context_window: first_u64_at(
            value,
            &[
                &["usage", "contextWindow"],
                &["result", "_meta", "usage", "contextWindow"],
                &["params", "update", "usage", "contextWindow"],
            ],
        ),
    };

    if let Some(model_usage) = value.get("modelUsage").and_then(Value::as_object) {
        let mut per_model = UsageFields::default();
        for model in model_usage.values() {
            per_model.input_tokens = sum_optional(
                per_model.input_tokens,
                first_u64_at(model, &[&["inputTokens"], &["input_tokens"]]),
            );
            per_model.output_tokens = sum_optional(
                per_model.output_tokens,
                first_u64_at(model, &[&["outputTokens"], &["output_tokens"]]),
            );
            per_model.cached_input_tokens = sum_optional(
                per_model.cached_input_tokens,
                first_u64_at(
                    model,
                    &[
                        &["cacheReadInputTokens"],
                        &["cachedReadTokens"],
                        &["cached_input_tokens"],
                    ],
                ),
            );
            per_model.reasoning_tokens = sum_optional(
                per_model.reasoning_tokens,
                first_u64_at(model, &[&["reasoningTokens"], &["reasoning_tokens"]]),
            );
        }
        usage.input_tokens = usage.input_tokens.or(per_model.input_tokens);
        usage.output_tokens = usage.output_tokens.or(per_model.output_tokens);
        usage.cached_input_tokens = usage.cached_input_tokens.or(per_model.cached_input_tokens);
        usage.reasoning_tokens = usage.reasoning_tokens.or(per_model.reasoning_tokens);
    }
    usage
}

fn sum_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn model_from_usage(value: &Value) -> Option<&str> {
    value
        .get("modelUsage")
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next())
        .map(String::as_str)
        .or_else(|| {
            first_string_at(
                value,
                &[
                    &["modelId"],
                    &["model_id"],
                    &["model"],
                    &["result", "modelId"],
                    &["params", "update", "model_id"],
                ],
            )
        })
}
