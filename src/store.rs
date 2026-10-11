pub mod catalog;
pub mod demands;
pub mod fs;
pub mod markdown;
pub mod registry;
pub mod sessions;
pub mod state;
pub mod workspace;

/// Valid records read from a directory, plus warnings for the ones that could not be read.
pub struct Scan<T> {
    pub items: Vec<T>,
    pub warnings: Vec<String>,
}

impl<T> Default for Scan<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
