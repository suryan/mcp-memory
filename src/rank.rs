//! Fuses vector, keyword, and graph evidence into one ranked recall result.

use serde::Serialize;

/// One memory found by any leg of a recall, before fusion.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Memory id. Candidates with the same id are merged.
    pub id: String,
    /// Memory title.
    pub title: String,
    /// Memory summary.
    pub summary: String,
    /// Memory content.
    pub content: String,
    /// Memory type name.
    pub memory_type: String,
    /// Scope name.
    pub scope: String,
    /// Repository key, empty for global memories.
    pub repo_key: String,
    /// Cosine distance from the vector leg; lower is closer.
    pub vector_distance: Option<f64>,
    /// BM25 score from the keyword leg; higher is better.
    pub text_score: Option<f64>,
    /// Reached through an entity link.
    pub from_graph: bool,
}

/// One memory in a recall result.
#[derive(Debug, Clone, Serialize)]
pub struct RankedHit {
    /// Memory id.
    pub id: String,
    /// Memory title.
    pub title: String,
    /// Memory summary.
    pub summary: String,
    /// Content, truncated to the recall byte budget.
    pub content: String,
    /// Memory type name.
    #[serde(rename = "type")]
    pub memory_type: String,
    /// Scope name.
    pub scope: String,
    /// Repository key, empty for global memories.
    pub repo: String,
    /// Fused score; higher is better.
    pub score: f64,
}

const VECTOR_MAX_DISTANCE: f64 = 0.72;
const RRF_K: f64 = 60.0;

/// Reciprocal-rank-fusion term for a zero-based position.
fn rrf(position: usize) -> f64 {
    1.0 / (RRF_K + f64::from(u32::try_from(position).unwrap_or(u32::MAX)) + 1.0)
}

/// Fuse vector, keyword, and graph evidence. Weak matches abstain. At most `limit`
/// hits, with total content within `byte_budget`.
pub fn rank(mut candidates: Vec<Candidate>, limit: usize, byte_budget: usize) -> Vec<RankedHit> {
    candidates.retain(|candidate| {
        candidate.from_graph
            || candidate
                .vector_distance
                .is_some_and(|distance| distance <= VECTOR_MAX_DISTANCE)
            || candidate.text_score.is_some_and(|score| score > 0.0)
    });
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut by_id: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if let Some(existing) = by_id.iter_mut().find(|item| item.id == candidate.id) {
            existing.vector_distance = existing.vector_distance.or(candidate.vector_distance);
            existing.text_score = existing.text_score.or(candidate.text_score);
            existing.from_graph |= candidate.from_graph;
        } else {
            by_id.push(candidate);
        }
    }

    let mut vector_order: Vec<(String, f64)> = by_id
        .iter()
        .filter_map(|candidate| {
            candidate
                .vector_distance
                .map(|distance| (candidate.id.clone(), distance))
        })
        .collect();
    vector_order.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut text_order: Vec<(String, f64)> = by_id
        .iter()
        .filter_map(|candidate| candidate.text_score.map(|score| (candidate.id.clone(), score)))
        .collect();
    text_order.sort_by(|left, right| right.1.partial_cmp(&left.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut ranked: Vec<(f64, RankedHit)> = by_id
        .into_iter()
        .map(|candidate| {
            let mut score = 0.0;
            if let Some(position) = vector_order.iter().position(|(id, _)| id == &candidate.id) {
                score += rrf(position);
            }
            if let Some(position) = text_order.iter().position(|(id, _)| id == &candidate.id) {
                score += rrf(position);
            }
            if candidate.from_graph {
                score += 0.05;
            }
            if candidate.scope == "repository" {
                score += 0.005;
            }
            (
                score,
                RankedHit {
                    id: candidate.id,
                    title: candidate.title,
                    summary: candidate.summary,
                    content: candidate.content,
                    memory_type: candidate.memory_type,
                    scope: candidate.scope,
                    repo: candidate.repo_key,
                    score,
                },
            )
        })
        .collect();
    ranked.sort_by(|left, right| right.0.partial_cmp(&left.0).unwrap_or(std::cmp::Ordering::Equal));
    ranked.retain(|(score, _)| *score >= 0.015);

    let mut used = 0usize;
    let mut hits = Vec::new();
    for (_, mut hit) in ranked {
        if hits.len() >= limit {
            break;
        }
        let remaining = byte_budget.saturating_sub(used);
        if remaining == 0 {
            break;
        }
        if hit.content.len() > remaining {
            let mut end = remaining;
            while !hit.content.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            hit.content.truncate(end);
        }
        used += hit.content.len();
        hits.push(hit);
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, distance: Option<f64>, text: Option<f64>, graph: bool) -> Candidate {
        Candidate {
            id: id.into(),
            title: id.into(),
            summary: "summary".into(),
            content: "content".into(),
            memory_type: "fact".into(),
            scope: "repository".into(),
            repo_key: "repo".into(),
            vector_distance: distance,
            text_score: text,
            from_graph: graph,
        }
    }

    #[test]
    fn drops_a_far_vector_with_no_other_evidence() {
        let hits = rank(vec![candidate("far", Some(0.95), None, false)], 5, 8000);
        assert!(hits.is_empty());
    }

    #[test]
    fn keeps_a_graph_hit() {
        let hits = rank(vec![candidate("linked", Some(0.9), None, true)], 5, 8000);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "linked");
    }
}
