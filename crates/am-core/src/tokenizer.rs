use rand::Rng;
use regex::Regex;
use std::sync::LazyLock;

use crate::episode::Episode;
use crate::neighborhood::Neighborhood;

static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w\s']").unwrap());
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[.!?]\s+").unwrap());
static APOSTROPHE_TRIM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^'+|'+$").unwrap());

/// Tokenize text into lowercase words.
/// Preserves apostrophes within words (e.g., "don't").
/// No stemming, no stop-word removal — IDF handles frequency naturally.
pub fn tokenize(text: &str) -> Vec<String> {
    let cleaned = NON_WORD.replace_all(text, " ");
    cleaned
        .to_lowercase()
        .split_whitespace()
        .map(|t| APOSTROPHE_TRIM.replace_all(t, "").to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Split text into sentences at sentence-ending punctuation followed by whitespace.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut last = 0;

    for m in SENTENCE_END.find_iter(text) {
        let end = m.end();
        let sentence = text[last..m.start() + 1].trim().to_string(); // include the punctuation
        if !sentence.is_empty() {
            sentences.push(sentence);
        }
        last = end;
    }

    // Remaining text after last sentence boundary
    let remainder = text[last..].trim().to_string();
    if !remainder.is_empty() {
        sentences.push(remainder);
    }

    sentences
}

/// Ingest text into an Episode.
/// Splits into 3-sentence chunks, each becoming a Neighborhood.
pub fn ingest_text(text: &str, name: Option<&str>, rng: &mut impl Rng) -> Episode {
    let mut episode = Episode::new(name.unwrap_or(""));
    let sentences = split_sentences(text);
    let chunk_size = 3;

    for chunk in sentences.chunks(chunk_size) {
        let combined = chunk.join(" ");
        let tokens = tokenize(&combined);
        if !tokens.is_empty() {
            let neighborhood = Neighborhood::from_tokens(&tokens, None, &combined, rng);
            episode.add_neighborhood(neighborhood);
        }
    }

    episode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokenize() {
        let tokens = tokenize("Hello, world!");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_apostrophe_preserved() {
        let tokens = tokenize("Don't stop!");
        assert_eq!(tokens, vec!["don't", "stop"]);
    }

    #[test]
    fn test_leading_trailing_apostrophes_stripped() {
        let tokens = tokenize("'hello' 'world'");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_empty_input() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_whitespace_only() {
        let tokens = tokenize("   \t\n  ");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_punctuation_stripped() {
        let tokens = tokenize("hello! world? foo.");
        assert_eq!(tokens, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn test_numbers_preserved() {
        let tokens = tokenize("test 123 hello");
        assert_eq!(tokens, vec!["test", "123", "hello"]);
    }

    #[test]
    fn test_sentence_splitting() {
        let sentences = split_sentences("First. Second! Third? Fourth.");
        assert_eq!(sentences.len(), 4);
    }

    #[test]
    fn test_ingest_text_3_sentence_chunks() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42);
        use rand::SeedableRng;
        let text = "First sentence. Second sentence. Third sentence. Fourth sentence. Fifth sentence. Sixth sentence.";
        let ep = ingest_text(text, Some("test"), &mut rng);

        // 6 sentences / 3 per chunk = 2 neighborhoods
        assert_eq!(ep.neighborhoods.len(), 2);
        assert_eq!(ep.name, "test");
    }

    #[test]
    fn test_ingest_text_empty() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(42);
        use rand::SeedableRng;
        let ep = ingest_text("", None, &mut rng);
        assert_eq!(ep.neighborhoods.len(), 0);
    }

    #[test]
    fn test_no_stemming() {
        let tokens = tokenize("running runs ran runner");
        assert_eq!(tokens, vec!["running", "runs", "ran", "runner"]);
    }

    #[test]
    fn test_no_stop_word_removal() {
        let tokens = tokenize("the a an is are was");
        assert_eq!(tokens, vec!["the", "a", "an", "is", "are", "was"]);
    }
}
