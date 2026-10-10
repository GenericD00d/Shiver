//! Window and webview layout.
//!
//! One window holds the shell webview (Shiver's UI, full-window, drawing the rail on the left), a
//! webview per open server pinned to that server's origin and inset by the rail (and by the DM list
//! while it shows a conversation), the bell overlay and its popup.
//! Child webviews stack in creation order with no way to raise one, so the bell is rebuilt after
//! any server webview is created and the popup is built fresh each time it opens.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::json;
pub use shiver_core::limit::Openings;
use shiver_core::LockExt;
use tauri::{
    webview::WebviewBuilder, AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl,
    Window, WindowEvent,
};
use url::Url;

use crate::{
    error::{Core, Error, Result},
    model::{is_same_origin, ServerEntry, Settings},
    store::Store,
};

mod layout;
mod links;
mod page;
mod profiles;

use layout::{
    bell_rect, content_rect, dm_content_rect, logical_size, page_rect, place, popup_rect,
};
use page::{push_visibility, startup_color, theme_payload};
use profiles::{data_store, profile_name, profiles_root};

pub use links::ask_to_open;
pub use page::{
    mark_all_read, push_muted, push_notify_level, push_theme, push_voice_lock, run_voice_action,
};
pub use profiles::{discard_profiles, prune_profiles};

pub const MAIN_WINDOW: &str = "main";

pub const SHELL_WEBVIEW: &str = "shell";

pub const OVERLAY_WEBVIEW: &str = "overlay";

pub const POPUP_WEBVIEW: &str = "popup";

/// Tells the bell whether its popup is open.
const POPUP_EVENT: &str = "shiver://popup";

/// Width of the rail, and of Shiver's DM list (matching Sharkord's `w-72` sidebar).
const RAIL_WIDTH: f64 = 72.0;

const DM_LIST_WIDTH: f64 = 288.0;

const BELL_SIZE: (f64, f64) = (48.0, 48.0);

const POPUP_SIZE: (f64, f64) = (380.0, 540.0);

/// A click on the bell this soon after the popup dismissed itself (on losing focus to that same
/// click) is the click that closed it, not a request to reopen it.
const POPUP_REOPEN_GUARD: Duration = Duration::from_millis(400);

const BRIDGE_SOURCE: &str = include_str!("../../generated/bridge.js");

pub fn webview_label(entry_id: &str) -> String {
    format!("server::{entry_id}")
}

pub fn is_shiver_chrome(label: &str) -> bool {
    matches!(label, SHELL_WEBVIEW | OVERLAY_WEBVIEW | POPUP_WEBVIEW)
}

/// The entry a page webview belongs to, from its label.
fn entry_of(label: &str) -> Option<&str> {
    label.strip_prefix("server::")
}

#[derive(Default)]
struct Screen {
    /// the server selected in the rail
    current: Option<String>,
    /// whether that server's page is what is on screen (not a Shiver panel over it)
    showing_server: bool,
    /// the entry whose page is on screen showing a conversation, beside Shiver's DM list
    conversation: Option<String>,
    /// who that conversation is with, as Shiver asked for it
    conversation_with: Option<String>,
    popup_open: bool,
    popup_dismissed_at: Option<Instant>,
    /// servers with a page, most recently shown first, then those opened hidden (what `trim_pages`
    /// closes from the back of)
    recent: Vec<String>,
    /// the entry whose page is fullscreen
    fullscreen: Option<String>,
    /// the window is minimised or hidden to the tray, so nothing in it is being read
    window_hidden: bool,
}

impl Screen {
    /// Records a page for `entry_id`: one shown goes to the front, one opened hidden to the back
    /// (so it is trimmed first if the user never looks at it).
    fn hold(&mut self, entry_id: &str, shown: bool) {
        if shown {
            self.recent.retain(|held| held != entry_id);
            self.recent.insert(0, entry_id.to_string());
        } else if !self.recent.iter().any(|held| held == entry_id) {
            self.recent.push(entry_id.to_string());
        }
    }
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

    /// Whether this entry's page is what is on screen (its server, or a conversation in it).
    pub fn is_on_screen(&self, entry_id: &str) -> bool {
        let screen = self.screen.locked();

        (screen.showing_server && screen.current.as_deref() == Some(entry_id))
            || screen.conversation.as_deref() == Some(entry_id)
    }

    /// On screen and the window in sight: what the page shows counts as read.
    pub fn is_being_read(&self, entry_id: &str) -> bool {
        self.is_on_screen(entry_id) && !self.screen.locked().window_hidden
    }

    /// Records whether the window is out of sight; returns whether that changed.
    pub fn set_window_hidden(&self, hidden: bool) -> bool {
        let mut screen = self.screen.locked();
        let changed = screen.window_hidden != hidden;

        screen.window_hidden = hidden;

        changed
    }

