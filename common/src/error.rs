pub fn format_error(error: anyhow::Error) -> String {
    error
        .chain()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\nCaused by:\n    ")
}
