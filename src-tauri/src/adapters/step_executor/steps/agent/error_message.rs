pub(crate) async fn format_agent_error_message(
    message: &str,
    machine_id: &str,
    exec: &dyn crate::ports::execution::ExecutionPort,
) -> String {
    if message.contains("OpenCode service failure")
        || message.contains("timed out")
        || message.contains("no output")
        || message.is_empty()
    {
        // Fetch last 100 lines of remote log
        if let Ok(logs) = exec
            .run_command(machine_id, "tail -n 100 /tmp/opencode_run.log")
            .await
        {
            if logs.contains("FreeUsageLimitError") || logs.contains("Rate limit exceeded") {
                return "OpenCode Rate Limit Exceeded: The free model rate limit was reached. Please try changing the model to a different model (e.g. 'opencode/big-pickle') or try again later.".to_string();
            }
            if logs.contains("CreditLimitError")
                || logs.contains("Insufficient funds")
                || logs.contains("credits limit")
                || logs.contains("insufficient balance")
            {
                return "OpenCode Credit Limit Exceeded: You have run out of credits or reached your usage quota. Please verify your billing/credits on OpenCode or switch to a free model.".to_string();
            }
            // Fallback search through last lines
            for line in logs.lines().rev() {
                if line.contains("FreeUsageLimitError") || line.contains("Rate limit exceeded") {
                    return "OpenCode Rate Limit Exceeded: The free model rate limit was reached. Please try changing the model to a different model (e.g. 'opencode/big-pickle') or try again later.".to_string();
                }
                if line.contains("CreditLimitError") || line.contains("credits limit") {
                    return "OpenCode Credit Limit Exceeded: You have run out of credits or reached your usage quota. Please verify your billing/credits on OpenCode or switch to a free model.".to_string();
                }
            }
        }
    }
    message.to_string()
}
