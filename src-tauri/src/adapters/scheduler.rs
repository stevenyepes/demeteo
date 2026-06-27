use crate::ports::db::WorkflowRepository;
use crate::ports::step_executor::StepExecutor;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTimeDecomposed {
    pub year: i32,
    pub month: i32,        // 1-12
    pub day_of_month: i32, // 1-31
    pub hour: i32,         // 0-23
    pub minute: i32,       // 0-59
    pub day_of_week: i32,  // 0-6 (0 = Sunday, 1 = Monday, ...)
}

fn days_to_date(mut days: i64) -> (i32, i32, i32) {
    // Jan 1, 1970 was a Thursday
    let mut year = 1970;
    loop {
        let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if is_leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_lengths = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for &length in &month_lengths {
        if days < length {
            break;
        }
        days -= length;
        month += 1;
    }

    (year, month, (days + 1) as i32)
}

pub fn decompose_timestamp(timestamp_secs: i64) -> DateTimeDecomposed {
    let days_since_epoch = timestamp_secs / 86400;
    let day_of_week = (days_since_epoch + 4) % 7; // Thursday = 4

    let (year, month, day_of_month) = days_to_date(days_since_epoch);
    let seconds_in_day = timestamp_secs % 86400;
    let hour = seconds_in_day / 3600;
    let minute = (seconds_in_day % 3600) / 60;

    DateTimeDecomposed {
        year,
        minute: minute as i32,
        hour: hour as i32,
        day_of_month,
        month,
        day_of_week: day_of_week as i32,
    }
}

fn match_cron_field(field: &str, val: i32) -> bool {
    if field == "*" {
        return true;
    }

    // Support list: "1,2,5"
    if field.contains(',') {
        return field.split(',').any(|part| match_cron_field(part, val));
    }

    // Support step: "*/5"
    if let Some(stripped) = field.strip_prefix("*/") {
        if let Ok(step) = stripped.parse::<i32>() {
            return val % step == 0;
        }
    }

    // Support range: "1-5"
    if field.contains('-') {
        let parts: Vec<&str> = field.split('-').collect();
        if parts.len() == 2 {
            if let (Ok(start), Ok(end)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                return val >= start && val <= end;
            }
        }
    }

    // Exact value
    if let Ok(exact) = field.parse::<i32>() {
        return val == exact;
    }

    false
}

pub fn match_cron(cron_expr: &str, dt: &DateTimeDecomposed) -> bool {
    let fields: Vec<&str> = cron_expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }

    match_cron_field(fields[0], dt.minute)
        && match_cron_field(fields[1], dt.hour)
        && match_cron_field(fields[2], dt.day_of_month)
        && match_cron_field(fields[3], dt.month)
        && match_cron_field(fields[4], dt.day_of_week)
}

pub fn calculate_next_run(cron_expr: &str, start_secs: i64) -> Option<i64> {
    let fields: Vec<&str> = cron_expr.split_whitespace().collect();
    if fields.len() != 5 {
        return None;
    }

    // Start scanning from the next minute boundary
    let mut current = start_secs - (start_secs % 60) + 60;

    // Limit scan to 1 year forward (527,040 minutes)
    for _ in 0..527040 {
        let dt = decompose_timestamp(current);
        if match_cron(cron_expr, &dt) {
            return Some(current);
        }
        current += 60;
    }
    None
}

fn format_title(template: &str, timestamp_ms: i64) -> String {
    let dt = decompose_timestamp(timestamp_ms / 1000);
    let dt_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        dt.year, dt.month, dt.day_of_month, dt.hour, dt.minute
    );
    template.replace("{{datetime}}", &dt_str)
}

pub fn start_scheduler(workflows: Arc<dyn WorkflowRepository>, executor: Arc<dyn StepExecutor>) {
    tauri::async_runtime::spawn(async move {
        // Poll every 60 seconds
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = check_schedules(&*workflows, &*executor).await {
                tracing::error!(error = %e, "Scheduler error executing scheduled runs");
            }
        }
    });
}

async fn check_schedules(
    workflows: &dyn WorkflowRepository,
    executor: &dyn StepExecutor,
) -> Result<(), String> {
    let scheduled = workflows.list_scheduled()?;
    let now_secs = crate::paths::now_ms() / 1000;

    for w in scheduled {
        if let Some(ref s) = w.schedule {
            let next_run = match s.next_run_at {
                Some(val) => val,
                None => {
                    // Initialize schedule's next_run_at if empty
                    if let Some(computed) = calculate_next_run(&s.cron, now_secs) {
                        let _ = workflows.update_schedule_next_run(&w.id, Some(computed));
                        computed
                    } else {
                        continue;
                    }
                }
            };

            if now_secs >= next_run {
                let now_ms = crate::paths::now_ms();
                let title = format_title(&s.title_template, now_ms);
                let description = format!("Scheduled execution of workflow '{}'", w.name);

                tracing::info!(workflow = %w.name, workflow_id = %w.id.0, "Scheduler triggering scheduled workflow");
                if let Err(e) = executor
                    .feature_start(
                        &s.project_id.0,
                        &w.id.0,
                        &title,
                        &description,
                        None,
                        None,
                        None,
                        None,
                        Vec::new(),
                    )
                    .await
                {
                    tracing::warn!(workflow = %w.name, error = %e, "Scheduler failed to auto-start workflow");
                }

                // Recalculate next run date
                let next_next = calculate_next_run(&s.cron, now_secs);
                let _ = workflows.update_schedule_next_run(&w.id, next_next);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "../../tests/infrastructure/scheduler.rs"]
mod tests;
