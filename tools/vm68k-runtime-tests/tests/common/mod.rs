use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct Source(pub PathBuf);
impl Source {
    pub fn new(source: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "actionc-68k-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("probe.act");
        std::fs::write(&path, source).unwrap();
        Self(path)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
