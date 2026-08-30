//! Bounded editor overlays and isolated execution workspaces.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, FileTimes};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use walaru_core::workspace::WorkspaceLayout;

/// Maximum number of dirty documents accepted in one live verification.
pub const MAX_OVERLAY_DOCUMENTS: usize = 256;
/// Maximum UTF-8 bytes accepted for one dirty document.
pub const MAX_OVERLAY_DOCUMENT_BYTES: usize = 1024 * 1024;
/// Maximum aggregate UTF-8 bytes accepted for one live verification.
pub const MAX_OVERLAY_BYTES: usize = 4 * 1024 * 1024;
const MAX_MIRROR_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// Versioned manifest written by an editor client.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayManifest {
    /// Manifest schema; currently `1`.
    pub schema_version: u32,
    /// Stable, bounded editor identity used to retain an incremental shadow build.
    pub session_id: String,
    /// Unsaved text documents relative to the workspace root.
    pub documents: Vec<OverlayDocument>,
}

/// One unsaved UTF-8 document.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayDocument {
    /// Workspace-relative path using `/` separators.
    pub path: String,
    /// Monotonic editor document version, useful for client-side result matching.
    pub version: i64,
    /// Complete unsaved document contents.
    pub content: String,
}

/// Validated overlay carried by a verification request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverlayRequest {
    /// Stable editor identity.
    pub session_id: String,
    /// Dirty documents to place over the disk snapshot.
    pub documents: Vec<OverlayDocument>,
}

impl OverlayManifest {
    /// Validates bounds and converts the public manifest into a request payload.
    pub fn into_request(self) -> Result<OverlayRequest, OverlayError> {
        if self.schema_version != 1 {
            return Err(OverlayError::Schema(self.schema_version));
        }
        let request = OverlayRequest {
            session_id: self.session_id,
            documents: self.documents,
        };
        request.validate()?;
        Ok(request)
    }
}

impl OverlayRequest {
    /// Enforces containment, uniqueness, and fixed payload bounds.
    pub fn validate(&self) -> Result<(), OverlayError> {
        if self.session_id.is_empty()
            || self.session_id.len() > 64
            || !self
                .session_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(OverlayError::Session);
        }
        if self.documents.len() > MAX_OVERLAY_DOCUMENTS {
            return Err(OverlayError::TooManyDocuments(self.documents.len()));
        }
        let mut paths = BTreeSet::new();
        let mut total = 0_usize;
        for document in &self.documents {
            validate_relative_path(&document.path)?;
            if paths.iter().any(|existing: &&str| {
                document
                    .path
                    .strip_prefix(*existing)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                    || existing
                        .strip_prefix(&document.path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }) {
                return Err(OverlayError::PathConflict(document.path.clone()));
            }
            if !paths.insert(document.path.as_str()) {
                return Err(OverlayError::DuplicatePath(document.path.clone()));
            }
            let bytes = document.content.len();
            if bytes > MAX_OVERLAY_DOCUMENT_BYTES {
                return Err(OverlayError::DocumentTooLarge {
                    path: document.path.clone(),
                    bytes,
                });
            }
            total = total.saturating_add(bytes);
            if total > MAX_OVERLAY_BYTES {
                return Err(OverlayError::PayloadTooLarge(total));
            }
        }
        Ok(())
    }
}

/// Prepared shadow root that preserves generated build state between live runs.
#[derive(Debug)]
pub struct OverlayWorkspace {
    root: PathBuf,
}

impl OverlayWorkspace {
    /// Synchronizes user-controlled workspace inputs and applies unsaved documents.
    #[cfg(test)]
    pub fn prepare(
        layout: &WorkspaceLayout,
        overlay: &OverlayRequest,
    ) -> Result<Self, OverlayError> {
        Self::prepare_cancellable(layout, overlay, &|| false)
    }

