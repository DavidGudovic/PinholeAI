//! Pinhole's local image check (RELEASE-SPEC §4). Small classifiers run on the
//! processor, offline, on every result before it is shown. Scores stay in memory:
//! nothing is logged, counted or written.
//!
//! * [`files`]: the pinned model files (compiled in) and their verification.
//! * [`rules`]: the pure rules over the scores, unit-tested with made-up scores.
//! * [`run`]: loading the models and turning an image into [`rules::Readings`].
pub mod files;
pub mod rules;
pub mod run;

pub use rules::{Face, Original, Readings, Rule, Tags};
pub use run::Checker;

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("{0} is missing")]
    Missing(&'static str),
    #[error("{0} is damaged")]
    Damaged(&'static str),
    #[error("the image couldn't be read")]
    Image,
    #[error("the check couldn't run: {0}")]
    Run(String),
}

impl From<tract_onnx::prelude::TractError> for CheckError {
    fn from(e: tract_onnx::prelude::TractError) -> Self {
        CheckError::Run(e.to_string())
    }
}
