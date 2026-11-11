pub struct StepRow {
    pub turn: u64,
    pub step: u64,
    pub account: String,
    pub model: String,
    pub ttfb_ms: u64,
    pub total_ms: u64,
    pub usage: crate::types::Usage,
    pub tools: Vec<(String, u64)>,
    pub retries: u64,
}

pub fn step_rows(_events: &[crate::trace::TraceEvent]) -> Vec<StepRow> {
    // Implementation to aggregate events into StepRows...
    Vec::new()
}

pub fn format_table(_rows: &[StepRow]) -> String {
    // Implementation to format rows into a table...
    String::new()
}