    pub(crate) fn prepare_cancellable(
        layout: &WorkspaceLayout,
        overlay: &OverlayRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, OverlayError> {
        overlay.validate()?;
        check_cancellation(cancelled)?;
        let session_root = layout.state_dir.join("live").join(&overlay.session_id);
        let repository = repository_root(&layout.root);
        let source_root = repository.clone().unwrap_or_else(|| layout.root.clone());
        let workspace_relative = layout
            .root
            .strip_prefix(&source_root)
            .expect("repository root contains the workspace");
        let mirror_root = session_root.join("mirror");
        let root = mirror_root.join(workspace_relative);
        let manifest_path = session_root.join("mirrored-files.json");
        let preparing_path = session_root.join("mirror-preparing");
        fs::create_dir_all(&session_root)?;
        let interrupted = preparing_path.exists();
        let mut previous = (!interrupted)
            .then(|| read_mirror_manifest(&manifest_path))
            .transpose()?
            .flatten();
        if previous.is_none() && mirror_root.exists() {
            remove_destination(&mirror_root)?;
        }
        if previous.is_none() {
            remove_destination(&manifest_path)?;
            previous = Some(MirrorManifest::new());
        }
        remove_destination(&preparing_path)?;
        File::create(&preparing_path)?.sync_all()?;
        fs::create_dir_all(&root)?;
        let previous = previous.expect("missing mirror metadata was initialized");
        let mut current = MirrorManifest::new();
        if repository.is_some() {
            mirror_repository(
                &source_root,
                &mirror_root,
                &previous,
                &mut current,
                cancelled,
            )?;
        } else {
            mirror_directory(
                &source_root,
                &source_root,
                &mirror_root,
                &previous,
                &mut current,
                cancelled,
            )?;
        }
        for document in &overlay.documents {
            check_cancellation(cancelled)?;
            let relative = workspace_relative.join(path_from_slashes(&document.path));
            let relative = relative.to_string_lossy().replace('\\', "/");
            current.paths.insert(relative.clone());
            current.overlay_paths.insert(relative);
        }
        for removed in previous.paths.difference(&current.paths) {
            check_cancellation(cancelled)?;
            let target = mirror_root.join(removed);
            if target.is_file() || target.is_symlink() {
                fs::remove_file(target)?;
            }
        }
        for document in &overlay.documents {
            check_cancellation(cancelled)?;
            let target = root.join(path_from_slashes(&document.path));
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            remove_destination(&target)?;
            fs::write(target, document.content.as_bytes())?;
        }
        check_cancellation(cancelled)?;
        write_mirror_manifest(&session_root, &manifest_path, &current)?;
        fs::remove_file(&preparing_path)?;
        Ok(Self { root })
    }

    /// Returns the isolated build root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Invalid or unsafe editor overlay.
#[derive(Debug, Error)]
pub enum OverlayError {
    /// Unsupported manifest schema.
    #[error("unsupported overlay schema {0}; expected 1")]
    Schema(u32),
    /// Unsafe session identifier.
    #[error("overlay sessionId must contain 1-64 ASCII letters, digits, '-' or '_'")]
    Session,
    /// Too many dirty documents.
    #[error("overlay contains {0} documents; maximum is {MAX_OVERLAY_DOCUMENTS}")]
    TooManyDocuments(usize),
    /// Unsafe or non-canonical relative path.
    #[error("overlay path `{0}` must be a canonical workspace-relative '/' path")]
    Path(String),
    /// A path appeared more than once.
    #[error("overlay path `{0}` appears more than once")]
    DuplicatePath(String),
    /// Two document paths require the same location to be both a file and a directory.
    #[error("overlay path `{0}` conflicts with another document path")]
    PathConflict(String),
    /// One document exceeded its bound.
    #[error("overlay document `{path}` is {bytes} bytes; maximum is {MAX_OVERLAY_DOCUMENT_BYTES}")]
    DocumentTooLarge {
        /// Workspace-relative path.
        path: String,
        /// Observed UTF-8 byte length.
        bytes: usize,
    },
    /// Aggregate content exceeded its bound.
    #[error("overlay payload is at least {0} bytes; maximum is {MAX_OVERLAY_BYTES}")]
    PayloadTooLarge(usize),
    /// Mirroring failed.
    #[error("overlay workspace I/O error: {0}")]
    Io(#[from] io::Error),
    /// Private mirror metadata was invalid.
    #[error("overlay workspace metadata is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// A newer editor generation superseded mirror preparation.
    #[error("overlay workspace preparation was cancelled")]
    Cancelled,
}

fn validate_relative_path(value: &str) -> Result<(), OverlayError> {
    if value.is_empty()
        || value.len() > 4_096
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value.split('/').any(str::is_empty)
    {
        return Err(OverlayError::Path(value.into()));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(OverlayError::Path(value.into()));
    }
    Ok(())
}

fn mirror_directory(
    source_root: &Path,
    source: &Path,
    destination_root: &Path,
    previous: &MirrorManifest,
    current: &mut MirrorManifest,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OverlayError> {
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        check_cancellation(cancelled)?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let source_path = entry.path();
        let relative = source_path
            .strip_prefix(source_root)
            .expect("walked path stays below its root");
        if file_type.is_dir() {
            if !ignored_directory(&entry.file_name().to_string_lossy()) {
                ensure_directory(&destination_root.join(relative))?;
                mirror_directory(
                    source_root,
                    &source_path,
                    destination_root,
                    previous,
                    current,
                    cancelled,
                )?;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative_string = relative.to_string_lossy().replace('\\', "/");
        let destination = destination_root.join(relative);
        mirror_file(
            &source_path,
            &destination,
            relative_string,
            previous,
            current,
            cancelled,
        )?;
    }
    Ok(())
}

fn ignored_directory(name: &str) -> bool {
    if name.starts_with(".gradle") {
        return true;
    }
    matches!(
        name,
        ".git" | ".idea" | ".kotlin" | "build" | "dist" | "out" | "target" | "node_modules"
    )
}

fn mirror_repository(
    source_root: &Path,
    destination_root: &Path,
    previous: &MirrorManifest,
    current: &mut MirrorManifest,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OverlayError> {
    check_cancellation(cancelled)?;
    let listed = Command::new("git")
        .args([
            "-C",
            source_root.to_string_lossy().as_ref(),
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()?;
    check_cancellation(cancelled)?;
    if !listed.status.success() {
        return Err(OverlayError::Io(io::Error::other(
            "git could not enumerate the overlay mirror",
        )));
    }
    for bytes in listed.stdout.split(|byte| *byte == 0) {
        check_cancellation(cancelled)?;
        if bytes.is_empty() {
            continue;
        }
        let relative = String::from_utf8_lossy(bytes).replace('\\', "/");
        validate_relative_path(&relative)?;
        let source = source_root.join(path_from_slashes(&relative));
        let Ok(file_type) = source
            .symlink_metadata()
            .map(|metadata| metadata.file_type())
        else {
            continue;
        };
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let destination = destination_root.join(path_from_slashes(&relative));
        mirror_file(
            &source,
            &destination,
            relative,
            previous,
            current,
            cancelled,
        )?;
    }
    Ok(())
}

fn repository_root(workspace: &Path) -> Option<PathBuf> {
    let candidate = workspace
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)?;
    if candidate == workspace {
        return Some(candidate);
    }
    let relative = workspace.strip_prefix(&candidate).ok()?;
    let tracked = Command::new("git")
        .args([
            "-C",
            candidate.to_string_lossy().as_ref(),
            "ls-files",
            "-z",
            "--",
        ])
        .arg(relative)
        .output()
        .ok()?;
    (tracked.status.success() && !tracked.stdout.is_empty()).then_some(candidate)
}

fn mirror_file(
    source: &Path,
    destination: &Path,
    relative: String,
    previous: &MirrorManifest,
    current: &mut MirrorManifest,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OverlayError> {
    check_cancellation(cancelled)?;
    let digest = file_digest(source, cancelled)?;
    let reusable = previous.digests.get(&relative) == Some(&digest)
        && !previous.overlay_paths.contains(&relative)
        && destination_matches(source, destination, &digest, cancelled)?;
    if !reusable {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        copy_input(source, destination, cancelled)?;
    }
    current.paths.insert(relative.clone());
    current.digests.insert(relative, digest);
    Ok(())
}

fn destination_matches(
    source: &Path,
    destination: &Path,
    source_digest: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<bool, OverlayError> {
    let source_metadata = source.metadata()?;
    let Ok(destination_metadata) = destination.symlink_metadata() else {
        return Ok(false);
    };
    if !destination_metadata.file_type().is_file() || destination_metadata.file_type().is_symlink()
    {
        return Ok(false);
    }
    if source_metadata.len() != destination_metadata.len() {
        return Ok(false);
    }
    if matches!(
        (source_metadata.modified(), destination_metadata.modified()),
        (Ok(source_modified), Ok(destination_modified)) if source_modified == destination_modified
    ) {
        return Ok(true);
    }
    Ok(file_digest(destination, cancelled)? == source_digest)
}

fn file_digest(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<String, OverlayError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        check_cancellation(cancelled)?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn copy_input(
    source: &Path,
    destination: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OverlayError> {
    let source_metadata = source.metadata()?;
    remove_destination(destination)?;
    let mut input = File::open(source)?;
    let mut output = File::create(destination)?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        check_cancellation(cancelled)?;
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
    }
    output.sync_all()?;
    fs::set_permissions(destination, source_metadata.permissions())?;
    if let (Ok(destination_file), Ok(modified)) = (
        File::options().write(true).open(destination),
        source_metadata.modified(),
    ) {
        // Matching mtimes make an unchanged destination a metadata-only check. Some
        // read-only inputs do not permit this optimization, so `destination_matches`
        // falls back to its content digest.
        let _ = destination_file.set_times(FileTimes::new().set_modified(modified));
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => {
            remove_destination(path)?;
            fs::create_dir_all(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path),
        Err(error) => Err(error),
    }
}

fn remove_destination(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MirrorManifest {
    schema_version: u32,
    paths: BTreeSet<String>,
    digests: BTreeMap<String, String>,
    overlay_paths: BTreeSet<String>,
}

impl MirrorManifest {
    fn new() -> Self {
        Self {
            schema_version: 1,
            paths: BTreeSet::new(),
            digests: BTreeMap::new(),
            overlay_paths: BTreeSet::new(),
        }
    }
}

fn read_mirror_manifest(path: &Path) -> Result<Option<MirrorManifest>, OverlayError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_MIRROR_MANIFEST_BYTES {
        return Ok(None);
    }
    let contents = fs::read(path)?;
    if let Ok(manifest) = serde_json::from_slice::<MirrorManifest>(&contents)
        && manifest.schema_version == 1
    {
        return Ok(Some(manifest));
    }
    Ok(serde_json::from_slice::<BTreeSet<String>>(&contents)
        .ok()
        .map(|paths| MirrorManifest {
            paths,
            ..MirrorManifest::new()
        }))
}

fn write_mirror_manifest(
    session_root: &Path,
    destination: &Path,
    manifest: &MirrorManifest,
) -> Result<(), OverlayError> {
    let encoded = serde_json::to_vec(manifest)?;
    if encoded.len() as u64 > MAX_MIRROR_MANIFEST_BYTES {
        return Err(OverlayError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "overlay mirror metadata exceeds its 64 MiB safety limit",
        )));
    }
    let temporary = session_root.join("mirrored-files.json.tmp");
    remove_destination(&temporary)?;
    let mut file = File::create(&temporary)?;
    file.write_all(&encoded)?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    remove_destination(destination)?;
    fs::rename(temporary, destination)?;
    Ok(())
}

fn check_cancellation(cancelled: &dyn Fn() -> bool) -> Result<(), OverlayError> {
    if cancelled() {
        Err(OverlayError::Cancelled)
    } else {
        Ok(())
    }
}

fn path_from_slashes(value: &str) -> PathBuf {
    value.split('/').collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use tempfile::tempdir;

    fn request(documents: Vec<OverlayDocument>) -> OverlayRequest {
        OverlayRequest {
            session_id: "vscode".into(),
            documents,
        }
    }

    #[test]
    fn rejects_escaping_duplicate_and_oversized_overlays() {
        let escaping = request(vec![OverlayDocument {
            path: "../outside.java".into(),
            version: 1,
            content: String::new(),
        }]);
        assert!(matches!(escaping.validate(), Err(OverlayError::Path(_))));

        let non_canonical = request(vec![OverlayDocument {
            path: "src//outside.java".into(),
            version: 1,
            content: String::new(),
        }]);
        assert!(matches!(
            non_canonical.validate(),
            Err(OverlayError::Path(_))
        ));

        let duplicate = request(vec![
            OverlayDocument {
                path: "src/A.java".into(),
                version: 1,
                content: String::new(),
            },
            OverlayDocument {
                path: "src/A.java".into(),
                version: 2,
                content: String::new(),
            },
        ]);
        assert!(matches!(
            duplicate.validate(),
            Err(OverlayError::DuplicatePath(_))
        ));

        let conflicting = request(vec![
            OverlayDocument {
                path: "src/A.java".into(),
                version: 1,
                content: String::new(),
            },
            OverlayDocument {
                path: "src/A.java/Child.java".into(),
                version: 1,
                content: String::new(),
            },
        ]);
        assert!(matches!(
            conflicting.validate(),
            Err(OverlayError::PathConflict(_))
        ));

        let oversized = request(vec![OverlayDocument {
            path: "src/A.java".into(),
            version: 1,
            content: "x".repeat(MAX_OVERLAY_DOCUMENT_BYTES + 1),
        }]);
        assert!(matches!(
            oversized.validate(),
            Err(OverlayError::DocumentTooLarge { .. })
        ));
    }

    #[test]
    fn mirrors_disk_inputs_preserves_build_state_and_applies_unsaved_text() {
        let directory = tempdir().unwrap();
        fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
        fs::create_dir_all(directory.path().join("build/classes")).unwrap();
        fs::write(
            directory.path().join("settings.gradle.kts"),
            "rootProject.name=\"demo\"",
        )
        .unwrap();
        fs::write(
            directory.path().join("src/main/java/demo/App.java"),
            "class App { int value = 1; }",
        )
        .unwrap();
        fs::write(
            directory.path().join("build/classes/ignored.class"),
            "generated",
        )
        .unwrap();
        let layout = WorkspaceLayout::new(directory.path()).unwrap();
        layout.ensure_state_dir().unwrap();
        let overlay = request(vec![
            OverlayDocument {
                path: "src/main/java/demo/App.java".into(),
                version: 7,
                content: "class App { int value = 2; }".into(),
            },
            OverlayDocument {
                path: "src/main/java/demo/New.java".into(),
                version: 1,
                content: "class New {}".into(),
            },
        ]);

        let workspace = OverlayWorkspace::prepare(&layout, &overlay).unwrap();

        assert_eq!(
            fs::read_to_string(workspace.root().join("src/main/java/demo/App.java")).unwrap(),
            "class App { int value = 2; }"
        );
        assert!(workspace.root().join("settings.gradle.kts").is_file());
        assert!(
            workspace
                .root()
                .join("src/main/java/demo/New.java")
                .is_file()
        );
        assert!(
            !workspace
                .root()
                .join("build/classes/ignored.class")
                .exists()
        );

        fs::write(workspace.root().join("build-cache.bin"), "keep").unwrap();
        fs::remove_file(directory.path().join("settings.gradle.kts")).unwrap();
        OverlayWorkspace::prepare(&layout, &overlay).unwrap();
        assert!(!workspace.root().join("settings.gradle.kts").exists());
        assert!(workspace.root().join("build-cache.bin").is_file());

        OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();
        assert_eq!(
            fs::read_to_string(workspace.root().join("src/main/java/demo/App.java")).unwrap(),
            "class App { int value = 1; }"
        );
        assert!(
            !workspace
                .root()
                .join("src/main/java/demo/New.java")
                .exists()
        );
    }

    #[test]
    fn nested_workspace_keeps_repository_relative_composite_build_paths() {
        let directory = tempdir().unwrap();
        let initialized = Command::new("git")
            .args(["-C", directory.path().to_str().unwrap(), "init", "-q"])
            .status()
            .unwrap();
        assert!(initialized.success());
        fs::write(
            directory.path().join("settings.gradle.kts"),
            "rootProject.name=\"logic\"",
        )
        .unwrap();
        let application = directory.path().join("examples/app");
        fs::create_dir_all(application.join("src/main/java/demo")).unwrap();
        fs::write(
            application.join("settings.gradle.kts"),
            "pluginManagement { includeBuild(\"../..\") }",
        )
        .unwrap();
        fs::write(
            application.join("src/main/java/demo/App.java"),
            "class App {}",
        )
        .unwrap();
        let added = Command::new("git")
            .args(["-C", directory.path().to_str().unwrap(), "add", "."])
            .status()
            .unwrap();
        assert!(added.success());
        let layout = WorkspaceLayout::new(&application).unwrap();
        layout.ensure_state_dir().unwrap();

        let workspace = OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();

        assert!(workspace.root().join("settings.gradle.kts").is_file());
        assert!(
            workspace
                .root()
                .join("../..")
                .join("settings.gradle.kts")
                .is_file()
        );
    }

    #[test]
    fn mirror_comparison_avoids_recopies_but_detects_same_length_changes() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.java");
        let destination = directory.path().join("destination.java");
        fs::write(&source, "class A {}").unwrap();
        let mut first = MirrorManifest::new();
        mirror_file(
            &source,
            &destination,
            "source.java".into(),
            &MirrorManifest::new(),
            &mut first,
            &|| false,
        )
        .unwrap();
        let copied_at = destination.metadata().unwrap().modified().unwrap();

        let mut second = MirrorManifest::new();
        mirror_file(
            &source,
            &destination,
            "source.java".into(),
            &first,
            &mut second,
            &|| false,
        )
        .unwrap();
        assert_eq!(
            destination.metadata().unwrap().modified().unwrap(),
            copied_at
        );

        fs::write(&source, "class B {}").unwrap();
        let mut third = MirrorManifest::new();
        mirror_file(
            &source,
            &destination,
            "source.java".into(),
            &second,
            &mut third,
            &|| false,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(destination).unwrap(), "class B {}");
    }

    #[test]
    fn mirror_handles_source_file_directory_transitions() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("changing");
        fs::write(&source, "file").unwrap();
        let layout = WorkspaceLayout::new(directory.path()).unwrap();
        layout.ensure_state_dir().unwrap();

        let workspace = OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();
        assert_eq!(
            fs::read_to_string(workspace.root().join("changing")).unwrap(),
            "file"
        );

        fs::remove_file(&source).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("Child.java"), "class Child {}").unwrap();
        OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();
        assert!(workspace.root().join("changing/Child.java").is_file());

        fs::remove_dir_all(&source).unwrap();
        fs::write(&source, "file again").unwrap();
        OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();
        assert_eq!(
            fs::read_to_string(workspace.root().join("changing")).unwrap(),
            "file again"
        );
    }

    #[test]
    fn interrupted_or_corrupt_mirror_state_recovers_without_manual_cleanup() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("App.java"), "class App {}").unwrap();
        let layout = WorkspaceLayout::new(directory.path()).unwrap();
        layout.ensure_state_dir().unwrap();
        let workspace = OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();
        fs::write(
            workspace.root().join("build-cache.bin"),
            "stale build state",
        )
        .unwrap();
        let session = layout.state_dir.join("live/vscode");
        fs::write(session.join("mirrored-files.json"), "{truncated").unwrap();

        let recovered = OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();

        assert_eq!(
            fs::read_to_string(recovered.root().join("App.java")).unwrap(),
            "class App {}"
        );
        assert!(!recovered.root().join("build-cache.bin").exists());
        assert!(!session.join("mirror-preparing").exists());
    }

    #[test]
    fn cancellation_marks_partial_mirror_for_safe_rebuild() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("App.java"), "class App {}").unwrap();
        fs::write(directory.path().join("Other.java"), "class Other {}").unwrap();
        let layout = WorkspaceLayout::new(directory.path()).unwrap();
        layout.ensure_state_dir().unwrap();
        let checks = Cell::new(0_u32);
        let cancelled = || {
            checks.set(checks.get() + 1);
            checks.get() > 3
        };

        let error =
            OverlayWorkspace::prepare_cancellable(&layout, &request(Vec::new()), &cancelled)
                .unwrap_err();

        assert!(matches!(error, OverlayError::Cancelled));
        let session = layout.state_dir.join("live/vscode");
        assert!(session.join("mirror-preparing").is_file());
        let recovered = OverlayWorkspace::prepare(&layout, &request(Vec::new())).unwrap();
        assert!(recovered.root().join("App.java").is_file());
        assert!(recovered.root().join("Other.java").is_file());
        assert!(!session.join("mirror-preparing").exists());
    }
}
