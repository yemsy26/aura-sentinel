use serde::{Deserialize, Serialize};
use crate::core::learning::fingerprint::TaskFingerprint;

/// Controlled set of mission execution strategies.
/// These are behavioral configurations — they do NOT execute tools directly.
/// AdaptiveRouter selects one; Runtime v4 has authority over execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum StrategyKind {
    /// Default for simple, well-defined tasks
    DirectImplementation,
    /// Inspect the codebase first, then implement (good for unfamiliar projects)
    InspectThenImplement,
    /// Write tests before implementation (good for well-defined requirements)
    TestFirst,
    /// Verify compile state before making changes (good for incremental fixes)
    CompileFirst,
    /// Small iterative changes with frequent verification
    IncrementalPatch,
    /// Diagnose root cause before attempting repair (good for bugs)
    DiagnoseThenRepair,
    /// Minimum possible change (good for targeted, risky modifications)
    MinimalChange,
}

impl StrategyKind {
    /// Default strategy based on fingerprint dimensions — no learning required.
    /// Used for cold-start and as fallback.
    pub fn default_for(fp: &TaskFingerprint) -> Self {
        match (fp.requires_tests, fp.complexity > 0.6, fp.verification_level) {
            (true, true, _)  => Self::InspectThenImplement,
            (true, false, _) => Self::CompileFirst,
            (false, true, 2) => Self::DiagnoseThenRepair,
            (false, true, _) => Self::IncrementalPatch,
            _                => Self::DirectImplementation,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DirectImplementation  => "DirectImplementation",
            Self::InspectThenImplement  => "InspectThenImplement",
            Self::TestFirst             => "TestFirst",
            Self::CompileFirst          => "CompileFirst",
            Self::IncrementalPatch      => "IncrementalPatch",
            Self::DiagnoseThenRepair    => "DiagnoseThenRepair",
            Self::MinimalChange         => "MinimalChange",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "InspectThenImplement" => Self::InspectThenImplement,
            "TestFirst"            => Self::TestFirst,
            "CompileFirst"         => Self::CompileFirst,
            "IncrementalPatch"     => Self::IncrementalPatch,
            "DiagnoseThenRepair"   => Self::DiagnoseThenRepair,
            "MinimalChange"        => Self::MinimalChange,
            _                      => Self::DirectImplementation,
        }
    }
}
