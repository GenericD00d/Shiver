//! Where each webview goes in the window: the rail, the content area (narrowed by the DM list
//! while a conversation shows), the bell and its popup, in logical pixels.

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, Window};

use super::{ActiveServer, BELL_SIZE, DM_LIST_WIDTH, POPUP_SIZE, RAIL_WIDTH};
use crate::error::Result;

pub(super) fn logical_size(window: &Window) -> Result<LogicalSize<f64>> {
    Ok(window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?))
}

/// Everything right of `left`, full height.
fn rect_from(window: &Window, left: f64) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let size = logical_size(window)?;

    Ok((
        LogicalPosition::new(left, 0.0),
        LogicalSize::new((size.width - left).max(0.0), size.height.max(0.0)),
    ))
}

pub(super) fn content_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    rect_from(window, RAIL_WIDTH)
}

pub(super) fn dm_content_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    rect_from(window, RAIL_WIDTH + DM_LIST_WIDTH)
}

/// Where an entry's page goes when it is not fullscreen.
pub(super) fn page_rect(
    app: &AppHandle,
    window: &Window,
    entry_id: &str,
) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let conversation = app
        .state::<ActiveServer>()
        .update(|screen| screen.conversation.as_deref() == Some(entry_id));

    if conversation {
        dm_content_rect(window)
    } else {
        content_rect(window)
    }
}

/// The bell, fixed to the top-right corner inside the space the bridge reserves in Sharkord's bar.
pub(super) fn bell_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let size = logical_size(window)?;
    let (width, height) = BELL_SIZE;

    Ok((
        LogicalPosition::new((size.width - width).max(RAIL_WIDTH), 0.0),
        LogicalSize::new(width, height),
    ))
}

/// The popup, under the bell, clamped to the window.
pub(super) fn popup_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let size = logical_size(window)?;
    let width = POPUP_SIZE.0.min((size.width - RAIL_WIDTH).max(0.0));
    let height = POPUP_SIZE.1.min((size.height - BELL_SIZE.1).max(0.0));

    Ok((
        LogicalPosition::new((size.width - width).max(RAIL_WIDTH), BELL_SIZE.1),
        LogicalSize::new(width, height),
    ))
}

pub(super) fn place(
    webview: &Webview,
    (position, size): (LogicalPosition<f64>, LogicalSize<f64>),
) -> Result<()> {
    webview.set_position(position)?;
    webview.set_size(size)?;

    Ok(())
}