    /// Who Shiver asked this entry's page to open a conversation with, while it shows it. A page's
    /// report that the conversation could not be opened names no one itself: this is the name told.
    pub fn conversation_with(&self, entry_id: &str) -> Option<String> {
        let screen = self.screen.locked();

        screen
            .conversation_with
            .clone()
            .filter(|_| screen.conversation.as_deref() == Some(entry_id))
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
        chrome_webview(SHELL_WEBVIEW, "index.html").disable_drag_drop_handler(),
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
                let hidden =
                    window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(true);

                set_window_hidden(&handle, hidden);
            }
        }

        if let WindowEvent::CloseRequested { api, .. } = event {
            if crate::tray::hide_instead_of_closing(&handle) {
                api.prevent_close();
            }
        }
    });

    Ok(window)
}

/// Whether Shiver's window is the one the user is using: shown, not minimised, and in front. Not
/// `is_focused` alone, which on Windows reads false while the keyboard is in one of its webviews.
pub fn is_in_front(app: &AppHandle) -> bool {
    let Ok(window) = main_window(app) else {
        return false;
    };

    if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(true) {
        return false;
    }

    #[cfg(windows)]
    {
        // SAFETY: takes no arguments and only reads which window is in front
        let front = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };

        window.hwnd().is_ok_and(|hwnd| hwnd.0 == front.0)
    }

    #[cfg(not(windows))]
    window.is_focused().unwrap_or(false)
}

/// Records whether the window is out of sight (minimised, or hidden to the tray) and tells the
/// pages when that changes.
pub fn set_window_hidden(app: &AppHandle, hidden: bool) {
    if app.state::<ActiveServer>().set_window_hidden(hidden) {
        push_visibility(app, hidden);
    }
}

