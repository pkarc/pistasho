//! The pure side of the embeddings: what text is embedded, how vectors are stored and compared.
//! The model itself (ONNX Runtime, downloads, preloading) lives in `embed.rs`.
//!
//! Vectors are L2-normalised, so cosine similarity is a dot product, and they are stored as
//! `i8` (value × 127): 30k entries take 12 MB instead of 47 MB and the ranking does not change
//! in any way the eval can see.

use crate::index::Entry;

/// Dimensions of `multilingual-e5-small`.
pub const DIM: usize = 384;

/// The text that stands for an entry in the vector space: the tool, the description and the
/// start of the other-language descriptions. The command itself is left out on purpose: it is
/// mostly flags and `{{placeholders}}`.
pub fn passage_text(tool: &str, desc: &str, alt: &str) -> String {
    let alt: String = alt.chars().take(150).collect();
    format!("{tool}. {desc}. {alt}")
}

/// Whether an entry gets a vector. History has no description worth embedding and it is private
/// and changes constantly; BM25 still finds it.
pub fn embeds(e: &Entry) -> bool {
    e.source != crate::index::Source::Hist
}

/// Key of a vector: a hash of its passage, so an unchanged entry keeps its vector across
/// `update` and only new or edited ones are embedded again.
pub fn key(passage: &str) -> i64 {
    // FNV-1a, 64 bits: stable across Rust versions and platforms, unlike `DefaultHasher`
    let mut h: u64 = 0xcbf29ce484222325;
    for b in passage.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h as i64
}

pub fn quantize(v: &[f32]) -> Vec<i8> {
    v.iter()
        .map(|x| (x * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

/// All stored vectors: `ids[i]` is the entry whose vector is `data[i * DIM..(i + 1) * DIM]`.
#[derive(Default)]
pub struct VectorIndex {
    pub ids: Vec<i64>,
    pub data: Vec<i8>,
}

impl VectorIndex {
    pub fn push(&mut self, id: i64, v: &[u8]) {
        if v.len() == DIM {
            self.ids.push(id);
            self.data.extend(v.iter().map(|&b| b as i8));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The `k` entries closest to `query` (a normalised vector), best first, as `(id, cosine)`.
    pub fn top_k(&self, query: &[f32], k: usize) -> Vec<(i64, f32)> {
        if query.len() != DIM {
            return Vec::new();
        }
        let q = quantize(query);
        let mut scored: Vec<(i64, i32)> = self
            .data
            .chunks_exact(DIM)
            .zip(&self.ids)
            .map(|(row, &id)| {
                let dot: i32 = row.iter().zip(&q).map(|(&a, &b)| a as i32 * b as i32).sum();
                (id, dot)
            })
            .collect();
        let k = k.min(scored.len());
        if k == 0 {
            return Vec::new();
        }
        scored.select_nth_unstable_by(k - 1, |a, b| b.1.cmp(&a.1));
        scored.truncate(k);
        scored.sort_by_key(|&(_, dot)| std::cmp::Reverse(dot));
        scored
            .into_iter()
            .map(|(id, dot)| (id, dot as f32 / (127.0 * 127.0)))
            .collect()
    }
}

/// Reciprocal rank fusion: each ranking gives `1 / (k + rank)` to what it lists, and what
/// several rankings agree on rises. Rank-based, so BM25 scores and cosines never have to be
/// put on one scale. Each ranking has a weight (1.0 = plain RRF). Returns `(item, fused score)`
/// best first.
pub fn rrf<K: std::hash::Hash + Eq + Clone>(rankings: &[(Vec<K>, f64)], k: f64) -> Vec<(K, f64)> {
    let mut score: std::collections::HashMap<K, f64> = std::collections::HashMap::new();
    let mut order: Vec<K> = Vec::new();
    for (ranking, weight) in rankings {
        for (pos, item) in ranking.iter().enumerate() {
            let s = score.entry(item.clone()).or_insert_with(|| {
                order.push(item.clone());
                0.0
            });
            *s += weight / (k + pos as f64 + 1.0);
        }
    }
    let mut out: Vec<(K, f64)> = order
        .into_iter()
        .map(|i| {
            let s = score[&i];
            (i, s)
        })
        .collect();
    // stable: ties keep the order in which they first appeared (the first ranking wins)
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(i: usize) -> Vec<f32> {
        let mut v = vec![0.0; DIM];
        v[i] = 1.0;
        v
    }

    #[test]
    fn key_is_stable_and_distinguishes_passages() {
        assert_eq!(key("tar. create an archive. "), key("tar. create an archive. "));
        assert_ne!(key("tar. create an archive. "), key("tar. extract an archive. "));
        // pinned: a changed hash would silently throw away every stored vector
        assert_eq!(key(""), 0xcbf29ce484222325u64 as i64);
    }

    #[test]
    fn passage_keeps_only_the_start_of_alt() {
        let p = passage_text("ls", "list files", &"é".repeat(400));
        assert_eq!(p.chars().count(), "ls. list files. ".len() + 150);
    }

    #[test]
    fn nearest_vector_comes_first() {
        let mut idx = VectorIndex::default();
        for i in 0..5 {
            let bytes: Vec<u8> = quantize(&unit(i)).into_iter().map(|b| b as u8).collect();
            idx.push(100 + i as i64, &bytes);
        }
        let hits = idx.top_k(&unit(3), 2);
        assert_eq!(hits[0].0, 103);
        assert!((hits[0].1 - 1.0).abs() < 0.01);
        assert!(hits[1].1.abs() < 0.01);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn wrong_sized_vectors_are_ignored() {
        let mut idx = VectorIndex::default();
        idx.push(1, &[0u8; 3]);
        assert!(idx.is_empty());
        assert!(idx.top_k(&[1.0, 0.0], 3).is_empty());
    }

    #[test]
    fn rrf_rewards_agreement() {
        let fused = rrf(&[(vec!["a", "b", "c"], 1.0), (vec!["b", "c", "d"], 1.0)], 60.0);
        let order: Vec<_> = fused.iter().map(|(k, _)| *k).collect();
        // b and c are listed by both rankings, a and d by one
        assert_eq!(order, vec!["b", "c", "a", "d"]);
    }
}
