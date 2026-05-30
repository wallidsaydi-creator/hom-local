use std::path::PathBuf;

pub fn hom_local_dir() -> PathBuf {
    if let Ok(value) = std::env::var("HOM_LOCAL_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".hom").join("local")
}

pub fn socket_path(hom_dir: impl Into<PathBuf>, helper: &str) -> PathBuf {
    let hom_dir = hom_dir.into();
    let candidate = hom_dir.join("sockets").join(format!("{helper}.sock"));
    if candidate.as_os_str().len() <= 100 {
        return candidate;
    }

    let uid = std::env::var("UID").unwrap_or_else(|_| "local".to_string());
    PathBuf::from("/tmp")
        .join(format!("hom-{uid}"))
        .join(format!("{helper}.sock"))
}
