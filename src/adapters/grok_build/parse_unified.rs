impl GrokBuildAdapter {
    fn parse_unified(
        &mut self,
        value: &Value,
        context: &AdapterContext,
    ) -> Vec<TelemetryEvent> {
        let Some(pid) = first_u64_at(value, &[&["pid"]])
            .and_then(|pid| u32::try_from(pid).ok())
        else {
            return Vec::new();
        };
        let Some(source_session) = string_at(value, &["sid"]) else {
            return Vec::new();
        };

        match string_at(value, &["msg"]).unwrap_or_default() {
            "session created" => {
                self.unified_roots.insert(pid, source_session.to_owned());
                vec![
                    event(
                        self.tool(),
                        source_session,
                        None,
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::SessionStarted,
                    ),
                    event(
                        self.tool(),
                        source_session,
                        None,
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::Metadata {
                            cwd: string_at(value, &["ctx", "cwd"]).map(str::to_owned),
                            model: None,
                            provider: Some("xai".to_owned()),
                            pid: None,
                        },
                    ),
                ]
            }
            "shell.turn.inference_done" => {
                let session = self
                    .unified_roots
                    .get(&pid)
                    .map(String::as_str)
                    .unwrap_or(source_session)
                    .to_owned();
                let Some(output_tokens) =
                    first_u64_at(value, &[&["ctx", "completion_tokens"]])
                else {
                    return Vec::new();
                };
                let tokens_per_second =
                    first_u64_at(value, &[&["ctx", "model_elapsed_ms"]])
                        .and_then(|elapsed| {
                            elapsed.checked_sub(
                                first_u64_at(value, &[&["ctx", "ttft_ms"]])
                                    .unwrap_or_default(),
                            )
                        })
                        .filter(|duration| *duration > 0 && output_tokens > 0)
                        .map(|duration| output_tokens as f64 * 1_000.0 / duration as f64);
                let delta = UsageFields {
                    input_tokens: first_u64_at(value, &[&["ctx", "prompt_tokens"]]),
                    output_tokens: Some(output_tokens),
                    cached_input_tokens: first_u64_at(
                        value,
                        &[&["ctx", "cached_prompt_tokens"]],
                    ),
                    reasoning_tokens: first_u64_at(value, &[&["ctx", "reasoning_tokens"]]),
                    context_window: None,
                };
                let cumulative = {
                    let total = self.unified_usage.entry(session.clone()).or_default();
                    add_usage(total, delta);
                    (*total).into_event(true)
                };
                let mut output = Vec::with_capacity(2);
                if let Some(tokens_per_second) = tokens_per_second {
                    output.push(event(
                        self.tool(),
                        &session,
                        None,
                        value,
                        context,
                        Confidence::Derived,
                        EventKind::RateReported {
                            output_tokens,
                            tokens_per_second,
                        },
                    ));
                }
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Exact,
                    cumulative,
                ));
                output
            }
            _ => Vec::new(),
        }
    }
}

fn add_usage(total: &mut UsageFields, delta: UsageFields) {
    total.input_tokens = Some(
        total
            .input_tokens
            .unwrap_or_default()
            .saturating_add(delta.input_tokens.unwrap_or_default()),
    );
    total.output_tokens = Some(
        total
            .output_tokens
            .unwrap_or_default()
            .saturating_add(delta.output_tokens.unwrap_or_default()),
    );
    total.cached_input_tokens = Some(
        total
            .cached_input_tokens
            .unwrap_or_default()
            .saturating_add(delta.cached_input_tokens.unwrap_or_default()),
    );
    total.reasoning_tokens = Some(
        total
            .reasoning_tokens
            .unwrap_or_default()
            .saturating_add(delta.reasoning_tokens.unwrap_or_default()),
    );
}
