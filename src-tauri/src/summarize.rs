use std::process::Command;

pub fn is_claude_cli_available() -> bool {
    Command::new("which")
        .arg("claude")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn build_prompt(transcription_text: &str) -> String {
    format!(
        "Resuma esta transcrição de reunião. Inclua um resumo geral e os key points principais:\n\n{}",
        transcription_text
    )
}

pub fn call_claude_cli(prompt: &str) -> Result<String, String> {
    let output = Command::new("claude")
        .arg("-p")
        .arg(prompt)
        .output()
        .map_err(|e| format!("Failed to run claude CLI: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("claude CLI failed: {}", stderr));
    }

    let summary = String::from_utf8(output.stdout)
        .map_err(|e| format!("Invalid UTF-8 from claude CLI: {}", e))?;

    Ok(summary.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_includes_transcription_text() {
        let text = "Alice said hello. Bob replied.";
        let prompt = build_prompt(text);

        assert!(prompt.contains("Resuma esta transcrição de reunião"));
        assert!(prompt.contains("key points"));
        assert!(prompt.contains(text));
    }

    #[test]
    fn build_prompt_handles_empty_text() {
        let prompt = build_prompt("");

        assert!(prompt.contains("Resuma esta transcrição de reunião"));
        assert!(prompt.ends_with("\n\n"));
    }
}
