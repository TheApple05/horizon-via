use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, Metadata, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

type Result<T> = std::result::Result<T, String>;
const STAGE_MARKER: &str = ".horizon-mixins-stage.json";
const REVISION_MARKER: &str = ".horizon-revision";
const STAGED_CRATE: &str = "native-crates/horizon_via/source_patch";

struct Entry {
    path: String,
    before: Option<String>,
    after: String,
}

struct Prepared {
    path: String,
    target: PathBuf,
    payload: Vec<u8>,
    original: Option<String>,
    desired: String,
    changed: bool,
}

/// Applies only a pinned, entirely prevalidated overlay to an isolated launcher stage.
/// The callback registers every input and output with Cargo, including absent targets.
pub(crate) fn apply(manifest_dir: &Path, mut watch: impl FnMut(&Path)) -> Result<usize> {
    let workspace = workspace_for(manifest_dir)?;
    for marker in [STAGE_MARKER, REVISION_MARKER] {
        watch(&workspace.join(marker));
    }
    let manifest_path = manifest_dir.join("data/overlay.json");
    watch(&manifest_path);
    checked_path(manifest_dir, Path::new("data/overlay.json"), false)?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&read_file(&manifest_path, "overlay manifest")?)
            .map_err(|_| "overlay manifest is not valid JSON".to_string())?;
    let object = manifest
        .as_object()
        .ok_or("overlay manifest must be an object")?;
    if object.len() != 3 || manifest["schema"].as_u64() != Some(1) {
        return Err("unsupported overlay manifest schema or fields".into());
    }
    let revision = manifest["horizon_revision"]
        .as_str()
        .filter(|value| is_hex(value, 40))
        .ok_or("overlay manifest has no valid Horizon revision")?;
    validate_stage(&workspace, revision)?;
    let files = manifest["files"]
        .as_array()
        .ok_or("overlay manifest has no files array")?;
    if files.is_empty() {
        return Err("overlay manifest has no files".into());
    }

    let mut entries = Vec::with_capacity(files.len());
    let mut seen = BTreeSet::new();
    for file in files {
        let record = file.as_object().ok_or("overlay file must be an object")?;
        if record.len() != 3
            || !record.contains_key("path")
            || !record.contains_key("before_sha256")
            || !record.contains_key("after_sha256")
        {
            return Err("overlay file has unsupported fields".into());
        }
        let path = file["path"].as_str().ok_or("overlay file has no path")?;
        validate_relative(path)?;
        if !allowed(path) {
            return Err(format!("target is outside the source allowlist: {path}"));
        }
        if !seen.insert(path.to_ascii_lowercase()) {
            return Err(format!("duplicate overlay target: {path}"));
        }
        let before = if file["before_sha256"].is_null() {
            None
        } else {
            Some(
                file["before_sha256"]
                    .as_str()
                    .filter(|value| is_hex(value, 64))
                    .ok_or_else(|| format!("invalid original digest: {path}"))?
                    .to_string(),
            )
        };
        let after = file["after_sha256"]
            .as_str()
            .filter(|value| is_hex(value, 64))
            .ok_or_else(|| format!("invalid payload digest: {path}"))?
            .to_string();
        entries.push(Entry {
            path: path.to_string(),
            before,
            after,
        });
    }

    // No source file or directory is written until every target and payload passes.
    let mut prepared = Vec::with_capacity(entries.len());
    for entry in entries {
        let relative = Path::new(&entry.path);
        let target = workspace.join(relative);
        let payload_relative = Path::new("data/payload").join(relative);
        let payload_path = manifest_dir.join(&payload_relative);
        watch(&target);
        watch(&payload_path);
        checked_path(&workspace, relative, true)?;
        checked_path(manifest_dir, &payload_relative, false)?;
        let payload = normalized(&read_file(&payload_path, &entry.path)?, &entry.path)?;
        if digest(&payload) != entry.after {
            return Err(format!("payload digest mismatch: {}", entry.path));
        }
        let original = existing_digest(&target, &entry.path)?;
        let accepts_original = original == entry.before;
        let accepts_desired = original.as_deref() == Some(entry.after.as_str());
        if !accepts_original && !accepts_desired {
            return Err(format!(
                "source conflict: {} differs from both the pinned original and mod payload",
                entry.path
            ));
        }
        prepared.push(Prepared {
            path: entry.path,
            target,
            payload,
            original,
            desired: entry.after,
            changed: !accepts_desired,
        });
    }

    let mut written = 0;
    for entry in prepared {
        if !entry.changed {
            continue;
        }
        // Detect source changes during validation rather than overwriting another writer.
        validate_stage(&workspace, revision)?;
        checked_path(&workspace, Path::new(&entry.path), true)?;
        if existing_digest(&entry.target, &entry.path)? != entry.original {
            return Err(format!(
                "source changed during overlay validation: {}",
                entry.path
            ));
        }
        atomic_write(&workspace, &entry.target, &entry.path, &entry.payload)?;
        if existing_digest(&entry.target, &entry.path)?.as_deref() != Some(entry.desired.as_str()) {
            return Err(format!("written source digest mismatch: {}", entry.path));
        }
        written += 1;
    }
    Ok(written)
}

