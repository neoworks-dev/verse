//! Fixing words whisper does not know: the vocabulary is passed as its prompt, then whatever it
//! still mishears is matched back to the vocabulary by sound, and the user's replacements run last.

use rphonetic::DoubleMetaphone;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

/// How many transcript words one vocabulary term may have been split into ("cube cuddle").
const MAX_SPAN: usize = 3;

/// Whisper reads about 224 tokens of prompt; stay well inside that.
const MAX_PROMPT_CHARS: usize = 600;

/// A glossary for whisper's initial prompt, which makes these spellings much more likely.
pub fn prompt(vocabulary: &[String]) -> Option<String> {
    let mut prompt = String::new();
    for term in vocabulary.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if prompt.len() + term.len() + 2 > MAX_PROMPT_CHARS {
            break;
        }
        if !prompt.is_empty() {
            prompt.push_str(", ");
        }
        prompt.push_str(term);
    }
    (!prompt.is_empty()).then(|| prompt + ".")
}

pub fn correct(text: &str, vocabulary: &[String], replacements: &[Replacement]) -> String {
    let text = match_sounds(text, vocabulary);
    replace(&text, replacements)
}

struct Term<'a> {
    text: &'a str,
    letters: String,
    codes: (String, String),
}

fn match_sounds(text: &str, vocabulary: &[String]) -> String {
    let encoder = DoubleMetaphone::new(Some(16));
    let codes = |s: &str| {
        let r = encoder.double_metaphone(s);
        (r.primary(), r.alternate())
    };
    let terms: Vec<Term> = vocabulary
        .iter()
        .map(|t| t.trim())
        .filter_map(|text| {
            let letters = letters(text);
            (letters.len() >= 3).then(|| Term { text, codes: codes(&letters), letters })
        })
        .collect();
    if terms.is_empty() {
        return text.to_string();
    }

    let find = |span: &[&str]| -> Option<&Term> {
        // A term never spans a sentence or clause break.
        if span[..span.len() - 1].iter().any(|w| w.ends_with(['.', ',', '!', '?', ';', ':'])) {
            return None;
        }
        let heard: String = span.iter().map(|w| letters(w)).collect();
        if heard.len() < 3 {
            return None;
        }
        let heard_codes = codes(&heard);
        terms.iter().find(|t| sounds_like(&heard, &heard_codes, span.len(), t))
    };

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::with_capacity(words.len());
    let mut i = 0;
    'words: while i < words.len() {
        for n in (1..=MAX_SPAN.min(words.len() - i)).rev() {
            let span = &words[i..i + n];
            let Some(term) = find(span) else { continue };
            // Vowel sounds barely register, so "a use effect" also sounds like "useEffect".
            // Leave the extra word out when the span without it still matches.
            if n > 1 && find(&span[1..]).is_some() {
                break;
            }
            let (lead, _, _) = split_punctuation(span[0]);
            let (_, _, trail) = split_punctuation(span[n - 1]);
            out.push(format!("{lead}{}{trail}", term.text));
            i += n;
            continue 'words;
        }
        out.push(words[i].to_string());
        i += 1;
    }
    out.join(" ")
}

fn sounds_like(heard: &str, codes: &(String, String), words: usize, term: &Term) -> bool {
    if heard == term.letters {
        // Same letters, so only case or spacing differ: "use effect" for "useEffect". For a
        // single word only casing like "GitHub" or "JSON" is worth restoring; a plain
        // capitalised term ("Bun", "Docker") is also an ordinary word or sits at a sentence start.
        return words > 1 || term.text.chars().skip(1).any(char::is_uppercase);
    }
    let same_sound = codes.0 == term.codes.0
        || codes.0 == term.codes.1
        || codes.1 == term.codes.0
        || codes.1 == term.codes.1;
    if !same_sound || term.codes.0.len() < 3 {
        return false;
    }
    if words > 1 {
        // A real word split into several heard ones; a long enough sound code is evidence enough.
        return term.codes.0.len() >= 4;
    }
    // One heard word that sounds alike is often just a different real word ("roast" for "Rust"),
    // so it also has to be spelled nearly the same.
    let allowed = match term.letters.len() {
        0..5 => return false,
        5..8 => 1,
        _ => 2,
    };
    levenshtein(heard, &term.letters) <= allowed
}

