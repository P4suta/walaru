//! Local source and release-distribution contract.

use std::fs;
use std::path::Path;

#[test]
fn repository_documents_and_packages_the_supported_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    for required in [
        "README.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "docs/architecture.md",
        "docs/contracts.md",
        "docs/library-api.md",
        "docs/replay.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "CODE_OF_CONDUCT.md",
        "scripts/check.sh",
        "scripts/lint-repository.sh",
        "scripts/package-linux.sh",
        "scripts/package-macos.sh",
        "scripts/package-windows.ps1",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        ".github/dependabot.yml",
        "clients/vscode/package.json",
        "clients/vscode/client.js",
        "clients/vscode/extension.js",
        "clients/vscode/test/client.test.js",
        "clients/intellij/walaru-external-tools.xml",
        "clients/intellij/README.md",
        "examples/java-library-first/build.gradle.kts",
        "examples/java-library-first/src/main/java/example/BinarySearch.java",
        "examples/kotlin-library-first/build.gradle.kts",
        "examples/kotlin-library-first/src/main/kotlin/example/Statistics.kt",
    ] {
        assert!(root.join(required).is_file(), "missing {required}");
    }

    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    for required in [
        "JDK 21",
        "zero-dependency API",
        "walaruExplain",
        "explain",
        "verified: true",
    ] {
        assert!(readme.contains(required), "README is missing `{required}`");
    }
    let check = fs::read_to_string(root.join("scripts/check.sh")).unwrap();
    assert!(check.contains("cargo test --workspace"));
    assert!(check.contains("./gradlew check"));
    assert!(check.contains("generatePomFileForLibraryPublication"));
    assert!(check.contains("node --test clients/vscode/test/*.test.js"));
    assert!(!check.contains("/home/"), "check script must be portable");
    let package = fs::read_to_string(root.join("scripts/package-linux.sh")).unwrap();
    assert!(package.contains("skills/walaru"));
    assert!(package.contains("sha256sum"));
    assert!(package.contains("workspace_version"));
    for library in ["walaru-api.jar", "walaru-client.jar", "walaru-testkit.jar"] {
        assert!(package.contains(library), "package is missing {library}");
    }
    assert!(!package.contains("jvm-agent-0.1.0"));
    let windows_package = fs::read_to_string(root.join("scripts/package-windows.ps1")).unwrap();
    assert!(windows_package.starts_with("#Requires -Version 7.0"));
    let contributing = fs::read_to_string(root.join("CONTRIBUTING.md")).unwrap();
    assert!(contributing.contains("PowerShell 7"));

    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    for required in ["ubuntu-24.04", "macos-15", "windows-2025", "21", "25"] {
        assert!(workflow.contains(required), "CI is missing `{required}`");
    }
    assert!(
        workflow.contains("gradle/actions/wrapper-validation@"),
        "CI must validate committed Gradle wrapper binaries"
    );

    let security = fs::read_to_string(root.join("SECURITY.md")).unwrap();
    assert!(
        security.contains("https://github.com/P4suta/walaru/security/advisories/new"),
        "security policy must link directly to private vulnerability reporting"
    );

    let vscode = fs::read_to_string(root.join("clients/vscode/client.js")).unwrap();
    assert!(vscode.contains("--format"));
    assert!(vscode.contains("--max-bytes"));
    assert!(vscode.contains("shell: false"));
    assert!(!vscode.contains("exec("));

    let manifest = fs::read_to_string(root.join("clients/vscode/package.json")).unwrap();
    assert!(manifest.contains("walaru.refreshIntervalSeconds"));
    assert!(manifest.contains("\"scope\": \"resource\""));

    let intellij =
        fs::read_to_string(root.join("clients/intellij/walaru-external-tools.xml")).unwrap();
    assert!(intellij.contains("$ProjectFileDir$"));
    assert!(intellij.contains("--format json"));
}

#[test]
fn dependency_fixtures_follow_the_version_catalog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let catalog = fs::read_to_string(root.join("gradle/libs.versions.toml")).unwrap();
    let testng = catalog_version(&catalog, "testng");
    let coroutines = catalog_version(&catalog, "coroutines");

    let testng_functional = fs::read_to_string(root.join(
        "gradle-adapter/src/test/kotlin/io/github/p4suta/walaru/gradle/TestNgFunctionalTest.kt",
    ))
    .unwrap();
    assert!(
        testng_functional.contains(&format!("org.testng:testng:{testng}")),
        "Gradle TestNG fixture does not use catalog version {testng}"
    );
    let testng_maven = fs::read_to_string(root.join("fixtures/maven-testng/pom.xml")).unwrap();
    assert!(
        testng_maven.contains(&format!("<version>{testng}</version>")),
        "Maven TestNG fixture does not use catalog version {testng}"
    );

    for fixture in [
        "gradle-adapter/src/test/kotlin/io/github/p4suta/walaru/gradle/MixedJvmFunctionalTest.kt",
        "fixtures/mixed-gradle/build.gradle.kts",
    ] {
        let contents = fs::read_to_string(root.join(fixture)).unwrap();
        assert!(
            contents.contains(&format!("kotlinx-coroutines-core:{coroutines}")),
            "{fixture} does not use catalog version {coroutines}"
        );
    }
}

fn catalog_version(catalog: &str, name: &str) -> String {
    let prefix = format!("{name} = \"");
    catalog
        .lines()
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix('"'))
        })
        .unwrap_or_else(|| panic!("version catalog is missing {name}"))
        .to_owned()
}