fn workspace_for(manifest_dir: &Path) -> Result<PathBuf> {
    if !manifest_dir.is_absolute() {
        return Err("source patch crate must have an absolute staged location".into());
    }
    let workspace = manifest_dir
        .ancestors()
        .nth(3)
        .ok_or("source patch crate is outside a native-mod stage")?;
    if workspace.join(STAGED_CRATE) != manifest_dir {
        return Err("source patch crate is outside native-crates/horizon_via/source_patch".into());
    }
    // Include ancestors above the stage: a symlinked stage parent is also a traversal.
    reject_reparse_chain(manifest_dir)?;
    Ok(workspace.to_path_buf())
}

fn validate_stage(workspace: &Path, revision: &str) -> Result<()> {
    reject_reparse_chain(workspace)?;
    match fs::symlink_metadata(workspace.join(".git")) {
        Ok(_) => {
            return Err("refusing to modify a Git checkout instead of a native-mod stage".into());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("cannot inspect stage Git boundary".into()),
    }
    checked_path(workspace, Path::new(REVISION_MARKER), false)?;
    checked_path(workspace, Path::new(STAGE_MARKER), false)?;
    let actual_revision = read_file(&workspace.join(REVISION_MARKER), REVISION_MARKER)?;
    if std::str::from_utf8(&actual_revision).map(str::trim) != Ok(revision) {
        return Err("native-mod stage Horizon revision does not match the overlay pin".into());
    }
    let stage: serde_json::Value =
        serde_json::from_slice(&read_file(&workspace.join(STAGE_MARKER), STAGE_MARKER)?)
            .map_err(|_| "native-mod stage marker is not valid JSON".to_string())?;
    if !stage["schema"].as_u64().is_some_and(|schema| schema >= 6) {
        return Err("native-mod stage marker has an unsupported schema".into());
    }
    let valid_crate = stage["crates"].as_array().is_some_and(|crates| {
        crates.iter().any(|entry| {
            entry["mod_id"] == "horizon_via"
                && entry["crate_id"] == "source_patch"
                && entry["package"] == "horizon-via-source-patch"
                && entry["path"] == STAGED_CRATE
                && entry["dependent"] == "horizon-core"
        })
    });
    if !valid_crate {
        return Err("stage marker does not register this patch crate before horizon-core".into());
    }
    Ok(())
}

fn validate_relative(path: &str) -> Result<()> {
    if path.is_empty()
        || path
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'/' | b'_' | b'-' | b'.'))
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("overlay contains an unsafe relative path".into());
    }
    Ok(())
}

