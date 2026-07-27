//! Regression test for #3478: folder import endpoint.
//!
//! The `POST /api/notes/import-folder` HTTP endpoint is a thin wrapper over
//! `import_markdown_async`, which in turn delegates to the existing
//! `import_markdown_with_context` / `collect_markdown_files` pipeline.
//! This test verifies that pipeline recursively walks a directory tree and
//! imports every `.md` file, ignoring non-markdown files. The HTTP-layer
//! request/response deserialization tests live in `http_bridge.rs`.

#[cfg(test)]
mod tests {
    use crate::storage::{
        import_markdown_with_context, initialize_storage_with_context, StorageContext,
    };
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn import_folder_walks_directory_recursively() {
        // Use PID + UUID in the temp dir name for process-level uniqueness
        // (mitigates the Windows CI flaky-test pattern documented in the
        // pipeline-developer skill — see the "flaky 测试修复模式" note).
        let temp = std::env::temp_dir().join(format!(
            "vaultpilot-issue3478-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&temp).expect("temp dir");

        let ctx = StorageContext::for_test(&temp);
        initialize_storage_with_context(&ctx).expect("init storage");

        // Build a nested folder structure inside the vault so the importer
        // can reach it (imports must be inside the vault dir, per #1826).
        let vault_root = temp.join("vault");
        let sub1 = vault_root.join("folder-a");
        let sub2 = vault_root.join("folder-b").join("nested");
        fs::create_dir_all(&sub1).expect("sub1");
        fs::create_dir_all(&sub2).expect("sub2");

        fs::write(
            sub1.join("note1.md"),
            "---\ntitle: Note One\n---\n\nBody of note one\n",
        )
        .expect("write note1");
        fs::write(
            sub2.join("note2.md"),
            "---\ntitle: Note Two\n---\n\nBody of note two\n",
        )
        .expect("write note2");
        // Non-markdown file should be ignored by the walker.
        fs::write(sub1.join("image.png"), b"PNG").expect("write image");

        let result =
            import_markdown_with_context(&ctx, &[vault_root.to_string_lossy().to_string()])
                .expect("import");
        assert_eq!(result.imported, 2, "expected 2 markdown files imported");
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);

        // Cleanup
        let _ = fs::remove_dir_all(&temp);
    }
}
