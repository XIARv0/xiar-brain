// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
//! The "System 1" contract: a small local classifier (Laya, an Apache-2.0 encoder model) that answers three kinds of
//! structured question about a text in one forward pass, without generating text:
//!
//! * `choice`: a probability for each of several options (which memory layer?);
//! * `noul`: a calibrated yes/no probability (worth storing? an injection attempt? a duplicate?);
//! * `score`: a value on an ordered scale (how important?).
//!
//! The model itself runs in an engine plug-in (ONNX via OpenVINO / CPU); the core only sees this trait.

use crate::LlmError;

pub trait Laya: Send + Sync {
    /// Probabilities over `options`, in order, summing to 1.
    fn choice(&self, text: &str, query: &str, options: &[&str]) -> Result<Vec<f32>, LlmError>;
    /// Probability in [0, 1] that the answer to `query` about `text` is yes.
    fn noul(&self, text: &str, query: &str) -> Result<f32, LlmError>;
    /// A value in [lo, hi].
    fn score(&self, text: &str, query: &str, lo: f32, hi: f32) -> Result<f32, LlmError>;
    fn model(&self) -> &str;
}

/// Checks a `choice` answer: right length, finite, non-negative, normalised to sum 1 (a model that returns raw scores is
/// tolerated; one that returns garbage is an error).
pub fn normalize_choice(probs: Vec<f32>, options: usize) -> Result<Vec<f32>, LlmError> {
    if probs.len() != options {
        return Err(LlmError::Fatal(format!("laya returned {} probabilities for {options} options", probs.len())));
    }
    if probs.iter().any(|p| !p.is_finite() || *p < 0.0) {
        return Err(LlmError::Fatal("laya returned a non-finite or negative probability".into()));
    }
    let sum: f32 = probs.iter().sum();
    if sum <= 0.0 {
        return Err(LlmError::Fatal("laya returned all-zero probabilities".into()));
    }
    Ok(probs.into_iter().map(|p| p / sum).collect())
}

/// Checks a `noul` answer is a probability.
pub fn check_probability(p: f32) -> Result<f32, LlmError> {
    if p.is_finite() && (0.0..=1.0).contains(&p) {
        Ok(p)
    } else {
        Err(LlmError::Fatal(format!("laya returned {p} where a probability was expected")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_answers_are_validated_and_normalised() {
        assert_eq!(normalize_choice(vec![1.0, 3.0], 2).unwrap(), vec![0.25, 0.75]);
        assert!(normalize_choice(vec![1.0], 2).is_err());
        assert!(normalize_choice(vec![f32::NAN, 1.0], 2).is_err());
        assert!(normalize_choice(vec![-0.1, 1.0], 2).is_err());
        assert!(normalize_choice(vec![0.0, 0.0], 2).is_err());
    }

    #[test]
    fn probabilities_must_be_in_range() {
        assert_eq!(check_probability(0.0).unwrap(), 0.0);
        assert_eq!(check_probability(1.0).unwrap(), 1.0);
        for bad in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
            assert!(check_probability(bad).is_err(), "{bad}");
        }
    }
}
