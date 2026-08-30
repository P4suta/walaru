//! Bundled AI Agent Skill contract.

use std::fs;
use std::path::Path;

#[test]
fn bundled_skill_uses_only_bounded_structured_cli_results() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let skill = fs::read_to_string(root.join("skills/walaru/SKILL.md")).unwrap();
    let metadata = fs::read_to_string(root.join("skills/walaru/agents/openai.yaml")).unwrap();

    assert!(skill.starts_with("---\nname: walaru\n"));
    for required in [
        "--format json",
        "capabilities",
        "nextActions",
        "argv",
        "--cursor",
        "--max-bytes",
        "exit code `1`",
        "exit code `4`",
    ] {
        assert!(skill.contains(required), "skill is missing `{required}`");
    }
    assert!(!skill.contains("| grep"));
    assert!(!skill.contains("eval "));
    assert!(metadata.contains("default_prompt:"));
    assert!(metadata.contains("$walaru"));
}
