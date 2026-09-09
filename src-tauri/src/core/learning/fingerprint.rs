use serde::{Deserialize, Serialize};
use crate::core::mission_contract::MissionContract;
use crate::core::project_profile::{ProjectProfile, PrimaryLanguage};

/// AL-v1 TaskFingerprint v2 — structural classification of a mission.
/// Deterministic: built from MissionContract + ProjectProfile only.
/// Never calls the LLM. Never fails — always returns a valid fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskFingerprint {
    /// Primary language detected from ProjectProfile (not from objective text)
    pub language: Option<String>,
    /// Primary framework detected from ProjectProfile
    pub framework: Option<String>,
    /// Complexity 0.0..1.0 — deterministic sum of complexity signals / max_signals
    pub complexity: f32,
    /// Ambiguity 0.0..1.0 — heuristic from objective length and word patterns
    pub ambiguity: f32,
    /// Scope bucket: 0=minimal, 1=small, 2=medium, 3=large
    pub scope_bucket: u8,
    pub requires_code: bool,
    pub requires_terminal: bool,
    pub requires_tests: bool,
    pub requires_network: bool,
    /// Verification depth: 0=none, 1=compile, 2=unit tests, 3=integration/UI
    pub verification_level: u8,
}

pub struct FingerprintBuilder;

impl FingerprintBuilder {
    /// Build a deterministic TaskFingerprint from contract + project profile.
    /// Backward-compatible shortcut without explicit WorldState.
    pub fn from_mission(contract: &MissionContract, profile: &ProjectProfile) -> TaskFingerprint {
        Self::from_mission_with_world(contract, profile, None)
    }

    /// Build a deterministic TaskFingerprint incorporating contract and WorldState context.
    /// No LLM calls, no I/O errors.
    pub fn from_mission_with_world(
        contract: &MissionContract,
        profile: &ProjectProfile,
        world_state: Option<&crate::core::world_state::WorldState>,
    ) -> TaskFingerprint {
        let language = Some(language_str(&profile.primary).to_string());
        let framework = profile.frameworks.first().cloned();
        let obj = contract.objective.to_lowercase();

        // Complexity signals (max 8 with world & contract context)
        let mut complexity_signals: u32 = 0;
        if !profile.secondary.is_empty()     { complexity_signals += 1; }
        if !profile.frameworks.is_empty()    { complexity_signals += 1; }
        if profile.has_docker                { complexity_signals += 1; }
        if obj.contains("test") || obj.contains("prueba") { complexity_signals += 1; }
        if obj.contains("integra") || obj.contains("connect") { complexity_signals += 1; }
        if obj.contains("refactor") || obj.contains("restructur") { complexity_signals += 1; }
        // Contract criteria depth
        if contract.acceptance_criteria.len() >= 3 { complexity_signals += 1; }
        // World state workspace size / existing code scale
        if let Some(ws) = world_state {
            if ws.files.len() > 10 {
                complexity_signals += 1;
            }
        }
        let complexity = (complexity_signals as f32 / 8.0).clamp(0.0, 1.0);

        // Ambiguity: long + abstract objective → higher ambiguity
        let word_count = obj.split_whitespace().count();
        let abstract_words = ["improve", "better", "optimize", "enhance",
                               "mejorar", "optimizar", "arreglar", "corregir"];
        let abstract_count = abstract_words.iter().filter(|&&w| obj.contains(w)).count();
        let ambiguity = ((word_count as f32 / 30.0).min(0.6)
            + (abstract_count as f32 / 4.0).min(0.4)).clamp(0.0, 1.0);

        // Scope bucket — estimated from complexity and architecture signals
        let scope_bucket = match complexity_signals {
            0..=1 => 0,
            2..=3 => 1,
            4..=5 => 2,
            _ => 3,
        };

        let requires_code     = obj.contains("creat") || obj.contains("implement")
            || obj.contains("escrib") || obj.contains("add") || obj.contains("añad")
            || !contract.acceptance_criteria.is_empty();
        let requires_terminal = obj.contains("ejecut") || obj.contains("run")
            || obj.contains("compil") || obj.contains("build") || obj.contains("instala")
            || contract.acceptance_criteria.iter().any(|c| matches!(c.verification, crate::core::mission_contract::VerificationMethod::CommandExitZero(_)));
        let requires_tests    = obj.contains("test") || obj.contains("prueba")
            || obj.contains("verif") || obj.contains("assert")
            || contract.acceptance_criteria.iter().any(|c| matches!(c.verification, crate::core::mission_contract::VerificationMethod::TestPassed));
        let requires_network  = obj.contains("http") || obj.contains("api")
            || obj.contains("fetch") || obj.contains("request") || obj.contains("endpoint");

        let verification_level = if requires_tests && obj.contains("integr") { 3 }
            else if requires_tests { 2 }
            else if requires_terminal { 1 }
            else { 0 };

        TaskFingerprint {
            language, framework, complexity, ambiguity, scope_bucket,
            requires_code, requires_terminal, requires_tests, requires_network,
            verification_level,
        }
    }

    /// Structural similarity 0.0..1.0 — compares dimensions, not text.
    pub fn similarity(a: &TaskFingerprint, b: &TaskFingerprint) -> f32 {
        let mut score = 0.0f32;

        // Language match (weight 0.30)
        if a.language == b.language { score += 0.30; }

        // Framework match (weight 0.10)
        if a.framework == b.framework { score += 0.10; }

        // Complexity proximity (weight 0.20)
        score += 0.20 * (1.0 - (a.complexity - b.complexity).abs());

        // Boolean flags (weight 0.08 each × 4 = 0.32)
        let booleans = [
            (a.requires_code,     b.requires_code),
            (a.requires_terminal, b.requires_terminal),
            (a.requires_tests,    b.requires_tests),
            (a.requires_network,  b.requires_network),
        ];
        for (x, y) in booleans {
            if x == y { score += 0.08; }
        }

        // Verification level match (weight 0.08)
        if a.verification_level == b.verification_level { score += 0.08; }

        score.clamp(0.0, 1.0)
    }
}

fn language_str(lang: &PrimaryLanguage) -> &'static str {
    match lang {
        PrimaryLanguage::Rust       => "rust",
        PrimaryLanguage::Python     => "python",
        PrimaryLanguage::JavaScript => "javascript",
        PrimaryLanguage::TypeScript => "typescript",
        PrimaryLanguage::Go         => "go",
        PrimaryLanguage::Unknown    => "unknown",
        _ => "unknown",
    }
}
