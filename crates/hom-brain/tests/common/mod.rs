//! Shared test utilities for public integration tests.

use tempfile::TempDir;

/// Create a temporary brain instance for testing.
pub fn create_test_brain() -> (TempDir, hom_brain::App) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let app =
        hom_brain::App::new(dir.path().to_path_buf()).expect("failed to create brain instance");
    (dir, app)
}
