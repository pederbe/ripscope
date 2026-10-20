//! Systems without a watch Ripscope uses: every query refreshes.

use super::{Cover, Happened};
use std::path::Path;

pub(super) struct Backend;

impl Backend {
    pub(super) const FOLLOWS_DIRECTORY: bool = false;
    pub(super) const FOLLOWS_LINKS: bool = false;
    pub(super) const ECHOES_ENTRIES: bool = false;

    pub(super) fn new(_root: &Path) -> Result<Self, String> {
        Err("this system has no file system watch Ripscope uses".into())
    }

    pub(super) fn discover(&mut self, _directory: &Path, _recursive: bool) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn cover(&mut self, _directory: &Path) -> Result<Cover, String> {
        Ok(Cover::Unwatched)
    }

    pub(super) fn tidy(&mut self) {}

    pub(super) fn events(&mut self) -> Vec<Happened> {
        vec![Happened::Lost]
    }
}
