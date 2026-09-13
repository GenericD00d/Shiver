//! Window and webview layout.
//!
//! Shiver draws one window holding the shell webview (Shiver's own UI, at the app origin) plus one
//! webview per server, each locked to that server's origin. The shell fills the window and the
//! active server sits on top of it in the content area, so showing a Shiver panel is just a matter
//! of hiding the server webview rather than fighting webview z-order.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde_json::json;
use tauri::{
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl, Window,
    WindowEvent,
};
use url::Url;

use crate::{
    error::{Error, Result},
    model::{is_same_origin, ServerEntry, Settings},
};

pub const MAIN_WINDOW: &str = "main";
pub const SHELL_WEBVIEW: &str = "shell";

/// Width of the server rail. The shell renders the rail in this strip and Shiver keeps the content
/// area clear of it, so the two never overlap.
pub const RAIL_WIDTH: f64 = 72.0;

/// Shiver's notification bell, as a small webview floating over the top-right of the server.
///
/// It is its own webview rather than part of the page because it shows a count across *every*
/// server, and no server may see that. It is not a full-width bar either: the bridge reserves this
/// much space at the right end of Sharkord's own top bar, so the bell lands inside that bar instead
/// of stacking a second one above it.
pub const OVERLAY_WEBVIEW: &str = "overlay";
const BELL_SIZE: (f64, f64) = (48.0, 48.0);

/// The notification feed, as a webview of its own, anchored under the bell.
///
/// Separate from the bell on purpose. One webview doing both meant resizing it on every click, and
/// a resize is two operations (move, then grow) with a painted frame in between: the bell visibly
/// jumped. A bell that never changes geometry cannot jump, whatever the popup does.
pub const POPUP_WEBVIEW: &str = "popup";
const POPUP_SIZE: (f64, f64) = (380.0, 540.0);

/// Event telling the bell whether its feed is open, so the two never disagree.
pub const POPUP_EVENT: &str = "shiver://popup";

/// How long after the feed dismisses itself a click on the bell still counts as the click that
/// dismissed it.
///
/// Clicking the bell while the feed is open moves focus to the bell first, and the feed closes on
/// losing it. The click then arrives at a bell the core believes is closed, and without this it
/// would reopen the thing the user just clicked to close.
const POPUP_REOPEN_GUARD: Duration = Duration::from_millis(400);

const BRIDGE_SOURCE: &str = include_str!("../generated/bridge.js");

/// Which server webview is currently on top, by entry id.
///
/// `gate` serialises everything that creates or reorders webviews. Two `select_server` calls can
/// overlap (a fast double click, or react's development double-render), and without it both see
/// "no webview yet" and race to create the same label, which fails for whichever loses.
#[derive(Default)]
pub struct ActiveServer {
    current: Mutex<Option<String>>,
    gate: Mutex<()>,
    overlay_expanded: Mutex<bool>,
    /// when the feed last dismissed itself, for `POPUP_REOPEN_GUARD`
    popup_dismissed_at: Mutex<Option<Instant>>,
    /// whether a server's own page is the thing on screen, rather than a Shiver panel
    showing_server: Mutex<bool>,
}

impl ActiveServer {
    pub fn get(&self) -> Option<String> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set(&self, value: Option<String>) {
        *self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
    }