fn allowed(path: &str) -> bool {
    (path.starts_with("crates/horizon-protocol/src/") && path.ends_with(".rs"))
        || matches!(
            path,
            "crates/horizon-protocol/examples/probe.rs"
                | "crates/horizon-client/src/menu.rs"
                | "crates/horizon-client/src/game/remote_stations.rs"
                | "crates/horizon-client/src/game/stations.rs"
                | "crates/horizon-client-render/src/light.rs"
                | "crates/horizon-client/src/game.rs"
                | "crates/horizon-client/src/game/world_entry.rs"
                | "crates/horizon-client/src/game/control.rs"
                | "crates/horizon-client/src/game/remote_projectiles.rs"
                | "crates/horizon-client/src/legacy47_flight.rs"
                | "crates/horizon-client/src/projectiles.rs"
                | "crates/horizon-protocol/src/adapter/v1_8_9/entity/fixtures/raw-fixtures.json"
                | "crates/horizon-protocol/src/adapter/v1_8_9/entity/fixtures/vanilla-entity-fixtures.json"
                | "crates/horizon-protocol/src/adapter/v1_8_9/inventory/extra-fixtures.json"
                | "crates/horizon-protocol/src/adapter/v1_8_9/worldfx/fixtures/vanilla-worldfx-fixtures.json"
                | "crates/horizon-protocol/src/adapter/v1_8_9/worldfx_extra/fixtures/bounds-oracle.hex"
                | "crates/horizon-protocol/src/adapter/v1_8_9/worldfx_extra/fixtures/potion-oracle.hex"
                | "crates/horizon-protocol/src/adapter/v1_8_9/worldfx_extra/fixtures/vanilla-extra-fixtures.json"
                | "crates/horizon-protocol/tests/checked47.rs"
                | "crates/horizon-protocol/tests/session47-goldens.json"
                | "crates/horizon-protocol/tests/session47_public.rs"
                | "crates/horizon-client-render/src/particles.rs"
                | "crates/horizon-client/src/game/lan_particles.rs"
                | "crates/horizon-client/src/game/remote.rs"
                | "crates/horizon-client/src/game/server_packs.rs"
                | "crates/horizon-client/src/game/remote_chat.rs"
                | "crates/horizon-client/src/lan/io_pool.rs"
                | "crates/horizon-client/src/item_sync.rs"
                | "crates/horizon-client/src/net.rs"
                | "crates/horizon-client/src/server_packs.rs"
                | "crates/horizon-client/src/multiplayer.rs"
                | "crates/horizon-client/src/title.rs"
                | "crates/horizon-client/src/multiplayer/version_select.rs"
                | "crates/horizon-client/assets/horizon/lang/en_us.json"
                | "crates/horizon-world/src/datafix/json.rs"
        )
}

/// All existing parents must be ordinary directories; the target must be a file.
/// Missing tails are allowed only when preparing new overlay target files.
fn checked_path(root: &Path, relative: &Path, allow_missing: bool) -> Result<()> {
    reject_reparse_chain(root)?;
    let parts: Vec<_> = relative.components().collect();
    let mut current = root.to_path_buf();
    for (index, component) in parts.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err("overlay contains an unsafe path component".into());
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                if is_reparse(&meta) {
                    return Err("overlay paths cannot traverse symlinks or reparse points".into());
                }
                let final_component = index + 1 == parts.len();
                if (final_component && !meta.is_file()) || (!final_component && !meta.is_dir()) {
                    return Err("overlay path has an unexpected file or directory type".into());
                }
            }
            Err(error) if allow_missing && error.kind() == io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(_) => return Err("overlay input is missing or inaccessible".into()),
        }
    }
    Ok(())
}

fn reject_reparse_chain(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) => {
                current.push(component);
                continue;
            }
            Component::RootDir | Component::Normal(_) => current.push(component),
            Component::ParentDir | Component::CurDir => {
                return Err("overlay location cannot contain relative traversal".into());
            }
        }
        let meta = fs::symlink_metadata(&current)
            .map_err(|_| "overlay location is missing or inaccessible".to_string())?;
        if is_reparse(&meta) || !meta.is_dir() {
            return Err("overlay location cannot traverse links or non-directories".into());
        }
    }
    Ok(())
}

fn is_reparse(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn read_file(path: &Path, label: &str) -> Result<Vec<u8>> {
    fs::read(path).map_err(|error| format!("cannot read {label}: {:?}", error.kind()))
}

fn normalized(bytes: &[u8], label: &str) -> Result<Vec<u8>> {
    std::str::from_utf8(bytes).map_err(|_| format!("source is not UTF-8: {label}"))?;
    let mut result = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
        }
        result.push(bytes[index]);
        index += 1;
    }
    Ok(result)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn existing_digest(path: &Path, label: &str) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(digest(&normalized(&bytes, label)?))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect source {label}: {:?}", error.kind())),
    }
}

