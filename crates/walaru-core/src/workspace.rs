//! Content revision capture and worktree-local state layout.

use std::collections::BTreeMap;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::event::{RevisionId, WorkspaceId};

/// Workspace discovery and hashing failure.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// Filesystem traversal or read failed.
    #[error("workspace I/O error: {0}")]
    Io(#[from] io::Error),
    /// A walked path unexpectedly escaped its root.
    #[error("path `{path}` is outside workspace `{workspace}`")]
    OutsideWorkspace {
        /// Escaping path.
        path: PathBuf,
        /// Expected workspace root.
        workspace: PathBuf,
    },
}

/// Deterministic digest of source, resource, and build inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionSnapshot {
    /// Versioned revision ID.
    pub revision: RevisionId,
    /// Included workspace-relative paths, useful for diagnostics.
    pub files: Vec<String>,
    /// Content digest per included workspace-relative path.
    pub file_digests: BTreeMap<String, String>,
    /// Conservative source surface/global-initializer digest per included path.
    pub surface_digests: BTreeMap<String, String>,
}

impl RevisionSnapshot {
    /// Captures all execution-relevant regular files in canonical path order.
    pub fn capture(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = root.as_ref();
        let mut paths = Vec::new();
        collect_files(root, root, &mut paths)?;
        paths.sort_by_key(|path| relative_string(root, path));

        let mut hasher = Sha256::new();
        hasher.update(b"walaru-revision-v1\0");
        let mut files = Vec::with_capacity(paths.len());
        let mut file_digests = BTreeMap::new();
        let mut surface_digests = BTreeMap::new();
        for path in paths {
            let relative = relative_string(root, &path);
            let contents = fs::read(&path)?;
            update_frame(&mut hasher, relative.as_bytes());
            update_frame(&mut hasher, &contents);
            file_digests.insert(relative.clone(), digest(&contents));
            surface_digests.insert(relative.clone(), source_surface_digest(&path, &contents));
            files.push(relative);
        }
        Ok(Self {
            revision: RevisionId::from_digest(hasher.finalize().into()),
            files,
            file_digests,
            surface_digests,
        })
    }
}

/// Paths used by one worktree daemon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceLayout {
    /// Canonical worktree root.
    pub root: PathBuf,
    /// Stable worktree identity.
    pub workspace_id: WorkspaceId,
    /// `.gradle/walaru/<worktree-id>`.
    pub state_dir: PathBuf,
    /// Worktree-local transport endpoint (Unix socket or Windows loopback descriptor).
    pub socket: PathBuf,
    /// SQLite database.
    pub database: PathBuf,
    /// Daemon PID/version metadata.
    pub daemon_metadata: PathBuf,
}

impl WorkspaceLayout {
    /// Resolves a canonical worktree layout without mutating it.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = root.as_ref().canonicalize()?;
        let workspace_id = WorkspaceId::from_path(&root.to_string_lossy());
        let state_dir = root
            .join(".gradle")
            .join("walaru")
            .join(workspace_id.as_str());
        let socket = local_endpoint_path(&state_dir, &workspace_id);
        Ok(Self {
            socket,
            database: state_dir.join("store.sqlite3"),
            daemon_metadata: state_dir.join("daemon.json"),
            root,
            workspace_id,
            state_dir,
        })
    }

    /// Creates only Walaru's private state directory.
    pub fn ensure_state_dir(&self) -> Result<(), WorkspaceError> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(&self.state_dir)?;
        Ok(())
    }
}

#[cfg(unix)]
fn local_endpoint_path(state_dir: &Path, workspace_id: &WorkspaceId) -> PathBuf {
    // macOS limits sockaddr_un paths to 104 bytes and Linux to 108. Leave room
    // for the terminating NUL and platform variation, while keeping normal
    // worktrees entirely under their private state directory.
    const SAFE_UNIX_SOCKET_PATH_BYTES: usize = 96;
    let local = state_dir.join("daemon.sock");
    if local.as_os_str().as_bytes().len() < SAFE_UNIX_SOCKET_PATH_BYTES {
        local
    } else {
        Path::new("/tmp").join(format!("walaru-{}.sock", workspace_id.as_str()))
    }
}

#[cfg(not(unix))]
fn local_endpoint_path(state_dir: &Path, _workspace_id: &WorkspaceId) -> PathBuf {
    state_dir.join("daemon.sock")
}

