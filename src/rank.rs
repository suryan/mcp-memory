use serde::Serialize;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub content: String,
    pub memory_type: String,
    pub scope: String,
    pub repo_key: String,
    pub vector_distance: Option<f32>,
    pub text_score: Option<f32>,
    pub from_graph: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedHit {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub content: String,
    #[serde(rename = "type")]
    pub memory_type: String,
    pub scope: String,
    pub repo: String,
    pub score: f32,
}

const VECTOR_MAX_DISTANCE: f32 = 0.72;
const RRF_K: f32 = 60.0;

/// Fuse vector, keyword, and graph evidence. Weak matches abstain.
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

    let mut vector_order: Vec<(String, f32)> = by_id
        .iter()
        .filter_map(|candidate| {
            candidate
                .vector_distance
                .map(|distance| (candidate.id.clone(), distance))
        })
        .collect();
    vector_order.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut text_order: Vec<(String, f32)> = by_id
        .iter()
        .filter_map(|candidate| candidate.text_score.map(|score| (candidate.id.clone(), score)))
        .collect();
    text_order.sort_by(|left, right| right.1.partial_cmp(&left.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut ranked: Vec<(f32, RankedHit)> = by_id
        .into_iter()
        .map(|candidate| {
            let mut score = 0.0;
            if let Some(position) = vector_order.iter().position(|(id, _)| id == &candidate.id) {
                score += 1.0 / (RRF_K + position as f32 + 1.0);
            }
            if let Some(position) = text_order.iter().position(|(id, _)| id == &candidate.id) {
                score += 1.0 / (RRF_K + position as f32 + 1.0);
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

    fn candidate(id: &str, distance: Option<f32>, text: Option<f32>, graph: bool) -> Candidate {
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