fn atomic_write(workspace: &Path, target: &Path, label: &str, bytes: &[u8]) -> Result<()> {
    let parent = target.parent().ok_or("overlay target has no parent")?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "cannot create source directory for {label}: {:?}",
            error.kind()
        )
    })?;
    reject_reparse_chain(parent)?;
    if !parent.starts_with(workspace) {
        return Err("overlay target escaped the native-mod stage".into());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..16 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".horizon-via-overlay-{}-{serial}.tmp",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary);
        let mut file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("cannot stage source {label}: {:?}", error.kind()));
            }
        };
        let result = file.write_all(bytes).and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(format!("cannot write source {label}: {:?}", error.kind()));
        }
        if let Err(error) = fs::rename(&temporary, target) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("cannot install source {label}: {:?}", error.kind()));
        }
        return Ok(());
    }
    Err(format!(
        "cannot allocate an atomic source update for {label}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicU64;

    const REVISION: &str = "d8edfbcd0f70aac5b41bab12b8bbf9cc3626de1f";
    const FIRST: &str = "crates/horizon-protocol/src/adapter.rs";
    const SECOND: &str = "crates/horizon-protocol/src/item/legacy.rs";
    const EXTRA_RUNTIME: [&str; 8] = [
        "crates/horizon-client/src/game/lan_particles.rs",
        "crates/horizon-client/src/game/remote.rs",
        "crates/horizon-client/src/game/world_entry.rs",
        "crates/horizon-client/src/game/remote_chat.rs",
        "crates/horizon-client/src/lan/io_pool.rs",
        "crates/horizon-client/src/server_packs.rs",
        "crates/horizon-world/src/datafix/json.rs",
        "crates/horizon-client/src/item_sync.rs",
    ];

    struct Stage {
        root: PathBuf,
        package: PathBuf,
    }

    impl Stage {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "horizon-via-overlay-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let package = root.join(STAGED_CRATE);
            fs::create_dir_all(package.join("data/payload")).unwrap();
            fs::write(root.join(REVISION_MARKER), REVISION).unwrap();
            fs::write(
                root.join(STAGE_MARKER),
                serde_json::to_vec(&json!({"schema":6,"crates":[{
                    "mod_id":"horizon_via","crate_id":"source_patch",
                    "package":"horizon-via-source-patch","path":STAGED_CRATE,
                    "dependent":"horizon-core"
                }]}))
                .unwrap(),
            )
            .unwrap();
            Self { root, package }
        }

        fn file(&self, path: &str, before: Option<&str>, after: &str) -> serde_json::Value {
            if let Some(before) = before {
                self.write(path, before);
            }
            let payload = self.package.join("data/payload").join(path);
            fs::create_dir_all(payload.parent().unwrap()).unwrap();
            fs::write(payload, after).unwrap();
            json!({
                "path":path,
                "before_sha256":before.map(|s| digest(&normalized(s.as_bytes(),path).unwrap())),
                "after_sha256":digest(&normalized(after.as_bytes(),path).unwrap())
            })
        }

        fn write(&self, path: &str, contents: &str) {
            let target = self.root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, contents).unwrap();
        }

        fn manifest(&self, files: Vec<serde_json::Value>) {
            fs::write(
                self.package.join("data/overlay.json"),
                serde_json::to_vec(&json!({
                    "schema":1,"horizon_revision":REVISION,"files":files
                }))
                .unwrap(),
            )
            .unwrap();
        }

        fn apply(&self) -> Result<usize> {
            apply(&self.package, |_| {})
        }
    }

    impl Drop for Stage {
        fn drop(&mut self) {
            // Only this test's freshly-created and fixed-name scratch directory is removed.
            assert!(self.root.starts_with(std::env::temp_dir()));
            assert!(
                self.root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("horizon-via-overlay-test-")
            );
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn applies_existing_and_new_sources_and_is_idempotent_across_line_endings() {
        let stage = Stage::new();
        let mut files = vec![
            stage.file(FIRST, Some("original\r\n"), "changed\r\n"),
            stage.file(SECOND, None, "added\n"),
        ];
        files.extend(
            EXTRA_RUNTIME
                .iter()
                .map(|path| stage.file(path, Some("old runtime\r\n"), "new runtime\r\n")),
        );
        stage.manifest(files);
        let total = 2 + EXTRA_RUNTIME.len();
        assert_eq!(stage.apply().unwrap(), total);
        assert_eq!(fs::read(stage.root.join(FIRST)).unwrap(), b"changed\n");
        for path in EXTRA_RUNTIME {
            assert_eq!(fs::read(stage.root.join(path)).unwrap(), b"new runtime\n");
        }
        assert_eq!(stage.apply().unwrap(), 0);
        stage.write(FIRST, "changed\r\n");
        assert_eq!(stage.apply().unwrap(), 0);
        // A launcher rebuild restores the pristine source and removes added files.
        stage.write(FIRST, "original\n");
        fs::remove_file(stage.root.join(SECOND)).unwrap();
        for path in EXTRA_RUNTIME {
            stage.write(path, "old runtime\n");
        }
        assert_eq!(stage.apply().unwrap(), total);
        for path in EXTRA_RUNTIME {
            assert_eq!(fs::read(stage.root.join(path)).unwrap(), b"new runtime\n");
        }
    }

    #[test]
    fn validates_every_source_before_writing_any_target() {
        let stage = Stage::new();
        stage.manifest(vec![
            stage.file(FIRST, Some("first original"), "first modded"),
            stage.file(SECOND, Some("second original"), "second modded"),
            stage.file(EXTRA_RUNTIME[5], Some("world original"), "world modded"),
        ]);
        stage.write(EXTRA_RUNTIME[5], "another mod's edit");
        assert!(stage.apply().unwrap_err().contains(EXTRA_RUNTIME[5]));
        assert_eq!(
            fs::read_to_string(stage.root.join(FIRST)).unwrap(),
            "first original"
        );
        assert_eq!(
            fs::read_to_string(stage.root.join(SECOND)).unwrap(),
            "second original"
        );
        assert_eq!(
            fs::read_to_string(stage.root.join(EXTRA_RUNTIME[5])).unwrap(),
            "another mod's edit"
        );
    }

    #[test]
    fn rejects_a_tampered_later_payload_before_any_source_write() {
        let stage = Stage::new();
        stage.manifest(vec![
            stage.file(FIRST, Some("original"), "changed"),
            stage.file(SECOND, None, "added"),
        ]);
        fs::write(stage.package.join("data/payload").join(SECOND), "tampered").unwrap();
        assert!(
            stage
                .apply()
                .unwrap_err()
                .contains("payload digest mismatch")
        );
        assert_eq!(
            fs::read_to_string(stage.root.join(FIRST)).unwrap(),
            "original"
        );
        assert!(!stage.root.join(SECOND).exists());
    }

    #[test]
    fn rejects_unmarked_wrong_revision_and_git_checkouts() {
        let stage = Stage::new();
        stage.manifest(vec![stage.file(FIRST, Some("original"), "changed")]);
        fs::write(stage.root.join(REVISION_MARKER), "0".repeat(40)).unwrap();
        assert!(stage.apply().unwrap_err().contains("revision"));
        fs::write(stage.root.join(REVISION_MARKER), REVISION).unwrap();
        fs::write(stage.root.join(".git"), "gitdir: elsewhere").unwrap();
        assert!(stage.apply().unwrap_err().contains("Git checkout"));
        fs::remove_file(stage.root.join(".git")).unwrap();
        fs::remove_file(stage.root.join(STAGE_MARKER)).unwrap();
        assert!(stage.apply().is_err());
        assert_eq!(
            fs::read_to_string(stage.root.join(FIRST)).unwrap(),
            "original"
        );
    }

    #[test]
    fn refuses_wrong_location_or_unregistered_patch_crate() {
        let stage = Stage::new();
        stage.manifest(vec![stage.file(FIRST, Some("original"), "changed")]);
        assert!(apply(&stage.root, |_| {}).unwrap_err().contains("outside"));
        fs::write(stage.root.join(STAGE_MARKER), r#"{"schema":6,"crates":[]}"#).unwrap();
        assert!(stage.apply().unwrap_err().contains("register"));
        assert_eq!(
            fs::read_to_string(stage.root.join(FIRST)).unwrap(),
            "original"
        );
    }

    #[test]
    fn refuses_path_traversal_unlisted_targets_and_duplicate_windows_paths() {
        for path in [
            "../outside.rs",
            "crates/horizon-protocol/src/../../outside.rs",
            "crates\\horizon-protocol\\src\\adapter.rs",
            "C:/outside.rs",
            "crates/horizon-client/src/auth.rs",
            "crates/horizon-client/src/game/auth.rs",
            "crates/horizon-client/src/item_sync/other.rs",
            "crates/horizon-client/src/item_sync_private.rs",
            "crates/horizon-world/src/datafix/other.rs",
            "crates/horizon-protocol/src/adapter.rs:secret",
            "crates/horizon-protocol/src//adapter.rs",
        ] {
            let stage = Stage::new();
            let mut entry = stage.file(FIRST, Some("original"), "changed");
            entry["path"] = path.into();
            stage.manifest(vec![entry]);
            assert!(stage.apply().is_err(), "accepted {path}");
            assert_eq!(
                fs::read_to_string(stage.root.join(FIRST)).unwrap(),
                "original"
            );
        }
        let stage = Stage::new();
        let entry = stage.file(FIRST, Some("original"), "changed");
        stage.manifest(vec![entry.clone(), entry]);
        assert!(stage.apply().unwrap_err().contains("duplicate"));
    }

    #[test]
    fn tracks_existing_and_absent_sources_payloads_and_stage_inputs() {
        let stage = Stage::new();
        stage.manifest(vec![
            stage.file(FIRST, Some("original"), "changed"),
            stage.file(SECOND, None, "added"),
        ]);
        let mut tracked = BTreeSet::new();
        apply(&stage.package, |path| {
            tracked.insert(path.to_path_buf());
        })
        .unwrap();
        for path in [FIRST, SECOND, STAGE_MARKER, REVISION_MARKER] {
            assert!(
                tracked.contains(&stage.root.join(path)),
                "not tracked: {path}"
            );
        }
        assert!(tracked.contains(&stage.package.join("data/overlay.json")));
        for path in [FIRST, SECOND] {
            assert!(tracked.contains(&stage.package.join("data/payload").join(path)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_target_parent_and_payload() {
        use std::os::unix::fs::symlink;
        let stage = Stage::new();
        stage.manifest(vec![stage.file(FIRST, Some("original"), "changed")]);
        let target = stage.root.join(FIRST);
        let held = target.with_extension("held");
        fs::rename(&target, &held).unwrap();
        symlink(&held, &target).unwrap();
        assert!(stage.apply().unwrap_err().contains("symlinks"));
        assert_eq!(fs::read_to_string(&held).unwrap(), "original");
        fs::remove_file(&target).unwrap();
        fs::rename(&held, &target).unwrap();
        let payload = stage.package.join("data/payload").join(FIRST);
        let held_payload = payload.with_extension("held");
        fs::rename(&payload, &held_payload).unwrap();
        symlink(&held_payload, &payload).unwrap();
        assert!(stage.apply().unwrap_err().contains("symlinks"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "original");
    }
    #[test]
    fn protocol47_new_paths_are_explicitly_allowlisted() {
        for path in [
            "crates/horizon-client-render/src/light.rs",
            "crates/horizon-client/src/game.rs",
            "crates/horizon-client/src/game/control.rs",
            "crates/horizon-client/src/game/remote_projectiles.rs",
            "crates/horizon-client/src/legacy47_flight.rs",
            "crates/horizon-client/src/projectiles.rs",
            "crates/horizon-protocol/src/adapter/v1_8_9/entity/fixtures/raw-fixtures.json",
            "crates/horizon-protocol/src/adapter/v1_8_9/entity/fixtures/vanilla-entity-fixtures.json",
            "crates/horizon-protocol/src/adapter/v1_8_9/inventory/extra-fixtures.json",
            "crates/horizon-protocol/src/adapter/v1_8_9/worldfx/fixtures/vanilla-worldfx-fixtures.json",
            "crates/horizon-protocol/src/adapter/v1_8_9/worldfx_extra/fixtures/bounds-oracle.hex",
            "crates/horizon-protocol/src/adapter/v1_8_9/worldfx_extra/fixtures/potion-oracle.hex",
            "crates/horizon-protocol/src/adapter/v1_8_9/worldfx_extra/fixtures/vanilla-extra-fixtures.json",
            "crates/horizon-protocol/tests/checked47.rs",
            "crates/horizon-protocol/tests/session47-goldens.json",
            "crates/horizon-protocol/tests/session47_public.rs",
        ] {
            assert!(allowed(path), "{path}");
        }
        for path in [
            "crates/horizon-protocol/src/adapter/v1_8_9/fixtures/unreviewed.json",
            "crates/horizon-protocol/src/adapter/v1_8_9/fixtures/private.jar",
            "crates/horizon-client/src/game/unreviewed.rs",
        ] {
            assert!(!allowed(path), "{path}");
        }
    }
}