    /// Whether the active server's page is actually on screen.
    ///
    /// `current` alone does not say this: it keeps naming a server while Shiver shows settings, the
    /// DM inbox or the connecting view over the top of it. The difference matters for treating a
    /// channel as read — a page that is merely *selected* while hidden is not one being looked at,
    /// and Sharkord notifies for the selected channel precisely when its window is hidden.
    pub fn showing_server(&self) -> bool {
        *self
            .showing_server
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_showing_server(&self, value: bool) {
        *self
            .showing_server
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
    }

    pub fn popup_open(&self) -> bool {
        *self
            .overlay_expanded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_popup_open(&self, value: bool) {
        *self
            .overlay_expanded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
    }

    pub fn note_popup_dismissed(&self) {
        *self
            .popup_dismissed_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
    }

    fn popup_dismissed_recently(&self) -> bool {
        self.popup_dismissed_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some_and(|at| at.elapsed() < POPUP_REOPEN_GUARD)
    }

    /// What a click on the bell should do.
    ///
    /// Not simply "the opposite of open". Clicking the bell while the feed is up focuses the bell,
    /// the feed dismisses itself on losing focus, and only then does the click arrive — at a core
    /// that now believes the feed is closed. Taken at face value that reopens what the user just
    /// clicked to close, so a dismissal this recent is treated as the feed still being open.
    pub fn should_open_popup(&self) -> bool {
        !self.popup_open() && !self.popup_dismissed_recently()
    }
}

pub fn webview_label(entry_id: &str) -> String {
    format!("server::{entry_id}")
}

pub fn main_window(app: &AppHandle) -> Result<Window> {
    app.get_window(MAIN_WINDOW)
        .ok_or_else(|| Error::Webview("The Shiver window is not open".into()))
}

/// Width of Shiver's cross-server DM list, matching Sharkord's own sidebar (`w-72`) so the two read
/// as one list.
pub const DM_LIST_WIDTH: f64 = 288.0;

/// A conversation is rendered by its *own* webview, separate from the one the user browses channels
/// in. Sharing one webview meant opening a DM from the inbox changed where the server view was
/// sitting: its DM mode and selected channel are page state, so returning to the server landed on
/// the conversation instead of the channel the user had left. Two webviews, two independent states.
pub fn dm_webview_label(entry_id: &str) -> String {
    format!("dm::{entry_id}")
}

fn is_shiver_chrome(label: &str) -> bool {
    matches!(label, SHELL_WEBVIEW | OVERLAY_WEBVIEW | POPUP_WEBVIEW)
}

/// Content area in logical pixels: the window minus the rail.
fn content_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);

    Ok((
        LogicalPosition::new(RAIL_WIDTH, 0.0),
        LogicalSize::new((size.width - RAIL_WIDTH).max(0.0), size.height.max(0.0)),
    ))
}

/// Where a conversation is drawn: right of the rail *and* right of Shiver's DM list.
fn dm_content_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);
    let left = RAIL_WIDTH + DM_LIST_WIDTH;

    Ok((
        LogicalPosition::new(left, 0.0),
        LogicalSize::new((size.width - left).max(0.0), size.height.max(0.0)),
    ))
}

/// Where the bell sits: hard against the top-right of the window, inside the space the bridge
/// reserves in Sharkord's top bar. Fixed, and never recalculated for the popup.
fn bell_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);
    let (width, height) = BELL_SIZE;

    Ok((
        LogicalPosition::new((size.width - width).max(RAIL_WIDTH), 0.0),
        LogicalSize::new(width, height),
    ))
}

/// Where the feed sits: directly under the bell, right edges aligned.
fn popup_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);
    let (width, height) = POPUP_SIZE;

    // clamped rather than pushed off-screen, so a small window still shows the whole panel
    let width = width.min((size.width - RAIL_WIDTH).max(0.0));
    let top = BELL_SIZE.1;
    let height = height.min((size.height - top).max(0.0));

    Ok((
        LogicalPosition::new((size.width - width).max(RAIL_WIDTH), top),
        LogicalSize::new(width, height),
    ))
}

/// Creates the window, the shell webview, and the resize handler that keeps everything aligned.
pub fn create_main_window(app: &AppHandle) -> Result<Window> {
    let window = tauri::window::WindowBuilder::new(app, MAIN_WINDOW)
        .title("Shiver")
        .inner_size(1280.0, 800.0)
        .min_inner_size(720.0, 480.0)
        // Dark, rather than whatever the desktop is set to. Everything *inside* this window is dark
        // by default — Shiver's own chrome and Sharkord's — so a light title bar and border was the
        // one part of the app that followed a setting nobody had made about it, framing a dark app
        // in white. Windows only draws the frame; there is nothing here to make configurable, and
        // the colours that are the user's own are in settings already.
        .theme(Some(tauri::Theme::Dark))
        .build()?;

    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);

    window.add_child(
        // the native drag-and-drop handler intercepts drags before the page sees them, which stops
        // html5 dragstart/dragover/drop from firing at all: the rail could not be reordered. Shiver
        // accepts no dropped files here, so nothing is lost by turning it off.
        WebviewBuilder::new(SHELL_WEBVIEW, WebviewUrl::App("index.html".into()))
            .disable_drag_drop_handler(),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.width, size.height),
    )?;

    ensure_overlay(app)?;

    let handle = app.clone();

    window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Err(error) = relayout(&handle) {
                eprintln!("[shiver] could not relayout after a resize: {error}");
            }
        }
    });

    Ok(window)
}

