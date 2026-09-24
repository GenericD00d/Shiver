//! Window and webview layout.
//!
//! One window holds the shell webview (Shiver's UI, full-window, drawing the rail on the left), a
//! webview per open server pinned to that server's origin and inset by the rail, an optional
//! conversation view per server (inset further by the DM list), the bell overlay and its popup.
//! Child webviews stack in creation order with no way to raise one, so the bell is rebuilt after
//! any server webview is created and the popup is built fresh each time it opens.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde_json::json;
pub use shiver_core::limit::Openings;
use shiver_core::LockExt;
use tauri::{
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl, Window,
    WindowEvent,
};
use url::Url;

use crate::{
    error::{Error, Result},
    model::{is_same_origin, ServerEntry, Settings},
    store::Store,
};

pub const MAIN_WINDOW: &str = "main";
pub const SHELL_WEBVIEW: &str = "shell";
pub const OVERLAY_WEBVIEW: &str = "overlay";
pub const POPUP_WEBVIEW: &str = "popup";

/// Tells the bell whether its popup is open.
pub const POPUP_EVENT: &str = "shiver://popup";

/// Width of the rail, and of Shiver's DM list (matching Sharkord's `w-72` sidebar).
pub const RAIL_WIDTH: f64 = 72.0;
pub const DM_LIST_WIDTH: f64 = 288.0;
const BELL_SIZE: (f64, f64) = (48.0, 48.0);
const POPUP_SIZE: (f64, f64) = (380.0, 540.0);

/// A click on the bell this soon after the popup dismissed itself (on losing focus to that same
/// click) is the click that closed it, not a request to reopen it.
const POPUP_REOPEN_GUARD: Duration = Duration::from_millis(400);

const BRIDGE_SOURCE: &str = include_str!("../generated/bridge.js");

pub fn webview_label(entry_id: &str) -> String {
    format!("server::{entry_id}")
}

/// The conversation view: a second client for the same server, so the server view keeps its channel.
pub fn dm_webview_label(entry_id: &str) -> String {
    format!("dm::{entry_id}")
}

fn is_shiver_chrome(label: &str) -> bool {
    matches!(label, SHELL_WEBVIEW | OVERLAY_WEBVIEW | POPUP_WEBVIEW)
}

/// The entry a page webview belongs to, from its label.
fn entry_of(label: &str) -> Option<&str> {
    label
        .strip_prefix("server::")
        .or_else(|| label.strip_prefix("dm::"))
}

#[derive(Default)]
struct Screen {
    /// the server selected in the rail
    current: Option<String>,
    /// whether that server's page is what is on screen (not a Shiver panel over it)
    showing_server: bool,
    /// the entry whose conversation view is on screen
    dm_on_screen: Option<String>,
    popup_open: bool,
    popup_dismissed_at: Option<Instant>,
    /// servers with a page, most recently shown first (what `trim_pages` closes from the back of)
    recent: Vec<String>,
    /// whether the page on screen is fullscreen
    fullscreen: bool,
}

/// What is on screen. `gate` serialises webview creation, so overlapping calls cannot race to
/// create the same label.
#[derive(Default)]
pub struct ActiveServer {
    screen: Mutex<Screen>,
    gate: Mutex<()>,
}

impl ActiveServer {
    pub fn get(&self) -> Option<String> {
        self.screen.locked().current.clone()
    }

    pub fn showing_server(&self) -> bool {
        self.screen.locked().showing_server
    }

    pub fn dm_on_screen(&self) -> Option<String> {
        self.screen.locked().dm_on_screen.clone()
    }

    pub fn note_popup_dismissed(&self) {
        self.screen.locked().popup_dismissed_at = Some(Instant::now());
    }

    /// Open unless it is open, or was dismissed by the very click being handled.
    pub fn should_open_popup(&self) -> bool {
        let screen = self.screen.locked();

        !screen.popup_open
            && !screen
                .popup_dismissed_at
                .is_some_and(|at| at.elapsed() < POPUP_REOPEN_GUARD)
    }