fn collect_files(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if !ignored_directory(&entry.file_name().to_string_lossy()) {
                collect_files(root, &path, output)?;
            }
        } else if file_type.is_file() && included_file(root, &path) {
            output.push(path);
        }
    }
    Ok(())
}

fn ignored_directory(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".gradle" | ".idea" | ".kotlin" | "build" | "out" | "target" | "node_modules"
    )
}

fn included_file(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative
        .components()
        .next()
        .is_some_and(|part| part.as_os_str() == "src")
    {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if matches!(
        name,
        "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "gradle.properties"
            | "libs.versions.toml"
            | "gradle-wrapper.properties"
            | "pom.xml"
            | "maven.config"
            | "jvm.config"
            | "extensions.xml"
            | "maven-wrapper.properties"
            | "walaru.toml"
    ) {
        return true;
    }
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("java" | "kt" | "kts" | "gradle")
    )
}

fn relative_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn update_frame(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn digest(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

/// Computes the conservative ABI/global-initializer surface digest for one input file.
#[must_use]
pub fn source_surface_digest(path: &Path, contents: &[u8]) -> String {
    let extension = path.extension().and_then(|value| value.to_str());
    let Some(source_kind) = extension.filter(|value| matches!(*value, "kt" | "kts" | "java"))
    else {
        return digest(contents);
    };
    let Ok(text) = std::str::from_utf8(contents) else {
        return digest(contents);
    };
    let surface = if matches!(source_kind, "kt" | "kts") {
        kotlin_surface(text)
    } else {
        java_surface(text)
    };
    digest(surface.as_bytes())
}

fn kotlin_surface(source: &str) -> String {
    let mut output = String::new();
    let mut brace_depth = 0_usize;
    for raw in source.lines() {
        let trimmed = raw.trim();
        let top_level = brace_depth == 0;
        let declaration = trimmed.starts_with("package ")
            || trimmed.starts_with("import ")
            || trimmed.starts_with('@')
            || contains_declaration(
                trimmed,
                &[
                    "class ",
                    "interface ",
                    "object ",
                    "fun ",
                    "val ",
                    "var ",
                    "typealias ",
                ],
            );
        if declaration {
            let global_property = top_level
                && contains_declaration(trimmed, &["val ", "var "])
                && !trimmed.starts_with("fun ");
            let selected = if global_property {
                normalize_space(trimmed)
            } else {
                normalize_space(cut_body(trimmed))
            };
            output.push_str(&selected);
            output.push('\n');
        }
        brace_depth = brace_depth
            .saturating_add(raw.chars().filter(|character| *character == '{').count())
            .saturating_sub(raw.chars().filter(|character| *character == '}').count());
    }
    output
}

fn java_surface(source: &str) -> String {
    if source
        .lines()
        .any(|line| line.trim_start().starts_with("static {"))
    {
        return source.into();
    }
    let mut output = String::new();
    for raw in source.lines() {
        let trimmed = raw.trim();
        let declaration = trimmed.starts_with("package ")
            || trimmed.starts_with("import ")
            || trimmed.starts_with('@')
            || contains_declaration(
                trimmed,
                &[
                    "class ",
                    "interface ",
                    "enum ",
                    "record ",
                    "public ",
                    "protected ",
                    "static ",
                ],
            );
        if declaration {
            let static_field =
                trimmed.contains(" static ") && trimmed.ends_with(';') && !trimmed.contains('(');
            let selected = if static_field {
                trimmed
            } else {
                cut_body(trimmed)
            };
            output.push_str(&normalize_space(selected));
            output.push('\n');
        }
    }
    output
}

fn contains_declaration(value: &str, declarations: &[&str]) -> bool {
    declarations.iter().any(|declaration| {
        value.starts_with(declaration) || value.contains(&format!(" {declaration}"))
    })
}

fn cut_body(value: &str) -> &str {
    let mut parentheses = 0_i32;
    let mut brackets = 0_i32;
    for (index, character) in value.char_indices() {
        match character {
            '(' => parentheses += 1,
            ')' => parentheses = (parentheses - 1).max(0),
            '[' => brackets += 1,
            ']' => brackets = (brackets - 1).max(0),
            '{' | '=' if parentheses == 0 && brackets == 0 => return value[..index].trim_end(),
            _ => {}
        }
    }
    value
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