/// Resizes the shell to the window, every server webview to the content area, and the bell to its
/// corner.
pub fn relayout(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;
    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);
    let (position, content) = content_rect(&window)?;
    let (dm_position, dm_content) = dm_content_rect(&window)?;
    let (bell_position, bell_size) = bell_rect(&window)?;
    let (popup_position, popup_size) = popup_rect(&window)?;

    for webview in window.webviews() {
        match webview.label() {
            SHELL_WEBVIEW => {
                webview.set_position(LogicalPosition::new(0.0, 0.0))?;
                webview.set_size(LogicalSize::new(size.width, size.height))?;
            }
            OVERLAY_WEBVIEW => {
                webview.set_position(bell_position)?;
                webview.set_size(bell_size)?;
            }
            POPUP_WEBVIEW => {
                webview.set_position(popup_position)?;
                webview.set_size(popup_size)?;
            }
            // a conversation view is inset by Shiver's DM list, a server view is not
            label if label.starts_with("dm::") => {
                webview.set_position(dm_position)?;
                webview.set_size(dm_content)?;
            }
            _ => {
                webview.set_position(position)?;
                webview.set_size(content)?;
            }
        }
    }

    Ok(())
}

/// Creates the bell overlay, or moves it back on top if it is already there.
///
/// Child webviews stack in creation order and there is no API to raise one, so opening a server for
/// the first time would bury the bell under it. Rebuilding the overlay afterwards is what keeps it
/// visible; it is a tiny local page, so this is cheap.
pub fn ensure_overlay(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

    // the popup is torn down with the bell, so it can never end up buried under a server webview.
    // it goes through `set_popup_open` rather than closing the webview directly because the core
    // has to agree the feed is gone: leaving the flag set meant the next click on the bell computed
    // "close" for a popup that was not there, and the feed took two clicks to come back.
    set_popup_open(app, false)?;

    if let Some(existing) = app.get_webview(OVERLAY_WEBVIEW) {
        existing.close()?;
    }

    let (position, size) = bell_rect(&window)?;

    window.add_child(
        WebviewBuilder::new(
            OVERLAY_WEBVIEW,
            WebviewUrl::App("index.html?view=overlay".into()),
        )
        .transparent(true),
        position,
        size,
    )?;

    Ok(())
}

/// Opens the notification feed under the bell, or closes it.
///
/// The popup is built fresh each time rather than hidden and reshown: child webviews stack in
/// creation order with no way to raise one, so creating it last is what keeps it above every
/// server. The bell is untouched either way, which is the point of them being separate.
pub fn set_popup_open(app: &AppHandle, open: bool) -> Result<()> {
    app.state::<ActiveServer>().set_popup_open(open);

    // the bell is a separate webview and cannot see the feed, so it is told: otherwise a feed that
    // dismissed itself would leave the bell drawn as though it were still open
    let _ = app.emit_to(
        OVERLAY_WEBVIEW,
        POPUP_EVENT,
        serde_json::json!({ "open": open }),
    );

    if !open {
        return close_popup(app);
    }

    if app.get_webview(POPUP_WEBVIEW).is_some() {
        return Ok(());
    }

    let window = main_window(app)?;
    let (position, size) = popup_rect(&window)?;

    // read and released before add_child, which blocks on the event loop: no lock is held across it
    let background = {
        let store = app.state::<crate::store::Store>();
        let registry = store.registry();

        startup_color(&registry.settings)
    };

    window.add_child(
        // painted before the page loads, so opening the feed does not flash white first
        WebviewBuilder::new(
            POPUP_WEBVIEW,
            WebviewUrl::App("index.html?view=popup".into()),
        )
        .background_color(background),
        position,
        size,
    )?;

    if let Some(popup) = app.get_webview(POPUP_WEBVIEW) {
        popup.set_focus()?;
    }

    Ok(())
}

fn close_popup(app: &AppHandle) -> Result<()> {
    if let Some(popup) = app.get_webview(POPUP_WEBVIEW) {
        popup.close()?;
    }

    Ok(())
}

fn find_server_webview(app: &AppHandle, entry_id: &str) -> Option<Webview> {
    app.get_webview(&webview_label(entry_id))
}

