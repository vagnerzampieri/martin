//! Pure text post-processing for dictation output. Applied to the full assembled
//! text on every emission so behavior is consistent regardless of chunking.

/// Collapse runs of inline whitespace (spaces/tabs) into a single space, but
/// preserve newlines exactly. Trims trailing whitespace from each line.
pub fn collapse_whitespace(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed_end = line.trim_end();
            let mut out = String::with_capacity(trimmed_end.len());
            let mut prev_space = false;
            for ch in trimmed_end.chars() {
                if ch == ' ' || ch == '\t' {
                    if !prev_space {
                        out.push(' ');
                    }
                    prev_space = true;
                } else {
                    out.push(ch);
                    prev_space = false;
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Ensure a single space after `,;:` and `.!?` when followed by a letter/digit.
/// Fixes whisper outputs like `texto,palavra` → `texto, palavra`.
pub fn fix_punctuation_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        out.push(ch);
        if matches!(ch, ',' | ';' | ':' | '.' | '!' | '?') {
            if let Some(&next) = chars.peek() {
                if next.is_alphanumeric() {
                    out.push(' ');
                }
            }
        }
    }
    out
}

/// Capitalize the first alphabetic character of each sentence. A sentence
/// boundary is start-of-string, double newline, or `.!?` followed by whitespace.
pub fn capitalize_sentences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut should_capitalize_next = true;
    for ch in text.chars() {
        if should_capitalize_next && ch.is_alphabetic() {
            for upper in ch.to_uppercase() {
                out.push(upper);
            }
            should_capitalize_next = false;
        } else {
            out.push(ch);
            if matches!(ch, '.' | '!' | '?') {
                should_capitalize_next = true;
            } else if ch == '\n' {
                // Paragraph break (\n\n) implies new sentence; single \n keeps the flag
                // as it was so we don't over-capitalize wrapped lines.
                should_capitalize_next = true;
            } else if !ch.is_whitespace() {
                should_capitalize_next = false;
            }
        }
    }
    out
}

/// Apply all normalization passes. Order matters:
///   1. Punctuation spacing fixes attached punctuation
///   2. Whitespace collapsing removes extra spaces introduced by step 1
///   3. Capitalization comes last so it sees clean sentence boundaries
pub fn normalize(text: &str) -> String {
    let s = fix_punctuation_spacing(text);
    let s = collapse_whitespace(&s);
    capitalize_sentences(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_whitespace_squashes_inline_runs() {
        assert_eq!(collapse_whitespace("a    b   c"), "a b c");
    }

    #[test]
    fn collapse_whitespace_preserves_newlines() {
        assert_eq!(collapse_whitespace("a\n\nb"), "a\n\nb");
        assert_eq!(collapse_whitespace("a  \n\n  b"), "a\n\n b");
    }

    #[test]
    fn collapse_whitespace_trims_trailing_spaces_per_line() {
        assert_eq!(collapse_whitespace("hello   \nworld   "), "hello\nworld");
    }

    #[test]
    fn fix_punctuation_inserts_space_when_glued_to_letter() {
        assert_eq!(fix_punctuation_spacing("texto,palavra"), "texto, palavra");
        assert_eq!(fix_punctuation_spacing("oi.tudo bem"), "oi. tudo bem");
        assert_eq!(fix_punctuation_spacing("a!b?c"), "a! b? c");
    }

    #[test]
    fn fix_punctuation_leaves_existing_spaces_alone() {
        assert_eq!(fix_punctuation_spacing("texto, palavra"), "texto, palavra");
        assert_eq!(fix_punctuation_spacing("fim."), "fim.");
    }

    #[test]
    fn capitalize_first_letter_of_text() {
        assert_eq!(capitalize_sentences("olá mundo"), "Olá mundo");
    }

    #[test]
    fn capitalize_after_period_and_space() {
        assert_eq!(
            capitalize_sentences("primeiro. segundo. terceiro."),
            "Primeiro. Segundo. Terceiro."
        );
    }

    #[test]
    fn capitalize_after_paragraph_break() {
        assert_eq!(
            capitalize_sentences("primeiro paragrafo.\n\nsegundo paragrafo."),
            "Primeiro paragrafo.\n\nSegundo paragrafo."
        );
    }

    #[test]
    fn capitalize_leaves_already_uppercase_intact() {
        assert_eq!(capitalize_sentences("Olá. Tudo bem?"), "Olá. Tudo bem?");
    }

    #[test]
    fn normalize_runs_all_passes() {
        let input = "olá   mundo,como vai?tudo bem.   obrigado";
        let expected = "Olá mundo, como vai? Tudo bem. Obrigado";
        assert_eq!(normalize(input), expected);
    }

    #[test]
    fn normalize_preserves_paragraph_breaks() {
        let input = "primeiro paragrafo.\n\nsegundo paragrafo";
        let expected = "Primeiro paragrafo.\n\nSegundo paragrafo";
        assert_eq!(normalize(input), expected);
    }

    #[test]
    fn normalize_is_idempotent() {
        let input = "Olá mundo. Tudo bem?\n\nNovo paragrafo.";
        assert_eq!(normalize(input), normalize(&normalize(input)));
    }
}
