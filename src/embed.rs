pub const EMBED_DIM: usize = 384;
pub const MODEL_ID: &str = "token-hash-v1";

/// Local embedder. Token hashes keep recall working with no model download.
/// Shared words land in the same dimensions, so a paraphrase that keeps the
/// important words still ranks near the stored statement.
pub fn embed(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0f32; EMBED_DIM];
    let tokens = tokenize(text);
    for window in tokens.windows(2) {
        add_feature(&mut vector, &format!("{} {}", window[0], window[1]), 0.5);
    }
    for token in &tokens {
        add_feature(&mut vector, token, 1.0);
    }
    normalize(&mut vector);
    vector
}

pub fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (a, b) in left.iter().zip(right) {
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 1.0;
    }
    1.0 - (dot / (left_norm.sqrt() * right_norm.sqrt()))
}

fn add_feature(vector: &mut [f32], token: &str, weight: f32) {
    let hash = fnv(token.as_bytes());
    let index = (hash as usize) % EMBED_DIM;
    let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
    vector[index] += sign * weight;
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() > 2)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_words_are_closer_than_unrelated_text() {
        let seeded = embed("seed the local database before integration tests");
        let tests = embed("integration tests need the database seeded");
        let other = embed("kubernetes helm chart for payments");
        assert!(cosine_distance(&seeded, &tests) < cosine_distance(&seeded, &other));
    }
}