/// Brings `entry`'s webview to the front, creating it on first use.
pub fn show_server(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    voice_locked: bool,
) -> Result<()> {
    let active = app.state::<ActiveServer>();
    let _guard = active
        .gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let window = main_window(app)?;

    // re-checked inside the gate: a call that queued behind a creation must see its result
    let created = find_server_webview(app, &entry.id).is_none();

    if created {
        create_server_webview(&window, entry, settings, token, muted, voice_locked).inspect_err(
            |error| {
                eprintln!("[shiver] could not open {}: {error}", entry.origin);
            },
        )?;
    }

    for webview in window.webviews() {
        if matches!(webview.label(), SHELL_WEBVIEW | OVERLAY_WEBVIEW)
            || webview.label() == webview_label(&entry.id)
        {
            continue;
        }

        webview.hide()?;
    }

    if let Some(webview) = find_server_webview(app, &entry.id) {
        let (position, size) = content_rect(&window)?;

        webview.set_position(position)?;
        webview.set_size(size)?;
        webview.show()?;
        webview.set_focus()?;
    }

    active.set(Some(entry.id.clone()));
    active.set_showing_server(true);

    // a page shown while the DM split is up has to be told, even though the layout did not change:
    // it may have been preloaded before the split and would otherwise still draw its own sidebar

    // a webview created just now sits above the bell, so the bell has to be rebuilt over it
    if created {
        ensure_overlay(app)?;
    }

    Ok(())
}

/// Opens a server's webview without showing it.
///
/// This is what makes every server live from launch: a hidden webview still runs the client, holds
/// its websocket and keeps feeding Shiver notifications and DMs, so the inbox is complete without the
/// user having visited each server first.
pub fn preload_server(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    voice_locked: bool,
) -> Result<()> {
    let active = app.state::<ActiveServer>();
    let _guard = active
        .gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if find_server_webview(app, &entry.id).is_some() {
        return Ok(());
    }

    let window = main_window(app)?;

    create_server_webview(&window, entry, settings, token, muted, voice_locked)?;

    // it must not cover whatever the user is already looking at
    if let Some(webview) = find_server_webview(app, &entry.id) {
        if active.get().as_deref() != Some(entry.id.as_str()) {
            webview.hide()?;
        }
    }

    ensure_overlay(app)?;

    Ok(())
}

/// Hides every server webview so the shell's own UI is visible across the whole window. Used for
/// Shiver panels (settings, the notification inbox) and when the rail is empty.
pub fn show_shell_only(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

    app.state::<ActiveServer>().set_showing_server(false);

    for webview in window.webviews() {
        // the bell belongs to Shiver, not to the server, so it stays up over Shiver's own panels
        if is_shiver_chrome(webview.label()) {
            continue;
        }

        webview.hide()?;
    }

    Ok(())
}

/// Tears a server's webview down. The webview owns the session, so closing it is also what ends
/// the signed-in session for that entry.
pub fn close_server(app: &AppHandle, entry_id: &str) -> Result<()> {
    if let Some(webview) = find_server_webview(app, entry_id) {
        webview.close()?;
    }

    let active = app.state::<ActiveServer>();

    if active.get().as_deref() == Some(entry_id) {
        active.set(None);
    }

    Ok(())
}

fn create_server_webview(
    window: &Window,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    voice_locked: bool,
) -> Result<()> {
    let (position, size) = content_rect(window)?;

    build_page_webview(
        window,
        entry,
        settings,
        token,
        muted,
        &webview_label(&entry.id),
        false,
        None,
        voice_locked,
        position,
        size,
    )
}