/// Re-places every webview for the current window size. One failure does not stop the rest.
pub fn relayout(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;
    let full = (LogicalPosition::new(0.0, 0.0), logical_size(&window)?);
    let fullscreen = app
        .state::<ActiveServer>()
        .update(|screen| screen.fullscreen.clone());

    for webview in window.webviews() {
        let label = webview.label();
        let rect = match (label, entry_of(label)) {
            (SHELL_WEBVIEW, _) => full,
            (OVERLAY_WEBVIEW, _) => bell_rect(&window)?,
            (POPUP_WEBVIEW, _) => popup_rect(&window)?,
            (_, Some(entry_id)) if fullscreen.as_deref() == Some(entry_id) => full,
            (_, Some(entry_id)) => page_rect(app, &window, entry_id)?,
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
    let next = on.then(|| entry_id.to_string());

    if app
        .state::<ActiveServer>()
        .update(|screen| std::mem::replace(&mut screen.fullscreen, next.clone()))
        == next
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
                page_rect(app, &window, entry_id)?
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

/// Whether `url` is one of Shiver's bundled pages: exactly Tauri's own origin (`http://tauri.localhost`
/// on Windows, `tauri://localhost` elsewhere), with no port. Any other port on a `*.localhost` host is
/// whatever listens on the loopback, and a page there with a chrome label would get every command.
/// Debug builds also allow the dev server.
fn is_shiver_page(url: &Url) -> bool {
    let (scheme, host) = if cfg!(windows) {
        ("http", "tauri.localhost")
    } else {
        ("tauri", "localhost")
    };

    (url.scheme() == scheme && url.host_str() == Some(host) && url.port().is_none())
        || (cfg!(debug_assertions) && url.scheme() == "http" && url.host_str() == Some("localhost"))
}

/// One of Shiver's own webviews, which only ever shows Shiver's pages.
fn chrome_webview(label: &str, path: &str) -> WebviewBuilder<tauri::Wry> {
    WebviewBuilder::new(label, WebviewUrl::App(path.into()))
        .on_navigation(is_shiver_page)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
}

/// Rebuilds the bell on top of everything (and closes the popup, which is rebuilt on demand).
fn ensure_overlay(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

    set_popup_open(app, false)?;

    if let Some(existing) = app.get_webview(OVERLAY_WEBVIEW) {
        existing.close()?;
    }

    let (position, size) = bell_rect(&window)?;

    window.add_child(
        chrome_webview(OVERLAY_WEBVIEW, "index.html?view=overlay").transparent(true),
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
        chrome_webview(POPUP_WEBVIEW, "index.html?view=popup").background_color(background),
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

    end_conversation(app);

    if created {
        build_page_webview(
            &window,
            entry,
            settings,
            token,
            muted,
            (voice_locked, None),
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
        screen.popup_open = false;
        screen.fullscreen = None;
        screen.hold(&entry.id, true);
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
        (voice_locked, None),
        content_rect(&window)?,
    )?;

    if active.get().as_deref() != Some(entry.id.as_str()) {
        if let Some(webview) = app.get_webview(&webview_label(&entry.id)) {
            webview.hide()?;
        }
    }

    active.update(|screen| screen.hold(&entry.id, false));

    ensure_overlay(app)
}

/// Hides every server page so a Shiver panel can use the whole window.
pub fn show_shell_only(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

    end_conversation(app);
    app.state::<ActiveServer>()
        .update(|screen| screen.showing_server = false);

    hide_all_but(&window, is_shiver_chrome);

    Ok(())
}

/// Shows an entry's own page as the conversation with `name`, beside Shiver's DM list: the page
/// hides its channels and opens the DM, and gets its channels back when the conversation ends.
pub fn show_conversation(
    app: &AppHandle,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    voice_locked: bool,
    name: &str,
) -> Result<()> {
    let active = app.state::<ActiveServer>();
    let _gate = active.gate.locked();
    let window = main_window(app)?;
    let label = webview_label(&entry.id);

    if active.update(|screen| {
        screen
            .conversation
            .as_ref()
            .is_some_and(|id| id != &entry.id)
    }) {
        end_conversation(app);
    }

    let created = match app.get_webview(&label) {
        Some(webview) => {
            webview.eval(format!(
                "window.__SHIVER_CONVERSATION__ && window.__SHIVER_CONVERSATION__({})",
                json!(name)
            ))?;

            false
        }
        None => {
            build_page_webview(
                &window,
                entry,
                settings,
                token,
                muted,
                (voice_locked, Some(name)),
                dm_content_rect(&window)?,
            )?;

            true
        }
    };

    hide_all_but(&window, |other| is_shiver_chrome(other) || other == label);

    if let Some(webview) = app.get_webview(&label) {
        place(&webview, dm_content_rect(&window)?)?;
        webview.show()?;
        webview.set_focus()?;
    }

    active.update(|screen| {
        screen.showing_server = false;
        screen.conversation = Some(entry.id.clone());
        screen.conversation_with = Some(name.to_string());
        screen.fullscreen = None;
        screen.hold(&entry.id, true);
    });

    if created {
        ensure_overlay(app)?;
    }

    Ok(())
}

/// Ends the conversation on screen, if any: its page gets its channels back and is hidden.
pub fn end_conversation(app: &AppHandle) {
    let Some(entry_id) = app.state::<ActiveServer>().update(|screen| {
        screen.conversation_with = None;
        screen.conversation.take()
    }) else {
        return;
    };

    let Some(webview) = app.get_webview(&webview_label(&entry_id)) else {
        return;
    };

    let _ = webview.eval("window.__SHIVER_CONVERSATION__ && window.__SHIVER_CONVERSATION__(null)");
    let _ = webview.hide();

    if let Ok(rect) = main_window(app).and_then(|window| content_rect(&window)) {
        let _ = place(&webview, rect);
    }
}

/// Closes pages beyond `pages_kept`, oldest first — never the selected or on-screen one, never one
/// in a call — and lets the sockets take over for them.
pub fn trim_pages(app: &AppHandle) {
    let keep = app.state::<Store>().registry().settings.pages_kept();
    let (showing, excess) = app.state::<ActiveServer>().update(|screen| {
        (
            [screen.current.clone(), screen.conversation.clone()],
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
        if showing
            .iter()
            .flatten()
            .chain(&in_call)
            .any(|kept| *kept == entry_id)
        {
            continue;
        }

        if let Err(error) = close_server(app, &entry_id) {
            eprintln!("[shiver] could not close the page for {entry_id}: {error}");
        }
    }

    crate::watch::sync(app);
}

/// Closes a server's page. The call it may have held ends with it.
pub fn close_server(app: &AppHandle, entry_id: &str) -> Result<()> {
    if let Some(webview) = app.get_webview(&webview_label(entry_id)) {
        webview.close()?;
    }

    app.state::<ActiveServer>().update(|screen| {
        for held in [
            &mut screen.current,
            &mut screen.conversation,
            &mut screen.fullscreen,
        ] {
            if held.as_deref() == Some(entry_id) {
                *held = None;
            }
        }

        if screen.conversation.is_none() {
            screen.conversation_with = None;
        }

        screen.recent.retain(|held| held != entry_id);
    });

    let voice = app.state::<crate::voice::VoiceState>();
    let held_the_call = voice.holder().as_deref() == Some(entry_id);

    voice.forget_entry(entry_id);

    // the rail's call bar would otherwise stay up with buttons that reach nothing
    if held_the_call {
        let _ = app.emit_to(SHELL_WEBVIEW, crate::drain::VOICE_EVENT, ());
    }

    Ok(())
}

/// Builds one page webview pinned to the entry's origin, in the entry's own profile, voice-locked
/// or not, and opening a conversation when given one.
///
/// Navigation stays on the origin and new windows are refused: the bridge hands over what the user
/// opens away from the page, and the drain opens it in the browser.
fn build_page_webview(
    window: &Window,
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    opening: (bool, Option<&str>),
    (position, size): (LogicalPosition<f64>, LogicalSize<f64>),
) -> Result<()> {
    let url = Url::parse(&entry.origin)
        .map_err(|_| Core::InvalidOrigin(format!("'{}' is not a valid address", entry.origin)))?;
    let origin = entry.origin.clone();

    let builder = WebviewBuilder::new(webview_label(&entry.id), WebviewUrl::External(url))
        // Sharkord uploads by listening for dragover/drop, which the native handler would swallow
        .disable_drag_drop_handler()
        .initialization_script(bridge_script(entry, settings, token, muted, opening))
        .data_store_identifier(data_store(entry))
        .on_navigation(move |target| {
            let allowed = is_same_origin(&origin, target);

            if !allowed {
                let away = target.origin().ascii_serialization();

                eprintln!("[shiver] {origin} tried to leave for {away}; refused");
            }

            allowed
        })
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
    let mut builder = crate::permissions::gate_downloads(builder, entry);

    if let Some(root) = profiles_root(window.app_handle()) {
        builder = builder.data_directory(root.join(profile_name(entry)));
    } else {
        eprintln!(
            "[shiver] no local data directory; {} shares the default browser profile",
            entry.id
        );
    }

    let webview = window.add_child(builder, position, size)?;

    crate::permissions::gate(window.app_handle(), &webview, entry);

    Ok(())
}

/// The bridge, preceded by this entry's own config. Nothing about any other server goes in. Only
/// the top frame on the entry's origin runs it: WebView2 adds the script to every frame, and the
/// config holds the session.
fn bridge_script(
    entry: &ServerEntry,
    settings: &Settings,
    token: Option<&str>,
    muted: &[i64],
    (voice_locked, conversation): (bool, Option<&str>),
) -> String {
    let config = json!({
        "entryId": entry.id,
        "origin": entry.origin,
        "theme": theme_payload(settings),
        "token": token,
        "muted": muted,
        "conversation": conversation,
        "soundVolume": settings.sound_volume.min(crate::model::MAX_SOUND_VOLUME),
        "minimiseAttachments": settings.minimise_attachments,
        "voiceLocked": voice_locked,
        "notify": entry.notify,
    });

    format!(
        "if (window === window.top && location.origin === {}) {{\nwindow.__SHIVER__ = {config};\n{BRIDGE_SOURCE}\n}}",
        json!(entry.origin)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiver_core::model::NotifyLevel;

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

    fn entry(profile: Option<&str>) -> ServerEntry {
        ServerEntry {
            id: "0b6d7e4e-1a4f-4bb0-9d3c-2f3b1a9e8c11".into(),
            origin: "https://chat.example.com".into(),
            name: "Chat".into(),
            icon_url: None,
            identity: None,
            account_label: None,
            folder_id: None,
            position: 0,
            accept_any_size: false,
            profile: profile.map(str::to_string),
            media_allowed: false,
            notify: NotifyLevel::default(),
        }
    }

    #[test]
    fn hidden_pages_queue_behind_the_ones_shown() {
        let mut screen = Screen::default();

        screen.hold("a", true);
        screen.hold("b", false);
        screen.hold("c", true);
        screen.hold("a", false);
        assert_eq!(screen.recent, ["c", "a", "b"]);

        screen.hold("b", true);
        assert_eq!(screen.recent, ["b", "c", "a"]);
    }

    #[test]
    fn the_bridge_runs_only_in_the_entrys_top_frame() {
        let script = bridge_script(
            &entry(None),
            &Settings::default(),
            Some("secret"),
            &[],
            (false, None),
        );

        assert!(script.starts_with(
            "if (window === window.top && location.origin === \"https://chat.example.com\") {"
        ));
        assert!(script.trim_end().ends_with('}'));
    }

    #[test]
    fn shivers_own_webviews_stay_on_its_exact_origin() {
        let url = |value: &str| Url::parse(value).expect("a url");
        let own = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };

        assert!(is_shiver_page(&url(&format!(
            "{own}/index.html?view=popup"
        ))));

        for away in [
            "http://tauri.localhost:8080/",
            "https://tauri.localhost/",
            "tauri://elsewhere/",
            "https://evil.example/",
        ] {
            assert!(!is_shiver_page(&url(away)), "{away}");
        }
    }

    #[test]
    fn labels_map_back_to_their_entry() {
        assert_eq!(entry_of(&webview_label("x")), Some("x"));
        assert_eq!(entry_of(SHELL_WEBVIEW), None);
        assert!(is_shiver_chrome(POPUP_WEBVIEW) && !is_shiver_chrome(&webview_label("shell")));
    }
}