    fn update<T>(&self, change: impl FnOnce(&mut Screen) -> T) -> T {
        change(&mut self.screen.locked())
    }
}

pub fn main_window<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<Window<R>> {
    app.get_window(MAIN_WINDOW)
        .ok_or_else(|| Error::Webview("The Shiver window is not open".into()))
}

fn logical_size(window: &Window) -> Result<LogicalSize<f64>> {
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

fn content_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    rect_from(window, RAIL_WIDTH)
}

fn dm_content_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    rect_from(window, RAIL_WIDTH + DM_LIST_WIDTH)
}

/// The bell, fixed to the top-right corner inside the space the bridge reserves in Sharkord's bar.
fn bell_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let size = logical_size(window)?;
    let (width, height) = BELL_SIZE;

    Ok((
        LogicalPosition::new((size.width - width).max(RAIL_WIDTH), 0.0),
        LogicalSize::new(width, height),
    ))
}

/// The popup, under the bell, clamped to the window.
fn popup_rect(window: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let size = logical_size(window)?;
    let width = POPUP_SIZE.0.min((size.width - RAIL_WIDTH).max(0.0));
    let height = POPUP_SIZE.1.min((size.height - BELL_SIZE.1).max(0.0));

    Ok((
        LogicalPosition::new((size.width - width).max(RAIL_WIDTH), BELL_SIZE.1),
        LogicalSize::new(width, height),
    ))
}

fn place(
    webview: &Webview,
    (position, size): (LogicalPosition<f64>, LogicalSize<f64>),
) -> Result<()> {
    webview.set_position(position)?;
    webview.set_size(size)?;

    Ok(())
}

/// Creates the window, the shell and the bell, and keeps the layout in step with resizes.
pub fn create_main_window(app: &AppHandle) -> Result<Window> {
    let window = tauri::window::WindowBuilder::new(app, MAIN_WINDOW)
        .title("Shiver")
        .inner_size(1280.0, 800.0)
        .min_inner_size(720.0, 480.0)
        // everything inside is dark, so the frame is too
        .theme(Some(tauri::Theme::Dark))
        .build()?;

    let size = logical_size(&window)?;

    // native drag-and-drop would swallow the html5 drags the rail is reordered with
    window.add_child(
        WebviewBuilder::new(SHELL_WEBVIEW, WebviewUrl::App("index.html".into()))
            .disable_drag_drop_handler(),
        LogicalPosition::new(0.0, 0.0),
        size,
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

            // minimising arrives as a resize on Windows; pages cannot see it themselves
            if let Ok(window) = main_window(&handle) {
                push_visibility(&handle, window.is_minimized().unwrap_or(false));
            }
        }
    });

    Ok(window)
}

/// Re-places every webview for the current window size. One failure does not stop the rest.
pub fn relayout(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;
    let full = (LogicalPosition::new(0.0, 0.0), logical_size(&window)?);
    let fullscreen = app
        .state::<ActiveServer>()
        .update(|screen| screen.fullscreen);
    let current = app
        .state::<ActiveServer>()
        .get()
        .map(|id| webview_label(&id));

    for webview in window.webviews() {
        let label = webview.label();
        let rect = match label {
            SHELL_WEBVIEW => full,
            OVERLAY_WEBVIEW => bell_rect(&window)?,
            POPUP_WEBVIEW => popup_rect(&window)?,
            _ if label.starts_with("dm::") => dm_content_rect(&window)?,
            _ if fullscreen && current.as_deref() == Some(label) => full,
            _ => content_rect(&window)?,
        };

        if let Err(error) = place(&webview, rect) {
            eprintln!("[shiver] could not lay out {label}: {error}");
        }
    }

    Ok(())
}