/// Builds one webview holding a server's own client, pinned to its origin.
///
/// `dm_role` marks the conversation view: that page hides its own sidebar and reports neither
/// notifications nor DMs, because the server view for the same entry is already doing both and two
/// pages reporting would double every notification and fight over the inbox.
#[allow(clippy::too_many_arguments)]
fn build_page_webview(
    window: &Window,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    label: &str,
    dm_role: bool,
    open_dm: Option<&str>,
    voice_locked: bool,
    position: LogicalPosition<f64>,
    size: LogicalSize<f64>,
) -> Result<()> {
    let url = Url::parse(&entry.origin)
        .map_err(|_| Error::InvalidOrigin(format!("'{}' is not a valid address", entry.origin)))?;

    let origin = entry.origin.clone();
    let handle = window.app_handle().clone();

    // whether a page on the pinned origin has actually finished loading. an off-origin navigation
    // before that is the server redirecting its own entry point, not the user following a link, and
    // must not be thrown at the browser: Sharkord in dev mode 302s '/' to its client's dev server,
    // which would otherwise pop a browser tab on every launch.
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_for_page = loaded.clone();
    let origin_for_nav = origin.clone();

    let builder = WebviewBuilder::new(label.to_string(), WebviewUrl::External(url))
        // Off for the same reason as the shell's, with the opposite motive. Tauri's native handler
        // takes a drag before the page can see it, so `dragover` and `drop` never fire — and
        // Sharkord uploads by listening for exactly those (`hooks/use-upload-files.ts`). Left on,
        // dragging a file onto a server does nothing at all. Shiver has no use for the drop itself:
        // the file belongs to the server's own uploader, and this gets out of its way.
        .disable_drag_drop_handler()
        .initialization_script(bridge_script(
            entry,
            settings,
            token,
            muted,
            dm_role,
            open_dm,
            voice_locked,
        ))
    .on_page_load(move |_webview, payload| {
        if matches!(payload.event(), PageLoadEvent::Finished) {
            loaded_for_page.store(true, Ordering::Relaxed);
        }
    })
    .on_navigation(move |target| {
        if is_same_origin(&origin_for_nav, target) {
            return true;
        }

        if !loaded.load(Ordering::Relaxed) {
            eprintln!(
                "[shiver] {origin_for_nav} redirected to {target} before it finished loading, so it was not opened"
            );

            return false;
        }

        // a link out of Sharkord, followed from a page that is already up. it opens in the user's
        // browser so a server can never navigate its own webview somewhere Shiver still treats as
        // that server.
        open_externally(&handle, target);

        false
    })
    .on_new_window({
        let handle = window.app_handle().clone();

        // Sharkord opens files and links with `target="_blank"`, which is a *new window* rather
        // than a navigation — so none of it reached the handler above, and wry's own default for an
        // unhandled request is `Deny`. Clicking an attachment or any link in a message did nothing
        // at all, silently, on both clients.
        //
        // Shiver has no second window to put it in, and it would not want one: a file or an outside
        // link belongs to the browser, which is where every other link out of Sharkord already
        // goes. So it is handed over and the request itself is refused.
        move |url, _features| {
            open_externally(&handle, &url);

            tauri::webview::NewWindowResponse::Deny
        }
    });

    window.add_child(builder, position, size)?;

    Ok(())
}

pub fn open_externally(app: &AppHandle, url: &Url) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }

    use tauri_plugin_opener::OpenerExt;

    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("[shiver] could not open {url} in the browser: {error}");
    }
}

/// The bridge runs in every server page. Its config is inlined ahead of it so the script itself
/// stays generic, and it carries only this entry's own data: nothing about any other server ever
/// reaches a server's page.
/// Shows the conversation view for one entry, beside Shiver's DM list, creating it on first use.
///
/// This is a *second* client for that server, dedicated to DMs. It costs an extra connection while
/// the inbox is open, and buys the thing the shared webview could not give: the server view keeps
/// whatever channel the user left it on.
pub fn show_dm_view(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    open_dm: &str,
) -> Result<bool> {
    let active = app.state::<ActiveServer>();
    let _guard = active
        .gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let window = main_window(app)?;
    let label = dm_webview_label(&entry.id);
    let created = app.get_webview(&label).is_none();

    if created {
        let (position, size) = dm_content_rect(&window)?;

        // the conversation travels in the page's own config, because nothing can be evaluated in a
        // webview that has not loaded yet
        build_page_webview(
            &window,
            entry,
            settings,
            token,
            &[],
            &label,
            true,
            Some(open_dm),
            // the conversation view hides Sharkord's own sidebar, so it has no voice controls to
            // lock and never reports a session of its own
            false,
            position,
            size,
        )?;
    }

    active.set_showing_server(false);

    // only ever one conversation view at a time, and no server view visible behind it
    for webview in window.webviews() {
        if is_shiver_chrome(webview.label()) || webview.label() == label {
            continue;
        }

        webview.hide()?;
    }

    if let Some(webview) = app.get_webview(&label) {
        let (position, size) = dm_content_rect(&window)?;

        webview.set_position(position)?;
        webview.set_size(size)?;
        webview.show()?;
        webview.set_focus()?;
    }

    if created {
        ensure_overlay(app)?;
    }

    Ok(created)
}

