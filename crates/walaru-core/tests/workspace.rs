//! Content revision and worktree layout contract.

use std::fs;

use tempfile::tempdir;
use walaru_core::workspace::{RevisionSnapshot, WorkspaceLayout};

#[test]
fn revision_tracks_source_resource_and_build_content_but_not_generated_state() {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/kotlin/demo")).unwrap();
    fs::create_dir_all(directory.path().join("src/main/resources")).unwrap();
    fs::write(
        directory.path().join("src/main/kotlin/demo/Example.kt"),
        "package demo\nfun answer() = 42\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("src/main/resources/app.conf"),
        "mode=test\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("build.gradle.kts"),
        "plugins { java }\n",
    )
    .unwrap();

    let original = RevisionSnapshot::capture(directory.path()).unwrap();
    fs::create_dir_all(directory.path().join(".gradle/walaru/ws-ignore")).unwrap();
    fs::write(
        directory
            .path()
            .join(".gradle/walaru/ws-ignore/store.sqlite3"),
        "generated",
    )
    .unwrap();
    fs::create_dir_all(directory.path().join("build/classes")).unwrap();
    fs::write(
        directory.path().join("build/classes/Example.class"),
        "bytecode",
    )
    .unwrap();
    let generated_change = RevisionSnapshot::capture(directory.path()).unwrap();
    assert_eq!(original.revision, generated_change.revision);

    fs::write(
        directory.path().join("src/main/resources/app.conf"),
        "mode=production\n",
    )
    .unwrap();
    let resource_change = RevisionSnapshot::capture(directory.path()).unwrap();
    assert_ne!(original.revision, resource_change.revision);
}

#[test]
fn revision_tracks_maven_reactor_and_wrapper_inputs() {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("module/src/main/java/demo")).unwrap();
    fs::create_dir_all(directory.path().join(".mvn/wrapper")).unwrap();
    fs::write(
        directory.path().join("pom.xml"),
        "<project><modules><module>module</module></modules></project>",
    )
    .unwrap();
    fs::write(
        directory.path().join("module/pom.xml"),
        "<project><artifactId>module</artifactId></project>",
    )
    .unwrap();
    fs::write(
        directory
            .path()
            .join(".mvn/wrapper/maven-wrapper.properties"),
        "distributionUrl=https://example.invalid/maven.zip\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("module/src/main/java/demo/App.java"),
        "package demo; class App {}",
    )
    .unwrap();

    let original = RevisionSnapshot::capture(directory.path()).unwrap();
    assert!(original.files.contains(&"pom.xml".into()));
    assert!(original.files.contains(&"module/pom.xml".into()));
    assert!(
        original
            .files
            .contains(&".mvn/wrapper/maven-wrapper.properties".into())
    );

    fs::write(
        directory.path().join("module/pom.xml"),
        "<project><artifactId>renamed</artifactId></project>",
    )
    .unwrap();
    let changed = RevisionSnapshot::capture(directory.path()).unwrap();
    assert_ne!(original.revision, changed.revision);
}

#[test]
fn revision_is_independent_of_file_creation_order() {
    let left = tempdir().unwrap();
    let right = tempdir().unwrap();
    for (root, files) in [
        (
            left.path(),
            [
                ("settings.gradle.kts", "rootProject.name=\"x\""),
                ("A.kt", "class A"),
            ],
        ),
        (
            right.path(),
            [
                ("A.kt", "class A"),
                ("settings.gradle.kts", "rootProject.name=\"x\""),
            ],
        ),
    ] {
        for (path, contents) in files {
            fs::write(root.join(path), contents).unwrap();
        }
    }
    assert_eq!(
        RevisionSnapshot::capture(left.path()).unwrap().revision,
        RevisionSnapshot::capture(right.path()).unwrap().revision,
    );
}

#[test]
fn source_surface_fingerprint_ignores_bodies_but_tracks_signatures_and_globals() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("Example.kt");
    fs::write(
        &source,
        "package demo\nfun answer(value: Int): Int {\n    return value + 1\n}\nval bootMode = \"safe\"\n",
    )
    .unwrap();
    let original = RevisionSnapshot::capture(directory.path()).unwrap();

    fs::write(
        &source,
        "package demo\nfun answer(value: Int): Int {\n    return value + 2\n}\nval bootMode = \"safe\"\n",
    )
    .unwrap();
    let body_change = RevisionSnapshot::capture(directory.path()).unwrap();
    assert_ne!(original.revision, body_change.revision);
    assert_eq!(
        original.surface_digests["Example.kt"],
        body_change.surface_digests["Example.kt"]
    );

    fs::write(
        &source,
        "package demo\nfun answer(value: Long): Int {\n    return value.toInt() + 2\n}\nval bootMode = \"safe\"\n",
    )
    .unwrap();
    let signature_change = RevisionSnapshot::capture(directory.path()).unwrap();
    assert_ne!(
        body_change.surface_digests["Example.kt"],
        signature_change.surface_digests["Example.kt"]
    );

    fs::write(
        &source,
        "package demo\nfun answer(value: Long): Int {\n    return value.toInt() + 2\n}\nval bootMode = \"unsafe\"\n",
    )
    .unwrap();
    let global_change = RevisionSnapshot::capture(directory.path()).unwrap();
    assert_ne!(
        signature_change.surface_digests["Example.kt"],
        global_change.surface_digests["Example.kt"]
    );
}

#[test]
fn state_is_scoped_to_the_worktree_id_under_gradle() {
    let directory = tempdir().unwrap();
    let layout = WorkspaceLayout::new(directory.path()).unwrap();
    assert_eq!(
        layout.state_dir,
        directory
            .path()
            .join(".gradle/walaru")
            .join(layout.workspace_id.as_str())
    );
    assert_eq!(layout.socket, layout.state_dir.join("daemon.sock"));
    assert_eq!(layout.database, layout.state_dir.join("store.sqlite3"));
}
