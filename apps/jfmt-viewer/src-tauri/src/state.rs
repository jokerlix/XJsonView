use std::sync::Arc;

use dashmap::DashMap;
use jfmt_viewer_core::Session;

pub struct ViewerState {
    pub sessions: DashMap<String, Arc<Session>>,
    pub search_cancels: DashMap<String, Arc<std::sync::atomic::AtomicBool>>,
}

impl ViewerState {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            search_cancels: DashMap::new(),
        }
    }
}