/// Opens a server's conversation view without showing it.
///
/// The same idea as `preload_server`, applied to the second client each entry keeps for DMs: the
/// page is up and signed in before the user asks for a conversation, so opening one is a matter of
/// showing a webview and naming the conversation rather than waiting on a fresh client to connect.
///
/// It carries no conversation of its own. `show_dm_view` handed one over in the page's config
/// precisely because the page did not exist yet; a preloaded page does, so it is simply told.
pub fn preload_dm_view(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
) -> Result<()> {
    let active = app.state::<ActiveServer>();
    let _guard = active
        .gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let label = dm_webview_label(&entry.id);

    if app.get_webview(&label).is_some() {
        return Ok(());
    }

    let window = main_window(app)?;
    let (position, size) = dm_content_rect(&window)?;

    build_page_webview(
        &window,
        entry,
        settings,
        token,
        &[],
        &label,
        true,
        None,
        // the conversation view hides Sharkord's own sidebar, so it has no voice controls to lock
        false,
        position,
        size,
    )?;

    if let Some(webview) = app.get_webview(&label) {
        webview.hide()?;
    }

    ensure_overlay(app)?;

    Ok(())
}

/// Closes one entry's conversation view, so it can be rebuilt on a new session.
pub fn close_dm_view(app: &AppHandle, entry_id: &str) {
    if let Some(webview) = app.get_webview(&dm_webview_label(entry_id)) {
        if let Err(error) = webview.close() {
            eprintln!("[shiver] could not close the conversation view for {entry_id}: {error}");
        }
    }
}

/// Hides every conversation view, for when the user leaves the inbox.
///
/// They are hidden rather than closed. Closing them is what the on-demand version did, and it threw
/// away a connected client every time the user stepped out of the inbox, so the next conversation
/// waited on a fresh sign-in. The cost of keeping them is honest and it is the point of preloading:
/// an entry in the rail is two running clients, not one.
pub fn hide_dm_views(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

    for webview in window.webviews() {
        if webview.label().starts_with("dm::") {
            webview.hide()?;
        }
    }

    Ok(())
}

/// Repaints every open server and conversation page in the user's colours, without reloading them.
pub fn push_theme(app: &AppHandle, settings: &Settings) {
    let Ok(window) = main_window(app) else {
        return;
    };

    let theme = theme_payload(settings);

    for webview in window.webviews() {
        if is_shiver_chrome(webview.label()) {
            continue;
        }

        let _ = webview.eval(format!(
            "window.__SHIVER_SET_THEME__ && window.__SHIVER_SET_THEME__({theme})"
        ));

        // Loudness moves with the same save, and waiting for a reload to hear it would make the
        // slider feel broken — it is the one setting whose whole point is to be adjusted by ear.
        let volume = settings.sound_volume.min(crate::model::MAX_SOUND_VOLUME);

        let _ = webview.eval(format!(
            "window.__SHIVER_SET_SOUND_VOLUME__ && window.__SHIVER_SET_SOUND_VOLUME__({volume})"
        ));

        let minimise = settings.minimise_attachments;

        let _ = webview.eval(format!(
            "window.__SHIVER_SET_ATTACHMENT_CARDS__ && window.__SHIVER_SET_ATTACHMENT_CARDS__({minimise})"
        ));
    }
}

/// The colour a Shiver webview paints before its own stylesheet has loaded.
///
/// Without it the webview starts white, which reads as a flash when the notification feed opens.
/// It follows the user's chosen background, so a themed Shiver does not flash the default grey
/// either.
fn startup_color(settings: &Settings) -> tauri::webview::Color {
    let hex = settings.theme_color.trim_start_matches('#');

    let channel = |index: usize| {
        hex.get(index..index + 2)
            .and_then(|part| u8::from_str_radix(part, 16).ok())
            .unwrap_or(0x17)
    };

    tauri::webview::Color(channel(0), channel(2), channel(4), 255)
}

/// `null` while the user is on Sharkord's own colours, so a stock Shiver restyles nothing.
fn theme_payload(settings: &Settings) -> serde_json::Value {
    if settings.uses_default_colors() {
        return serde_json::Value::Null;
    }

    json!({
        "themeColor": settings.theme_color,
        "accentColor": settings.accent_color,
        "textColor": settings.text_color,
    })
}