/// Case-insensitive whole-word replacement of the user's own rules.
fn replace(text: &str, replacements: &[Replacement]) -> String {
    let rules: Vec<(Vec<String>, &str)> = replacements
        .iter()
        .filter_map(|r| {
            let from: Vec<String> = r.from.split_whitespace().map(letters).collect();
            (!from.is_empty() && from.iter().all(|w| !w.is_empty())).then_some((from, r.to.as_str()))
        })
        .collect();
    if rules.is_empty() {
        return text.to_string();
    }

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    'words: while i < words.len() {
        for (from, to) in &rules {
            let n = from.len();
            if i + n <= words.len() && words[i..i + n].iter().zip(from).all(|(w, f)| letters(w) == *f) {
                let (lead, _, _) = split_punctuation(words[i]);
                let (_, _, trail) = split_punctuation(words[i + n - 1]);
                out.push(format!("{lead}{to}{trail}"));
                i += n;
                continue 'words;
            }
        }
        out.push(words[i].to_string());
        i += 1;
    }
    out.join(" ")
}

fn letters(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Splits "(word)," into "(", "word", "),".
fn split_punctuation(word: &str) -> (&str, &str, &str) {
    let start = word.find(char::is_alphanumeric).unwrap_or(word.len());
    let end = word.rfind(char::is_alphanumeric).map_or(start, |i| i + word[i..].chars().next().unwrap().len_utf8());
    (&word[..start], &word[start..end], &word[end..])
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (prev + (ca != *cb) as usize).min(row[j] + 1).min(row[j + 1] + 1);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(terms: &[&str]) -> Vec<String> {
        terms.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn rejoins_split_terms() {
        let v = vocab(&["kubectl", "useEffect", "Hyprland"]);
        assert_eq!(match_sounds("run cube cuddle get pods.", &v), "run kubectl get pods.");
        assert_eq!(match_sounds("wrap it in a use effect,", &v), "wrap it in a useEffect,");
        assert_eq!(match_sounds("push it to github", &vocab(&["GitHub"])), "push it to GitHub");
    }

    #[test]
    fn presets_leave_everyday_speech_alone() {
        let all: Vec<String> = crate::presets::PRESETS
            .iter()
            .flat_map(|p| p.terms.iter().map(|t| t.to_string()))
            .collect();
        let speech = "Docker said the hot dog bun was cold, so we went fishing at the lake. \
                      I'd like to rest a bit, then call my mom and talk to you about the weather. \
                      Could you grab a few things from the store? The view from the hill was \
                      great and the rug looks nice. Let's sync up after lunch, okay?";
        assert_eq!(match_sounds(speech, &all), speech);
    }

    #[test]
    fn fixes_near_spellings() {
        let v = vocab(&["neoworks", "PostgreSQL"]);
        assert_eq!(match_sounds("open neoworx now", &v), "open neoworks now");
        assert_eq!(match_sounds("(postgresql)", &v), "(PostgreSQL)");
    }

    #[test]
    fn leaves_real_words_alone() {
        let v = vocab(&["Rust", "tokio"]);
        assert_eq!(match_sounds("the roast was good", &v), "the roast was good");
        assert_eq!(match_sounds("I'm going to Tokyo.", &v), "I'm going to Tokyo.");
    }

    #[test]
    fn does_not_cross_sentences() {
        let v = vocab(&["kubectl"]);
        assert_eq!(match_sounds("a cube. Cuddle it", &v), "a cube. Cuddle it");
    }

    #[test]
    fn replaces_phrases() {
        let r = vec![
            Replacement { from: "pseudo".into(), to: "sudo".into() },
            Replacement { from: "rust c".into(), to: "rustc".into() },
        ];
        assert_eq!(replace("Pseudo pacman, then Rust C main.rs.", &r), "sudo pacman, then rustc main.rs.");
    }

    #[test]
    fn caps_the_prompt() {
        let many: Vec<String> = (0..500).map(|i| format!("term{i}")).collect();
        let p = prompt(&many).unwrap();
        assert!(p.len() <= MAX_PROMPT_CHARS + 1 && p.ends_with('.'));
        assert_eq!(prompt(&vocab(&["a", " "])), Some("a.".into()));
        assert_eq!(prompt(&[]), None);
    }
}
