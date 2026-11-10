//! Readers for each agent's local logs. Each returns one [`Event`] per model request.

pub mod claude;
pub mod codex;
pub mod kimi;
pub mod opencode;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::event::Event;

pub fn load(agent: &str) -> Vec<Event> {
    match agent {
        "claude" => claude::load(),
        "codex" => codex::load(),
        "kimi" => kimi::load(),
        "opencode" => opencode::load(),
        _ => Vec::new(),
    }
}

/// Where an agent keeps its data; the source id file lives here too.
pub fn home(agent: &str) -> Option<PathBuf> {
    match agent {
        "claude" => claude::roots().into_iter().next(),
        "codex" => Some(codex::root()),
        "kimi" => kimi::roots().into_iter().next(),
        "opencode" => Some(opencode::root()),
        _ => None,
    }
}

pub fn user_home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Files under `root` whose name satisfies `keep`, sorted so results are deterministic.
pub fn files(root: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name().to_str().is_some_and(&keep))
        .map(walkdir::DirEntry::into_path)
        .collect();
    found.sort();
    found
}

/// Lines of a file (lossy UTF-8), skipping unreadable ones.
pub fn lines(path: &Path) -> impl Iterator<Item = String> {
    let reader = File::open(path).ok().map(BufReader::new);
    reader.into_iter().flat_map(|reader| reader.split(b'\n').map_while(Result::ok).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}