/// Takes Shiver's chrome away while the page on screen is fullscreen: the page is widened over the
/// rail and the bell (and popup) are hidden. Lags by up to one drain, since pages cannot call in.
///
/// Only real fullscreen counts (the bridge reads the browser's own getter), and switching away
/// restores the chrome (`show_server` resets it).
pub fn set_page_fullscreen(app: &AppHandle, entry_id: &str, on: bool) -> Result<()> {
    if app
        .state::<ActiveServer>()
        .update(|screen| std::mem::replace(&mut screen.fullscreen, on))
        == on
    {
        return Ok(());
    }

    let window = main_window(app)?;

    if let Some(webview) = app.get_webview(&webview_label(entry_id)) {
        place(
            &webview,
            if on {
                (LogicalPosition::new(0.0, 0.0), logical_size(&window)?)
            } else {
                content_rect(&window)?
            },
        )?;
    }

    for other in window.webviews() {
        match other.label() {
            OVERLAY_WEBVIEW | POPUP_WEBVIEW if on => other.hide()?,
            // the popup is only reopened when asked for
            OVERLAY_WEBVIEW => other.show()?,
            _ => {}
        }
    }

    Ok(())
}

/// Rebuilds the bell on top of everything (and closes the popup, which is rebuilt on demand).
pub fn ensure_overlay(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

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

/// Opens or closes the notification popup under the bell, and tells the bell.
pub fn set_popup_open(app: &AppHandle, open: bool) -> Result<()> {
    app.state::<ActiveServer>()
        .update(|screen| screen.popup_open = open);

    let _ = app.emit_to(OVERLAY_WEBVIEW, POPUP_EVENT, json!({ "open": open }));

    if !open {
        if let Some(popup) = app.get_webview(POPUP_WEBVIEW) {
            popup.close()?;
        }

        return Ok(());
    }

    if app.get_webview(POPUP_WEBVIEW).is_some() {
        return Ok(());
    }

    let window = main_window(app)?;
    let (position, size) = popup_rect(&window)?;
    let background = startup_color(&app.state::<Store>().registry().settings);

    // painted before the page loads, so opening does not flash white
    window.add_child(
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

/// Hides every webview the predicate does not keep, carrying on past failures.
fn hide_all_but(window: &Window, keep: impl Fn(&str) -> bool) {
    for webview in window.webviews() {
        if !keep(webview.label()) {
            if let Err(error) = webview.hide() {
                eprintln!("[shiver] could not hide {}: {error}", webview.label());
            }
        }
    }
}

/// Brings a server's page to the front, creating it on first use.
pub fn show_server(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    voice_locked: bool,
) -> Result<()> {
    let active = app.state::<ActiveServer>();
    let _gate = active.gate.locked();
    let window = main_window(app)?;
    let label = webview_label(&entry.id);
    let created = app.get_webview(&label).is_none();

    if created {
        build_page_webview(
            &window,
            entry,
            settings,
            token,
            muted,
            PageRole::Server { voice_locked },
            content_rect(&window)?,
        )
        .inspect_err(|error| eprintln!("[shiver] could not open {}: {error}", entry.origin))?;
    }

    hide_all_but(&window, |other| is_shiver_chrome(other) || other == label);

    if let Some(webview) = app.get_webview(&label) {
        place(&webview, content_rect(&window)?)?;
        webview.show()?;
        webview.set_focus()?;
    }

    active.update(|screen| {
        screen.current = Some(entry.id.clone());
        screen.showing_server = true;
        screen.dm_on_screen = None;
        screen.popup_open = false;
        screen.fullscreen = false;
        screen.recent.retain(|held| held != &entry.id);
        screen.recent.insert(0, entry.id.clone());
    });

    // the bell may have been hidden for a fullscreen page, and a new page sits above it
    if created {
        ensure_overlay(app)?;
    } else if let Some(bell) = app.get_webview(OVERLAY_WEBVIEW) {
        bell.show()?;
    }

    Ok(())
}

/// Opens a server's page without showing it.
pub fn preload_server(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    voice_locked: bool,
) -> Result<()> {
    let active = app.state::<ActiveServer>();
    let _gate = active.gate.locked();

    if app.get_webview(&webview_label(&entry.id)).is_some() {
        return Ok(());
    }

    let window = main_window(app)?;

    build_page_webview(
        &window,
        entry,
        settings,
        token,
        muted,
        PageRole::Server { voice_locked },
        content_rect(&window)?,
    )?;

    if active.get().as_deref() != Some(entry.id.as_str()) {
        if let Some(webview) = app.get_webview(&webview_label(&entry.id)) {
            webview.hide()?;
        }
    }

    ensure_overlay(app)
}

/// Hides every server page so a Shiver panel can use the whole window.
pub fn show_shell_only(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

    app.state::<ActiveServer>().update(|screen| {
        screen.showing_server = false;
        screen.dm_on_screen = None;
    });

    hide_all_but(&window, is_shiver_chrome);

    Ok(())
}

/// Closes pages beyond `pages_kept`, oldest first — never the one on screen, never one in a call —
/// and lets the sockets take over for them.
pub fn trim_pages(app: &AppHandle) {
    let keep = app.state::<Store>().registry().settings.pages_kept();
    let (showing, excess) = app.state::<ActiveServer>().update(|screen| {
        (
            screen.current.clone(),
            screen
                .recent
                .iter()
                .skip(keep)
                .rev()
                .cloned()
                .collect::<Vec<_>>(),
        )
    });
    let in_call = app.state::<crate::voice::VoiceState>().holder();

    for entry_id in excess {
        if Some(&entry_id) == showing.as_ref() || Some(&entry_id) == in_call.as_ref() {
            continue;
        }

        if let Err(error) = close_server(app, &entry_id) {
            eprintln!("[shiver] could not close the page for {entry_id}: {error}");
        }

        close_dm_view(app, &entry_id);
    }

    crate::watch::sync(app);
}

/// Closes a server's page. The call it may have held ends with it.
pub fn close_server(app: &AppHandle, entry_id: &str) -> Result<()> {
    if let Some(webview) = app.get_webview(&webview_label(entry_id)) {
        webview.close()?;
    }

    app.state::<ActiveServer>().update(|screen| {
        if screen.current.as_deref() == Some(entry_id) {
            screen.current = None;
        }

        screen.recent.retain(|held| held != entry_id);
    });
    app.state::<crate::voice::VoiceState>()
        .forget_entry(entry_id);

    Ok(())
}

/* ── browser profiles ── */

fn profiles_root(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_local_data_dir()
        .ok()
        .map(|dir| dir.join("webviews"))
}

/// The directory name of an entry's current browser profile. Each entry has its own profile (two
/// accounts on one server must not share `localStorage`), and logging out moves to a new one.
fn profile_name(entry: &ServerEntry) -> String {
    match &entry.profile {
        Some(generation) => format!("{}-{generation}", entry.id),
        None => entry.id.clone(),
    }
}

/// Deletes profile directories in the background, retrying while WebView2 releases its files.
/// Anything still left is removed by `prune_profiles` on the next launch.
fn delete_dirs_later(dirs: Vec<PathBuf>) {
    if dirs.is_empty() {
        return;
    }

    std::thread::spawn(move || {
        for _ in 0..20 {
            let remaining: Vec<&PathBuf> = dirs
                .iter()
                .filter(|dir| match std::fs::remove_dir_all(dir) {
                    Ok(()) => false,
                    Err(error) => error.kind() != std::io::ErrorKind::NotFound,
                })
                .collect();

            if remaining.is_empty() {
                return;
            }

            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Every profile directory belonging to an entry (current and older generations).
fn profile_dirs_of(app: &AppHandle, entry_id: &str, keep: Option<&str>) -> Vec<PathBuf> {
    let Some(root) = profiles_root(app) else {
        return Vec::new();
    };

    std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(|dir| dir.ok())
        .map(|dir| dir.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            (name == entry_id || name.starts_with(&format!("{entry_id}-")))
                && Some(name.as_str()) != keep
        })
        .map(|name| root.join(name))
        .collect()
}

/// Discards all of an entry's browser storage (cookies, localStorage, cache). Its pages must be
/// closed first.
pub fn discard_profiles(app: &AppHandle, entry_id: &str, keep: Option<&ServerEntry>) {
    let keep = keep.map(profile_name);

    delete_dirs_later(profile_dirs_of(app, entry_id, keep.as_deref()));
}

/// At launch, deletes profiles that belong to no current entry (removed servers, old generations,
/// deletions that did not finish last time).
pub fn prune_profiles(app: &AppHandle) {
    let Some(root) = profiles_root(app) else {
        return;
    };

    let current: std::collections::HashSet<String> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(profile_name)
        .collect();

    let stale = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(|dir| dir.ok())
        .filter(|dir| !current.contains(dir.file_name().to_string_lossy().as_ref()))
        .map(|dir| dir.path())
        .collect();

    delete_dirs_later(stale);
}

#[derive(Clone, Copy)]
enum PageRole<'a> {
    /// the page the user browses channels in
    Server { voice_locked: bool },
    /// the conversation view: hides its sidebar, reports only which DM is open
    Dm { open: Option<&'a str> },
}

/// Builds one page webview pinned to the entry's origin, in the entry's own profile.
///
/// Navigation stays on the origin; a link out of it (after the first load) and every new-window
/// request go to the browser, subject to the entry's opening allowance. An off-origin redirect
/// before the first load is refused rather than opened.
fn build_page_webview(
    window: &Window,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    role: PageRole<'_>,
    (position, size): (LogicalPosition<f64>, LogicalSize<f64>),
) -> Result<()> {
    let url = Url::parse(&entry.origin)
        .map_err(|_| Error::InvalidOrigin(format!("'{}' is not a valid address", entry.origin)))?;
    let label = match role {
        PageRole::Server { .. } => webview_label(&entry.id),
        PageRole::Dm { .. } => dm_webview_label(&entry.id),
    };

    let app = window.app_handle().clone();
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_for_page = loaded.clone();
    let origin = entry.origin.clone();
    let entry_id = entry.id.clone();
    let (nav_app, nav_id) = (app.clone(), entry_id.clone());

    let mut builder = WebviewBuilder::new(label, WebviewUrl::External(url))
        // Sharkord uploads by listening for dragover/drop, which the native handler would swallow
        .disable_drag_drop_handler()
        .initialization_script(bridge_script(entry, settings, token, muted, role))
        .on_page_load(move |_webview, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                loaded_for_page.store(true, Ordering::Relaxed);
            }
        })
        .on_navigation(move |target| {
            if is_same_origin(&origin, target) {
                return true;
            }

            if loaded.load(Ordering::Relaxed) {
                open_for_page(&nav_app, &nav_id, target);
            } else {
                eprintln!("[shiver] {origin} redirected to {target} before it finished loading; not opened");
            }

            false
        })
        .on_new_window(move |url, _features| {
            open_for_page(&app, &entry_id, &url);

            tauri::webview::NewWindowResponse::Deny
        });

    if let Some(root) = profiles_root(window.app_handle()) {
        builder = builder.data_directory(root.join(profile_name(entry)));
    } else {
        eprintln!(
            "[shiver] no local data directory; {} shares the default browser profile",
            entry.id
        );
    }

    window.add_child(builder, position, size)?;

    crate::permissions::apply_pending_reset(window.app_handle(), &entry.id);

    Ok(())
}

/// Opens a link from a page in the user's browser, within the entry's allowance.
pub fn open_for_page(app: &AppHandle, entry_id: &str, url: &Url) {
    if app.state::<Openings>().take(entry_id, 1) == 1 {
        open_in_browser(app, url);
    } else {
        eprintln!("[shiver] {entry_id} is opening links too quickly; {url} was not opened");
    }
}

/// Opens an http(s) link in the user's browser.
pub fn open_in_browser(app: &AppHandle, url: &Url) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }

    use tauri_plugin_opener::OpenerExt;

    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("[shiver] could not open {url} in the browser: {error}");
    }
}

/// Shows an entry's conversation view beside Shiver's DM list, creating it if needed. Returns
/// whether it was created (a new page opens `open_dm` itself; an existing one must be told).
pub fn show_dm_view(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    open_dm: &str,
) -> Result<bool> {
    let active = app.state::<ActiveServer>();
    let _gate = active.gate.locked();
    let window = main_window(app)?;
    let label = dm_webview_label(&entry.id);
    let created = app.get_webview(&label).is_none();

    if created {
        build_page_webview(
            &window,
            entry,
            settings,
            token,
            &[],
            PageRole::Dm {
                open: Some(open_dm),
            },
            dm_content_rect(&window)?,
        )?;
    }

    active.update(|screen| screen.showing_server = false);
    hide_all_but(&window, |other| is_shiver_chrome(other) || other == label);

    if let Some(webview) = app.get_webview(&label) {
        place(&webview, dm_content_rect(&window)?)?;
        webview.show()?;
        webview.set_focus()?;
        active.update(|screen| screen.dm_on_screen = Some(entry.id.clone()));
    }

    if created {
        ensure_overlay(app)?;
    }

    Ok(created)
}

/// Opens an entry's conversation view hidden, so opening a DM later is instant.
pub fn preload_dm_view(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
) -> Result<()> {
    let active = app.state::<ActiveServer>();
    let _gate = active.gate.locked();
    let label = dm_webview_label(&entry.id);

    if app.get_webview(&label).is_some() {
        return Ok(());
    }

    let window = main_window(app)?;

    build_page_webview(
        &window,
        entry,
        settings,
        token,
        &[],
        PageRole::Dm { open: None },
        dm_content_rect(&window)?,
    )?;

    if let Some(webview) = app.get_webview(&label) {
        webview.hide()?;
    }

    ensure_overlay(app)
}

pub fn close_dm_view(app: &AppHandle, entry_id: &str) {
    if let Some(webview) = app.get_webview(&dm_webview_label(entry_id)) {
        if let Err(error) = webview.close() {
            eprintln!("[shiver] could not close the conversation view for {entry_id}: {error}");
        }
    }

    app.state::<ActiveServer>().update(|screen| {
        if screen.dm_on_screen.as_deref() == Some(entry_id) {
            screen.dm_on_screen = None;
        }
    });
}

/// Hides (does not close) every conversation view, for leaving the inbox.
pub fn hide_dm_views(app: &AppHandle) -> Result<()> {
    hide_all_but(&main_window(app)?, |label| !label.starts_with("dm::"));
    app.state::<ActiveServer>()
        .update(|screen| screen.dm_on_screen = None);

    Ok(())
}

/// Evaluates `script` in every server and conversation page.
fn eval_in_pages(app: &AppHandle, script: &str) {
    let Ok(window) = main_window(app) else {
        return;
    };

    for webview in window.webviews() {
        if entry_of(webview.label()).is_some() {
            let _ = webview.eval(script);
        }
    }
}

/// Tells pages whether the window is minimised; WebView2 does not change `document.hidden` for it,
/// and Sharkord only notifies for the selected channel when the page is hidden.
pub fn push_visibility(app: &AppHandle, hidden: bool) {
    eval_in_pages(
        app,
        &format!("window.__SHIVER_SET_HIDDEN__ && window.__SHIVER_SET_HIDDEN__({hidden})"),
    );
}

/// Repaints every page in the user's colours and applies the sound and attachment settings, in place.
pub fn push_theme(app: &AppHandle, settings: &Settings) {
    let theme = theme_payload(settings);
    let volume = settings.sound_volume.min(crate::model::MAX_SOUND_VOLUME);
    let minimise = settings.minimise_attachments;

    eval_in_pages(
        app,
        &format!(
            "window.__SHIVER_SET_THEME__ && window.__SHIVER_SET_THEME__({theme});\
             window.__SHIVER_SET_SOUND_VOLUME__ && window.__SHIVER_SET_SOUND_VOLUME__({volume});\
             window.__SHIVER_SET_ATTACHMENT_CARDS__ && window.__SHIVER_SET_ATTACHMENT_CARDS__({minimise});"
        ),
    );
}

/// The colour a Shiver webview paints before its stylesheet loads, from the user's background.
fn startup_color(settings: &Settings) -> tauri::webview::Color {
    let [r, g, b] = shiver_core::model::rgb(&settings.theme_color).unwrap_or([0x17; 3]);

    tauri::webview::Color(r, g, b, 255)
}

fn theme_payload(settings: &Settings) -> serde_json::Value {
    shiver_core::model::theme_payload(
        &settings.theme_color,
        &settings.accent_color,
        settings.text_color.as_deref(),
    )
}

fn eval_in(app: &AppHandle, label: &str, script: &str) {
    if let Some(webview) = app.get_webview(label) {
        let _ = webview.eval(script);
    }
}

/// Tells one page whether a call is running elsewhere (never where).
pub fn push_voice_lock(app: &AppHandle, entry_id: &str, locked: bool) {
    eval_in(
        app,
        &webview_label(entry_id),
        &format!("window.__SHIVER_SET_VOICE_LOCK__ && window.__SHIVER_SET_VOICE_LOCK__({locked})"),
    );
}

/// Clicks one of Sharkord's own voice controls in a page.
pub fn run_voice_action(app: &AppHandle, entry_id: &str, action: &str) {
    eval_in(
        app,
        &webview_label(entry_id),
        &format!(
            "window.__SHIVER_VOICE__ && window.__SHIVER_VOICE__({})",
            json!(action)
        ),
    );
}

pub fn mark_all_read(app: &AppHandle, entry_id: &str) {
    eval_in(
        app,
        &webview_label(entry_id),
        "window.__SHIVER_MARK_ALL_READ__ && window.__SHIVER_MARK_ALL_READ__()",
    );
}

/// Tells one page its mute list changed.
pub fn push_muted(app: &AppHandle, entry_id: &str, muted: &[i64]) {
    eval_in(
        app,
        &webview_label(entry_id),
        &format!(
            "window.__SHIVER_SET_MUTED__ && window.__SHIVER_SET_MUTED__({})",
            json!(muted)
        ),
    );
}

/// The bridge, preceded by this entry's own config. Nothing about any other server goes in.
fn bridge_script(
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    role: PageRole<'_>,
) -> String {
    let (role_name, open_dm, voice_locked) = match role {
        PageRole::Server { voice_locked } => ("server", None, voice_locked),
        PageRole::Dm { open } => ("dm", open, false),
    };

    let config = json!({
        "entryId": entry.id,
        "origin": entry.origin,
        "theme": theme_payload(settings),
        "token": token,
        "muted": muted,
        "role": role_name,
        "openDm": open_dm,
        "soundVolume": settings.sound_volume.min(crate::model::MAX_SOUND_VOLUME),
        "minimiseAttachments": settings.minimise_attachments,
        "voiceLocked": voice_locked,
    });

    format!("window.__SHIVER__ = {config};\n{BRIDGE_SOURCE}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bell_toggles_but_ignores_the_click_that_dismissed_it() {
        let state = ActiveServer::default();

        assert!(state.should_open_popup());

        state.update(|screen| screen.popup_open = true);
        assert!(!state.should_open_popup());

        state.note_popup_dismissed();
        state.update(|screen| screen.popup_open = false);
        assert!(!state.should_open_popup());

        std::thread::sleep(POPUP_REOPEN_GUARD + Duration::from_millis(50));
        assert!(state.should_open_popup());
    }

    #[test]
    fn labels_map_back_to_their_entry() {
        assert_eq!(entry_of(&webview_label("x")), Some("x"));
        assert_eq!(entry_of(&dm_webview_label("x")), Some("x"));
        assert_eq!(entry_of(SHELL_WEBVIEW), None);
    }
}
