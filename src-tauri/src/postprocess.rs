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

/// Replace spoken formatting commands with their punctuation/whitespace equivalents.
/// Matches are case-insensitive and respect word boundaries — `avírgulab` is left alone.
/// Order matters: longest phrases must be tried first so `ponto de interrogação`
/// is not eaten by `ponto final`.
pub fn replace_voice_commands(text: &str) -> String {
    // Sorted longest-first to prevent shorter phrases from cannibalizing longer ones.
    const COMMANDS: &[(&str, &str)] = &[
        ("ponto de interrogação", "?"),
        ("ponto de exclamação", "!"),
        ("novo parágrafo", "\n\n"),
        ("nova linha", "\n"),
        ("ponto final", "."),
        ("abre aspas", "\""),
        ("fecha aspas", "\""),
        ("vírgula", ","),
    ];

    let mut result = String::with_capacity(text.len());
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let original: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < original.len() {
        let mut matched = false;
        for &(phrase, replacement) in COMMANDS {
            let phrase_chars: Vec<char> = phrase.chars().collect();
            if i + phrase_chars.len() > original.len() {
                continue;
            }
            if !lower[i..i + phrase_chars.len()]
                .iter()
                .zip(phrase_chars.iter())
                .all(|(a, b)| a == b)
            {
                continue;
            }
            // Word boundaries: char before must be non-alphabetic (or start),
            // char after must be non-alphabetic (or end).
            let before_ok = i == 0 || !original[i - 1].is_alphabetic();
            let after_idx = i + phrase_chars.len();
            let after_ok = after_idx == original.len() || !original[after_idx].is_alphabetic();
            if !before_ok || !after_ok {
                continue;
            }

            // Eat one leading space already in `result` if this replacement starts a new line
            // or is a punctuation that should be glued to the previous word.
            // Special case: "fecha aspas" (closing quote) should eat the leading space,
            // but "abre aspas" (opening quote) should not.
            let is_closing_quote = phrase == "fecha aspas";
            if matches!(replacement, "\n" | "\n\n" | "." | "," | "?" | "!")
                && result.ends_with(' ')
            {
                result.pop();
            }
            if is_closing_quote && result.ends_with(' ') {
                result.pop();
            }
            result.push_str(replacement);
            i = after_idx;
            // Eat one trailing space after the command.
            // For newlines, always eat; for any quote, always eat.
            if i < original.len() && original[i] == ' ' {
                if matches!(replacement, "\n" | "\n\n" | "\"") {
                    i += 1;
                }
            }
            matched = true;
            break;
        }

        if !matched {
            result.push(original[i]);
            i += 1;
        }
    }

    result
}

/// Apply all normalization passes. Order matters:
///   1. Voice command substitutions (must run first so we capitalize correctly later)
///   2. Punctuation spacing fixes
///   3. Whitespace collapsing
///   4. Capitalization
pub fn normalize(text: &str) -> String {
    let s = replace_voice_commands(text);
    let s = fix_punctuation_spacing(&s);
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

    #[test]
    fn voice_commands_replace_paragraph_command() {
        assert_eq!(
            replace_voice_commands("primeiro novo parágrafo segundo"),
            "primeiro\n\nsegundo"
        );
    }

    #[test]
    fn voice_commands_replace_newline_command() {
        assert_eq!(
            replace_voice_commands("linha um nova linha linha dois"),
            "linha um\nlinha dois"
        );
    }

    #[test]
    fn voice_commands_replace_punctuation_commands() {
        assert_eq!(
            replace_voice_commands("texto vírgula mais texto ponto final"),
            "texto, mais texto."
        );
        assert_eq!(
            replace_voice_commands("isso ponto de interrogação"),
            "isso?"
        );
        assert_eq!(
            replace_voice_commands("uau ponto de exclamação"),
            "uau!"
        );
    }

    #[test]
    fn voice_commands_replace_quote_commands() {
        assert_eq!(
            replace_voice_commands("ele disse abre aspas oi fecha aspas"),
            "ele disse \"oi\""
        );
    }

    #[test]
    fn voice_commands_are_case_insensitive() {
        assert_eq!(
            replace_voice_commands("Novo Parágrafo segundo"),
            "\n\nsegundo"
        );
        assert_eq!(
            replace_voice_commands("texto VÍRGULA mais"),
            "texto, mais"
        );
    }

    #[test]
    fn voice_commands_ignore_substring_inside_word() {
        // "vírgula" inside a longer word should NOT match
        assert_eq!(
            replace_voice_commands("avírgulab"),
            "avírgulab"
        );
    }

    #[test]
    fn normalize_applies_voice_commands_first() {
        let input = "olá vírgula tudo bem ponto de interrogação";
        let expected = "Olá, tudo bem?";
        assert_eq!(normalize(input), expected);
    }
}
