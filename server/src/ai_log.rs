use sqlx::MySqlPool;

/// Log an AI request to the ai_logs table.
pub async fn log_ai_request(
    db: &MySqlPool,
    request_type: &str,
    model: &str,
    duration_ms: u64,
    status: &str,
    error_message: Option<&str>,
    context: Option<&str>,
    input_params: Option<&str>,
    output_result: Option<&str>,
) {
    let ctx = context.map(|c| crate::truncate_chars(c, 2000));
    let _ = sqlx::query(
        "INSERT INTO ai_logs (request_type, model, duration_ms, status, error_message, context, input_params, output_result) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(request_type)
    .bind(model)
    .bind(duration_ms as i32)
    .bind(status)
    .bind(error_message)
    .bind(ctx)
    .bind(input_params)
    .bind(output_result)
    .execute(db)
    .await;
}
