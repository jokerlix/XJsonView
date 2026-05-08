use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use jfmt_viewer_core::{
    run_search, ChildSummary, Format, NodeId, SearchHit, SearchQuery, Session, ViewerError,
};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::state::ViewerState;

#[derive(Serialize)]
pub struct OpenFileResp {
    pub session_id: String,
    pub root_id: u64,
    pub format: Format,
    pub total_bytes: u64,
}

#[derive(Serialize, Clone)]
#[serde(tag = "phase", rename_all = "lowercase")]
#[allow(dead_code)] // Error variant emitted in M8.2 when index errors are
                    // surfaced through the channel; kept here so the frontend
                    // discriminated union stays stable.
pub enum IndexProgress {
    Scanning { bytes_done: u64, bytes_total: u64 },
    Ready { build_ms: u64 },
    Error { message: String },
}

#[tauri::command]
pub async fn open_file(
    path: String,
    on_progress: Channel<IndexProgress>,
    state: State<'_, ViewerState>,
) -> Result<OpenFileResp, ViewerError> {
    let path = PathBuf::from(&path);
    if !path.exists() {
        return Err(ViewerError::NotFound(path.display().to_string()));
    }
    let start = Instant::now();
    let session = tokio::task::spawn_blocking(move || Session::open(&path))
        .await
        .map_err(|e| ViewerError::Io(e.to_string()))??;
    let id = uuid::Uuid::new_v4().to_string();
    let total = session.byte_len();
    let format = session.format();
    state.sessions.insert(id.clone(), Arc::new(session));

    let _ = on_progress.send(IndexProgress::Scanning {
        bytes_done: total,
        bytes_total: total,
    });
    let _ = on_progress.send(IndexProgress::Ready {
        build_ms: start.elapsed().as_millis() as u64,
    });

    Ok(OpenFileResp {
        session_id: id,
        root_id: NodeId::ROOT.0,
        format,
        total_bytes: total,
    })
}

#[tauri::command]
pub async fn close_file(
    session_id: String,
    state: State<'_, ViewerState>,
) -> Result<(), ViewerError> {
    state.sessions.remove(&session_id);
    Ok(())
}

#[derive(Serialize)]
pub struct GetChildrenResp {
    pub items: Vec<ChildSummary>,
    pub total: u32,
}

#[tauri::command]
pub async fn get_children(
    session_id: String,
    parent: u64,
    offset: u32,
    limit: u32,
    state: State<'_, ViewerState>,
) -> Result<GetChildrenResp, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let resp = session.get_children(NodeId(parent), offset, limit)?;
    Ok(GetChildrenResp {
        items: resp.items,
        total: resp.total,
    })
}

#[derive(Serialize)]
pub struct GetValueResp {
    pub json: String,
    pub truncated: bool,
}

#[tauri::command]
pub async fn get_value(
    session_id: String,
    node: u64,
    max_bytes: Option<u64>,
    state: State<'_, ViewerState>,
) -> Result<GetValueResp, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let resp = session.get_value(NodeId(node), max_bytes)?;
    Ok(GetValueResp {
        json: resp.json,
        truncated: resp.truncated,
    })
}

#[derive(Serialize)]
pub struct GetPointerResp {
    pub pointer: String,
}

#[tauri::command]
pub async fn get_pointer(
    session_id: String,
    node: u64,
    state: State<'_, ViewerState>,
) -> Result<GetPointerResp, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let pointer = session.get_pointer(NodeId(node))?;
    Ok(GetPointerResp { pointer })
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[allow(dead_code)] // Progress + Error variants constructed in M8.2 once
                    // run_search emits intermediate events; kept here so the
                    // frontend's TypeScript union type stays stable.
pub enum SearchEvent {
    Hit {
        node: Option<u64>,
        path: String,
        matched_in: jfmt_viewer_core::MatchedIn,
        snippet: String,
    },
    Progress {
        bytes_done: u64,
        bytes_total: u64,
        hits_so_far: u32,
    },
    Done {
        total_hits: u32,
        elapsed_ms: u64,
    },
    Cancelled,
    Error {
        message: String,
    },
}

#[derive(Serialize)]
pub struct SearchHandle {
    pub id: String,
}

#[tauri::command]
pub async fn search(
    session_id: String,
    query: SearchQuery,
    on_event: Channel<SearchEvent>,
    state: State<'_, ViewerState>,
) -> Result<SearchHandle, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let handle_id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .search_cancels
        .insert(handle_id.clone(), cancel.clone());

    let on_event_clone = on_event.clone();
    let cancel_clone = cancel.clone();
    let started = Instant::now();
    tokio::task::spawn_blocking(move || {
        let result = run_search(&session, &query, &cancel_clone, |hit: &SearchHit| {
            let _ = on_event_clone.send(SearchEvent::Hit {
                node: hit.node.map(|n| n.0),
                path: hit.path.clone(),
                matched_in: hit.matched_in,
                snippet: hit.snippet.clone(),
            });
        });
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(s) if s.cancelled => {
                let _ = on_event_clone.send(SearchEvent::Cancelled);
            }
            Ok(s) => {
                let _ = on_event_clone.send(SearchEvent::Done {
                    total_hits: s.total_hits,
                    elapsed_ms,
                });
            }
            Err(e) => {
                let _ = on_event_clone.send(SearchEvent::Error {
                    message: e.to_string(),
                });
            }
        }
    });

    Ok(SearchHandle { id: handle_id })
}

#[tauri::command]
pub async fn cancel_search(
    handle: String,
    state: State<'_, ViewerState>,
) -> Result<(), ViewerError> {
    if let Some((_, cancel)) = state.search_cancels.remove(&handle) {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}
