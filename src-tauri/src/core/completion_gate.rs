#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use super::mission_contract::MissionContract;
use super::evidence::EvidenceGraph;
use super::cognitive_state::CognitiveState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionDecision {
    Complete,
    Incomplete(Vec<String>),
    Blocked(Vec<String>),
}

fn normalize_command_str(cmd: &str) -> String {
    cmd.trim()
        .replace('\\', "/")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn paths_match(p1: &str, p2: &str) -> bool {
    let s1 = p1.trim();
    let s2 = p2.trim();
    if s1.is_empty() || s2.is_empty() {
        return false;
    }
    let n1 = s1.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let n2 = s2.replace('\\', "/").trim_end_matches('/').to_lowercase();
    n1 == n2 || n1 == "." || n2 == "."
}

pub struct CompletionGate;

impl CompletionGate {
    pub fn evaluate(
        contract: &MissionContract,
        _state: &CognitiveState,
        evidence: &EvidenceGraph,
        current_world_hash: u64,
        workspace_path: &std::path::Path,
    ) -> CompletionDecision {
        // Guard: empty contract must never silently complete a build/coding mission.
        // A contract with no required criteria AND no required evidence is invalid
        // for any non-trivial mission — block it explicitly.
        let has_any_requirements = !contract.acceptance_criteria.is_empty()
            || !contract.required_evidence.is_empty();
        if !has_any_requirements {
            return CompletionDecision::Incomplete(vec![
                "El contrato de misión no tiene criterios de aceptación ni evidencia requerida. \
                 Agrega criterios antes de declarar completitud.".to_string(),
            ]);
        }

        let mut missing = Vec::new();

        // 1. Evaluate REQUIRED acceptance criteria DYNAMICALLY based on physical workspace inspection & EvidenceGraph.
        // We do NOT trust 'status == Satisfied' set by the LLM.
        for ac in &contract.acceptance_criteria {
            if !ac.required {
                continue;
            }

            let is_satisfied = match &ac.verification {
                crate::core::mission_contract::VerificationMethod::TestPassed => {
                    use crate::core::evidence::StructuredFact;
                    let ws_str = workspace_path.to_string_lossy().to_string();
                    evidence.has_valid_structured_evidence(
                        0.5,
                        current_world_hash,
                        |fact| {
                            match fact {
                                StructuredFact::TestResult { exit_code, cwd, .. } => {
                                    *exit_code == 0 && paths_match(cwd, &ws_str)
                                },
                                StructuredFact::CommandResult { command, cwd, exit_code, .. } => {
                                    let norm = normalize_command_str(command);
                                    *exit_code == 0
                                        && (norm == "cargo test" || norm.starts_with("cargo test ") || norm == "npm test" || norm.starts_with("pytest") || norm.starts_with("python -m unittest") || norm.starts_with("python -m pytest"))
                                        && paths_match(cwd, &ws_str)
                                },
                                _ => false,
                            }
                        }
                    )
                },
                crate::core::mission_contract::VerificationMethod::CommandExitZero(cmd) => {
                    use crate::core::evidence::StructuredFact;
                    let expected_cmd = normalize_command_str(cmd);
                    let ws_str = workspace_path.to_string_lossy().to_string();
                    evidence.has_valid_structured_evidence(
                        0.5,
                        current_world_hash,
                        |fact| {
                            match fact {
                                StructuredFact::CommandResult { command, cwd, exit_code, .. } => {
                                    *exit_code == 0
                                        && normalize_command_str(command) == expected_cmd
                                        && paths_match(cwd, &ws_str)
                                },
                                _ => false
                            }
                        }
                    )
                },
                crate::core::mission_contract::VerificationMethod::FileExistence(file) => {
                    // P0: Physical disk check required! Historical evidence alone is not enough if file was deleted.
                    crate::core::workspace_resolver::WorkspaceResolver::new(workspace_path)
                        .and_then(|r| r.file_exists(file))
                        .unwrap_or(false)
                },
                crate::core::mission_contract::VerificationMethod::ContentMatches { file, regex } => {
                    // P0: Read real file content from disk and match regex!
                    let target_path = crate::core::workspace_resolver::WorkspaceResolver::new(workspace_path)
                        .and_then(|r| r.resolve_exact(file))
                        .ok();
                    if let Some(p) = target_path.filter(|p| p.is_file()) {
                        if let Ok(content) = std::fs::read_to_string(&p) {
                            if let Ok(re) = regex::Regex::new(regex) {
                                re.is_match(&content)
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                },
                crate::core::mission_contract::VerificationMethod::ManualReview => {
                    evidence.has_valid_evidence_for_state(&format!("{} verified manually", ac.id), 1.0, current_world_hash)
                },
            };

            if !is_satisfied {
                missing.push(format!("[{}] {}", ac.id, ac.description));
            }
        }

        // 2. Required evidence – uses STRICT exact match + state_hash validation
        for req in &contract.required_evidence {
            if !evidence.has_valid_evidence_for_state(&req.claim, req.min_reliability, current_world_hash) {
                missing.push(format!(
                    "Evidencia requerida ausente o inválida por cambio de código: '{}' (confiabilidad mínima: {:.0}%)",
                    req.claim, req.min_reliability * 100.0
                ));
            }
        }

        if missing.is_empty() {
            CompletionDecision::Complete
        } else {
            CompletionDecision::Incomplete(missing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mission_contract::{MissionContract, EvidenceRequirement};
    use crate::core::evidence::{EvidenceGraph, EvidenceKind, StructuredFact};
    use std::path::Path;

    #[test]
    fn test_completion_gate_blocks_on_min_reliability() {
        let mut contract = MissionContract::new("Build app");
        contract.required_evidence.push(EvidenceRequirement {
            claim: "cargo test".to_string(),
            min_reliability: 0.8,
        });

        let mut evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build app");
        let hash = 12345;
        let dummy_path = Path::new(".");

        // No evidence at all -> Incomplete
        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));

        let test_fact = crate::core::evidence::StructuredFact::TestResult {
            command: "cargo test".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            passed: 1,
            failed: 0,
            ignored: 0,
        };

        // Evidence with insufficient reliability -> still Incomplete
        evidence.record_structured(EvidenceKind::Test, "cargo", test_fact.clone(), 0.6, 1, Some(hash)).unwrap();
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        // Evidence with sufficient reliability but WRONG hash -> Incomplete
        evidence.record_structured(EvidenceKind::Test, "cargo", test_fact.clone(), 0.9, 2, Some(99999)).unwrap();
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec3, CompletionDecision::Incomplete(_)));

        // Evidence with sufficient reliability AND CORRECT hash -> Complete
        evidence.record_structured(EvidenceKind::Test, "cargo", test_fact, 0.9, 3, Some(hash)).unwrap();
        let dec4 = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert_eq!(dec4, CompletionDecision::Complete);
    }

    #[test]
    fn test_regression_false_satisfied() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion(
            "AC-1",
            "Tests pass",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true
        );
        contract.mark_criterion("AC-1", true);

        let evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build app");
        let hash = 111;

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, Path::new("."));
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));
    }

    #[test]
    fn test_regression_evidence_without_hash_rejected() {
        let mut evidence = EvidenceGraph::new();
        let res = evidence.record(EvidenceKind::Compilation, "tool", "claim", "value", 1.0, 1);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Security Violation"));
    }

    #[test]
    fn test_physical_file_existence_verification() {
        let temp_dir = std::env::temp_dir().join(format!("aura_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut contract = MissionContract::new("Check file");
        contract.add_criterion(
            "AC-FILE",
            "Output file exists",
            crate::core::mission_contract::VerificationMethod::FileExistence("output.txt".to_string()),
            true
        );
        let evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "File check");

        // Test 1: File does not exist -> Incomplete
        let dec1 = CompletionGate::evaluate(&contract, &state, &evidence, 100, &temp_dir);
        assert!(matches!(dec1, CompletionDecision::Incomplete(_)));

        // Test 2: File created physically -> Complete
        let file_path = temp_dir.join("output.txt");
        std::fs::write(&file_path, "hello").unwrap();
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, 100, &temp_dir);
        assert_eq!(dec2, CompletionDecision::Complete);

        // Test 3: File deleted physically -> Incomplete (despite previous existence)
        std::fs::remove_file(&file_path).unwrap();
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence, 100, &temp_dir);
        assert!(matches!(dec3, CompletionDecision::Incomplete(_)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_physical_content_matches_verification() {
        let temp_dir = std::env::temp_dir().join(format!("aura_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut contract = MissionContract::new("Check content");
        contract.add_criterion(
            "AC-CONTENT",
            "JSON contains total_words",
            crate::core::mission_contract::VerificationMethod::ContentMatches {
                file: "stats.json".to_string(),
                regex: r#""total_words"\s*:\s*\d+"#.to_string(),
            },
            true
        );
        let evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Content check");
        let file_path = temp_dir.join("stats.json");

        // Test 4: File contains matching JSON -> Complete
        std::fs::write(&file_path, r#"{"total_words": 42}"#).unwrap();
        let dec1 = CompletionGate::evaluate(&contract, &state, &evidence, 200, &temp_dir);
        assert_eq!(dec1, CompletionDecision::Complete);

        // Test 5: File modified to not match -> Incomplete
        std::fs::write(&file_path, r#"{"error": "failed"}"#).unwrap();
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, 200, &temp_dir);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_command_exit_zero_and_test_passed_verification() {
        let mut contract = MissionContract::new("Build and test");
        contract.add_criterion(
            "AC-BUILD",
            "cargo build passes",
            crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()),
            true
        );
        contract.add_criterion(
            "AC-TEST",
            "cargo test passes",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true
        );

        let mut evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build & Test");
        let current_hash = 999;
        let old_hash = 111;
        let dummy_path = Path::new(".");

        // Test 6 & 7: CommandExitZero with old hash -> Incomplete
        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            stdout_hash: "h".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 1, Some(old_hash)).unwrap();
        let dec1 = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, dummy_path);
        assert!(matches!(dec1, CompletionDecision::Incomplete(_)));

        // Record cargo build with current hash
        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            stdout_hash: "h".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 2, Some(current_hash)).unwrap();
        // Still missing test passed
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, dummy_path);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        // Test 8: TestPassed with current hash -> Complete
        evidence.record_structured(EvidenceKind::Test, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo test".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            stdout_hash: "h".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 3, Some(current_hash)).unwrap();
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, dummy_path);
        assert_eq!(dec3, CompletionDecision::Complete);
    }

    #[test]
    fn test_unstructured_textual_claim_rejected_by_gate() {
        let mut contract = MissionContract::new("Build and test");
        contract.add_criterion(
            "AC-BUILD",
            "cargo build passes",
            crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()),
            true
        );
        contract.add_criterion(
            "AC-TEST",
            "cargo test passes",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true
        );

        let mut evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build & Test");
        let hash = 888;
        let dummy_path = Path::new(".");

        // Fraudulent / textual evidence using Generic for technical kinds is strictly rejected at registration
        assert!(evidence.record_generic_with_hash(EvidenceKind::StaticAnalysis, "TOOL_LLM", "cargo build passes", "0", 1.0, 1, Some(hash)).is_err());
        assert!(evidence.record_generic_with_hash(EvidenceKind::StaticAnalysis, "TOOL_LLM", "cargo test passes", "0", 1.0, 2, Some(hash)).is_err());

        // CompletionGate MUST REJECT completion
        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));
    }

    // ─── AUDIT CLOSURE TESTS: 8 SCENARIOS ─────────────────────────────────────

    #[test]
    fn test_audit_scenario_1_exit_code_none_produces_no_command_exit_code_evidence() {
        use crate::core::mission_runtime::MissionRuntime;
        use crate::core::observation::Observation;

        let mut runtime = MissionRuntime::new(".", "Test none exit code", 10);
        let mut obs = Observation::success("TOOL_TERMINAL", "output without exit code", vec![]);
        obs.exit_code = None; // Explicitly None
        obs.command = Some("cargo build".to_string());
        obs.cwd = Some(".".to_string());
        obs.state_hash_after = Some(100);

        runtime.record_observation(&obs);

        // Verification: No CommandExitCode evidence was recorded!
        let has_command_evidence = runtime.evidence_graph.entries.iter().any(|e| e.kind == EvidenceKind::CommandExitCode);
        assert!(!has_command_evidence, "exit_code=None must not produce CommandExitCode evidence");

        // Contract requiring CommandExitZero cannot complete
        runtime.contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        assert!(matches!(runtime.can_complete(), CompletionDecision::Incomplete(_)));
    }

    #[test]
    fn test_audit_scenario_2_exit_code_zero_is_valid() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        let state = CognitiveState::new("m1", "Build");
        let hash = 42;
        let mut evidence = EvidenceGraph::new();

        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            stdout_hash: "h1".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 1, Some(hash)).unwrap();

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, Path::new("."));
        assert_eq!(dec, CompletionDecision::Complete);
    }

    #[test]
    fn test_audit_scenario_3_exit_code_one_is_invalid() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        let state = CognitiveState::new("m1", "Build");
        let hash = 42;
        let mut evidence = EvidenceGraph::new();

        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build".to_string(),
            cwd: ".".to_string(),
            exit_code: 1, // Non-zero!
            stdout_hash: "h1".to_string(),
            stderr_hash: "error".to_string(),
        }, 1.0, 1, Some(hash)).unwrap();

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, Path::new("."));
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));
    }

    #[test]
    fn test_audit_scenario_4_generic_fact_rejected_for_technical_criteria() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        let state = CognitiveState::new("m1", "Build");
        let hash = 42;
        let mut evidence = EvidenceGraph::new();

        // 1. Technical kinds reject Generic at recording
        assert!(evidence.record_generic_with_hash(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", "cargo build passes", "0", 1.0, 1, Some(hash)).is_err());

        // 2. Even if non-technical kind has Generic, CompletionGate rejects it for technical criteria
        evidence.record_generic_with_hash(EvidenceKind::UserConfirmation, "user", "cargo build passes", "0", 1.0, 1, Some(hash)).unwrap();
        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, Path::new("."));
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));
    }

    #[test]
    fn test_audit_scenario_5_command_identity_exact_match_rejects_subsets() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        let state = CognitiveState::new("m1", "Build");
        let hash = 42;
        let mut evidence = EvidenceGraph::new();

        // Evidence ran "cargo build --release", but criterion requires exact "cargo build"
        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build --release".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            stdout_hash: "h1".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 1, Some(hash)).unwrap();

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, Path::new("."));
        assert!(matches!(dec, CompletionDecision::Incomplete(_)), "Exact command identity required: 'cargo build --release' must not satisfy 'cargo build'");
    }

    #[test]
    fn test_audit_scenario_6_old_world_hash_is_invalid() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        let state = CognitiveState::new("m1", "Build");
        let old_hash = 100;
        let current_hash = 200;
        let mut evidence = EvidenceGraph::new();

        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            stdout_hash: "h1".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 1, Some(old_hash)).unwrap();

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, Path::new("."));
        assert!(matches!(dec, CompletionDecision::Incomplete(_)), "Old world hash evidence must be invalidated");
    }

    #[test]
    fn test_audit_scenario_7_current_world_hash_is_valid() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        let state = CognitiveState::new("m1", "Build");
        let current_hash = 200;
        let mut evidence = EvidenceGraph::new();

        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build".to_string(),
            cwd: ".".to_string(),
            exit_code: 0,
            stdout_hash: "h1".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 1, Some(current_hash)).unwrap();

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, Path::new("."));
        assert_eq!(dec, CompletionDecision::Complete, "Evidence with current world hash must be valid");
    }

    #[test]
    fn test_audit_scenario_8_missing_cwd_is_invalid_for_cwd_requiring_criteria() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion("AC-1", "Build", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        let state = CognitiveState::new("m1", "Build");
        let hash = 42;
        let mut evidence = EvidenceGraph::new();

        // Empty/missing cwd
        evidence.record_structured(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", StructuredFact::CommandResult {
            command: "cargo build".to_string(),
            cwd: "".to_string(), // Missing cwd!
            exit_code: 0,
            stdout_hash: "h1".to_string(),
            stderr_hash: "".to_string(),
        }, 1.0, 1, Some(hash)).unwrap();

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, Path::new("."));
        assert!(matches!(dec, CompletionDecision::Incomplete(_)), "Missing cwd must fail criteria requiring workspace containment");
    }

    #[test]
    fn test_audit_scenario_cargo_toml_missing_incomplete_then_fixed_complete() {
        let temp_dir = std::env::temp_dir().join(format!("aura_e2e_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(temp_dir.join("src"));

        // Step 1: src/main.rs exists, but Cargo.toml is missing
        let main_rs = temp_dir.join("src").join("main.rs");
        std::fs::write(&main_rs, "fn main() { println!(\"hello\"); }").unwrap();

        let mut contract = MissionContract::new("Rust project completion");
        contract.add_criterion(
            "AC-CARGO-TOML",
            "Cargo.toml must exist",
            crate::core::mission_contract::VerificationMethod::FileExistence("Cargo.toml".to_string()),
            true,
        );
        contract.add_criterion(
            "AC-MAIN-RS",
            "src/main.rs must exist",
            crate::core::mission_contract::VerificationMethod::FileExistence("src/main.rs".to_string()),
            true,
        );
        contract.add_criterion(
            "AC-BUILD",
            "cargo build passes",
            crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()),
            true,
        );

        let mut evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m_rust", "Rust build test");
        let hash_1 = 1111;

        // Gate evaluation while Cargo.toml is missing and build hasn't succeeded
        let dec_1 = CompletionGate::evaluate(&contract, &state, &evidence, hash_1, &temp_dir);
        assert!(matches!(dec_1, CompletionDecision::Incomplete(_)));
        if let CompletionDecision::Incomplete(reasons) = dec_1 {
            assert!(reasons.iter().any(|r| r.contains("Cargo.toml")), "Must report missing Cargo.toml");
        }

        // Step 2: Now write Cargo.toml
        let cargo_toml = temp_dir.join("Cargo.toml");
        std::fs::write(&cargo_toml, "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();

        // Even with Cargo.toml physically present, without valid build evidence for current world hash -> Incomplete
        let dec_2 = CompletionGate::evaluate(&contract, &state, &evidence, hash_1, &temp_dir);
        assert!(matches!(dec_2, CompletionDecision::Incomplete(_)));

        // Step 3: Record valid build evidence with exit_code: 0, current world hash, and matching cwd
        let hash_2 = 2222;
        evidence.record_structured(
            EvidenceKind::CommandExitCode,
            "TOOL_TERMINAL",
            StructuredFact::CommandResult {
                command: "cargo build".to_string(),
                cwd: temp_dir.to_string_lossy().to_string(),
                exit_code: 0,
                stdout_hash: "build_ok".to_string(),
                stderr_hash: "".to_string(),
            },
            1.0,
            1,
            Some(hash_2),
        ).unwrap();

        // Now with Cargo.toml present, src/main.rs present, and valid build evidence -> Complete
        let dec_3 = CompletionGate::evaluate(&contract, &state, &evidence, hash_2, &temp_dir);
        assert_eq!(dec_3, CompletionDecision::Complete);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

