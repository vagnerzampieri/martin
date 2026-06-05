//! Builds Whisper's `initial_prompt` from the user's glossary terms.
//!
//! The prompt is a plain comma-separated list — no prefix sentence, so it
//! stays language-neutral (works for both pt and en transcriptions).

/// Whisper's prompt window is ~224 tokens. 700 is a safe margin below
/// that for both pt and en text. Measured in BYTES (`str::len`), which
/// is intentionally conservative for multi-byte text: accented terms
/// consume budget faster, never overflowing the real token window.
pub const MAX_PROMPT_CHARS: usize = 700;

/// Joins glossary terms into a comma-separated prompt, dropping terms
/// (in insertion order) once the cap would be exceeded. Returns `None`
/// when no term fits — callers then transcribe without a prompt.
pub fn build_initial_prompt(terms: &[String]) -> Option<String> {
    let mut prompt = String::new();

    for term in terms {
        let term = term.trim();
        if term.is_empty() {
            continue;
        }
        let needed = if prompt.is_empty() {
            term.len()
        } else {
            term.len() + 2 // ", "
        };
        if prompt.len() + needed > MAX_PROMPT_CHARS {
            break;
        }
        if !prompt.is_empty() {
            prompt.push_str(", ");
        }
        prompt.push_str(term);
    }

    if prompt.is_empty() {
        None
    } else {
        Some(prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_terms_produce_no_prompt() {
        assert_eq!(build_initial_prompt(&[]), None);
    }

    #[test]
    fn blank_terms_produce_no_prompt() {
        let terms = vec!["".to_string(), "   ".to_string()];
        assert_eq!(build_initial_prompt(&terms), None);
    }

    #[test]
    fn single_term_is_the_prompt() {
        let terms = vec!["PipeWire".to_string()];
        assert_eq!(build_initial_prompt(&terms), Some("PipeWire".to_string()));
    }

    #[test]
    fn terms_join_with_comma_and_space() {
        let terms = vec![
            "PipeWire".to_string(),
            "Tauri".to_string(),
            "Svelte".to_string(),
        ];
        assert_eq!(
            build_initial_prompt(&terms),
            Some("PipeWire, Tauri, Svelte".to_string())
        );
    }

    #[test]
    fn terms_are_trimmed() {
        let terms = vec!["  PipeWire  ".to_string(), " Tauri".to_string()];
        assert_eq!(
            build_initial_prompt(&terms),
            Some("PipeWire, Tauri".to_string())
        );
    }

    #[test]
    fn cap_is_measured_in_bytes_for_multibyte_terms() {
        // "ção" is 3 chars but 5 bytes; budget is consumed by bytes.
        let term = "ç".repeat(MAX_PROMPT_CHARS / 2 + 1); // > cap in bytes, < cap in chars
        assert_eq!(build_initial_prompt(&[term]), None);
    }

    #[test]
    fn terms_beyond_the_cap_are_dropped_in_insertion_order() {
        // First term fills most of the cap; second still fits; third does not.
        let big = "x".repeat(MAX_PROMPT_CHARS - 10);
        let terms = vec![big.clone(), "12345678".to_string(), "dropped".to_string()];
        let prompt = build_initial_prompt(&terms).expect("prompt expected");
        assert_eq!(prompt, format!("{}, 12345678", big));
        assert!(prompt.len() <= MAX_PROMPT_CHARS);
    }

    #[test]
    fn a_single_term_longer_than_the_cap_is_dropped() {
        let too_big = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert_eq!(build_initial_prompt(&[too_big]), None);
    }
}