/// Tells one page whether a call is running on some other server.
///
/// A locked page refuses to join voice. It is told only that, never which server holds the call:
/// server A learning that the user is in a call is unavoidable if the join is to be refused there,
/// but learning anything identifying about server B is not, so nothing else crosses.
pub fn push_voice_lock(app: &AppHandle, entry_id: &str, locked: bool) {
    let Some(webview) = app.get_webview(&webview_label(entry_id)) else {
        return;
    };

    let _ = webview.eval(format!(
        "window.__SHIVER_SET_VOICE_LOCK__ && window.__SHIVER_SET_VOICE_LOCK__({locked})"
    ));
}

/// Works one of Sharkord's own voice controls in a page, on the user's behalf.
///
/// This is how Shiver's rail controls act: the client keeps `ownVoiceState` and its toggles in redux
/// and React context, so there is nothing to call, and the buttons are the public surface.
pub fn run_voice_action(app: &AppHandle, entry_id: &str, action: &str) {
    let Some(webview) = app.get_webview(&webview_label(entry_id)) else {
        return;
    };

    let payload = json!(action);

    let _ = webview.eval(format!(
        "window.__SHIVER_VOICE__ && window.__SHIVER_VOICE__({payload})"
    ));
}

/// Asks a page to mark every one of its text channels read.
pub fn mark_all_read(app: &AppHandle, entry_id: &str) {
    let Some(webview) = app.get_webview(&webview_label(entry_id)) else {
        return;
    };

    let _ = webview.eval("window.__SHIVER_MARK_ALL_READ__ && window.__SHIVER_MARK_ALL_READ__()");
}

/// Tells one page its mute list changed, so the channel list redraws without a reload.
pub fn push_muted(app: &AppHandle, entry_id: &str, muted: &[i64]) {
    let Some(webview) = app.get_webview(&webview_label(entry_id)) else {
        return;
    };

    let payload = json!(muted);

    let _ = webview.eval(format!(
        "window.__SHIVER_SET_MUTED__ && window.__SHIVER_SET_MUTED__({payload})"
    ));
}

fn bridge_script(
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    dm_role: bool,
    open_dm: Option<&str>,
    voice_locked: bool,
) -> String {
    // `theme` is null while the user is on the default colours, and the bridge then restyles
    // nothing at all: an untouched Shiver has to leave Sharkord looking like Sharkord
    let theme = theme_payload(settings);

    let config = json!({
        "entryId": entry.id,
        "origin": entry.origin,
        "theme": theme,
        "token": token,
        "muted": muted,
        "role": if dm_role { "dm" } else { "server" },
        "openDm": open_dm,
        // Clamped here rather than trusted: the page never sets it, but the settings file is a file,
        // and a gain of four hundred is a way to hurt somebody wearing headphones.
        "soundVolume": settings.sound_volume.min(crate::model::MAX_SOUND_VOLUME),
        "minimiseAttachments": settings.minimise_attachments,
        // handed over at creation rather than pushed afterwards, so a page opened while a call is
        // already up refuses a join from its very first frame instead of from the next drain
        "voiceLocked": voice_locked,
    });

    format!("window.__SHIVER__ = {config};\n{BRIDGE_SOURCE}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bell_opens_the_feed_when_it_is_closed() {
        let state = ActiveServer::default();

        assert!(state.should_open_popup());
    }

    #[test]
    fn the_bell_closes_the_feed_when_it_is_open() {
        let state = ActiveServer::default();

        state.set_popup_open(true);

        assert!(!state.should_open_popup());
    }

    /// The click that dismissed the feed must not then reopen it: focus moves to the bell first,
    /// so by the time the click lands the feed has already closed itself.
    #[test]
    fn the_click_that_dismissed_the_feed_does_not_reopen_it() {
        let state = ActiveServer::default();

        state.set_popup_open(true);

        // losing focus to the bell
        state.note_popup_dismissed();
        state.set_popup_open(false);

        assert!(!state.should_open_popup());
    }

    #[test]
    fn a_dismissal_stops_counting_once_the_guard_has_passed() {
        let state = ActiveServer::default();

        state.note_popup_dismissed();
        state.set_popup_open(false);

        std::thread::sleep(POPUP_REOPEN_GUARD + Duration::from_millis(50));

        assert!(state.should_open_popup());
    }
}
